//! The editor widget: renders one `View` and turns input into `Action`s.

use egui::{
    Align2, CursorIcon, Event, EventFilter, FontId, Id, ImeEvent, Pos2, Rect, Response, Sense,
    Stroke, Ui, Vec2, WidgetInfo, WidgetType,
};
use ochrepad_core::{
    Action, ByteOffset, Document, Editor, Motion, Selection, SelectionSet, ViewId,
};
use ochrepad_fileio::normalize_line_endings;

use crate::keymap::{EditorCommand, Keymap};
use crate::layout::{LayoutCache, LineLayout, Styling, visual_col_at_x};

/// Supplies syntax highlighting for visible lines.
pub trait LineStyler {
    /// Spans for each line in `lines`; `None` draws a line unstyled.
    fn line_spans(
        &mut self,
        doc: &Document,
        lines: std::ops::Range<usize>,
    ) -> Vec<Option<ochrepad_syntax::LineSpans>>;
}

const BLINK_HALF_PERIOD: f64 = 0.53;
const SCROLLBAR_W: f32 = 12.0;
const TEXT_PAD: f32 = 4.0;
const MULTI_CLICK_TIME: f64 = 0.5;

#[derive(Clone, Copy, PartialEq, Eq)]
enum DragMode {
    Char,
    Word,
    Line,
    /// Alt+drag: a rectangle from (line, visual column).
    Column(usize, ochrepad_core::VisualCol),
}

struct Drag {
    mode: DragMode,
    /// The range the drag started from (a caret, word or line).
    anchor: (ByteOffset, ByteOffset),
}

#[derive(Clone, Copy, PartialEq)]
enum BarDrag {
    Vertical { grab: f32 },
    Horizontal { grab: f32 },
}

pub struct EditorViewState {
    cache: LayoutCache,
    pub font_size: f32,
    last_activity: f64,
    preedit: Option<String>,
    drag: Option<Drag>,
    bar_drag: Option<BarDrag>,
    last_press: Option<(f64, Pos2, u32)>,
    max_width: f32,
    reveal_caret: bool,
    request_focus: bool,
}

impl Default for EditorViewState {
    fn default() -> Self {
        Self {
            cache: LayoutCache::default(),
            font_size: 14.0,
            last_activity: 0.0,
            preedit: None,
            drag: None,
            bar_drag: None,
            last_press: None,
            max_width: 0.0,
            reveal_caret: true,
            request_focus: false,
        }
    }
}

impl EditorViewState {
    pub fn request_focus(&mut self) {
        self.request_focus = true;
    }

    /// Scroll the primary caret into view on the next frame.
    pub fn reveal_caret(&mut self) {
        self.reveal_caret = true;
    }

    pub fn preedit(&self) -> Option<&str> {
        self.preedit.as_deref()
    }
}

/// Geometry of one frame of the widget.
struct Frame {
    content: Rect,
    gutter: Rect,
    vbar: Rect,
    hbar: Rect,
    font: FontId,
    cell_w: f32,
    line_h: f32,
    rows_full: usize,
    line_count: usize,
    max_scroll: f64,
}

impl Frame {
    fn text_left(&self) -> f32 {
        self.content.min.x + TEXT_PAD
    }
}

enum Op {
    Apply(Action),
    Copy,
    Cut,
}

/// Optional extras drawn over the text.
#[derive(Default)]
pub struct Decorations<'a> {
    /// Syntax highlighting.
    pub styler: Option<&'a mut dyn LineStyler>,
    /// Highlights every match of the find panel's pattern.
    pub find_highlight: Option<&'a ochrepad_core::Searcher>,
}

/// Shows the editor for `view` filling the available space. `label` names the
/// file for accessibility.
pub fn editor_view(
    ui: &mut Ui,
    editor: &mut Editor,
    view: ViewId,
    state: &mut EditorViewState,
    keymap: &Keymap,
    label: &str,
    decorations: Decorations<'_>,
) -> Response {
    let Some(buffer_id) = editor.view(view).map(|v| v.buffer()) else {
        return ui.label("(no view)");
    };
    let rect = ui.available_rect_before_wrap();
    let id = editor_id(view);
    ui.advance_cursor_after_rect(rect);
    let response = ui.interact(rect, id, Sense::click_and_drag());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::TextEdit, true, label));

    let now = ui.input(|i| i.time);
    let tab_width = editor.settings.tab_width;
    let font = FontId::monospace(state.font_size);
    let (cell_w, line_h) = ui.fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));

    let geometry = |line_count: usize| {
        let digits = line_count.to_string().len().max(3);
        let gutter_w = (digits + 1) as f32 * cell_w + TEXT_PAD;
        let body = Rect::from_min_max(
            rect.min,
            Pos2::new(rect.max.x - SCROLLBAR_W, rect.max.y - SCROLLBAR_W),
        );
        let gutter = Rect::from_min_max(body.min, Pos2::new(body.min.x + gutter_w, body.max.y));
        let content = Rect::from_min_max(Pos2::new(gutter.max.x, body.min.y), body.max);
        let rows_full = ((content.height() / line_h).floor() as usize).max(1);
        Frame {
            vbar: Rect::from_min_max(
                Pos2::new(body.max.x, rect.min.y),
                Pos2::new(rect.max.x, body.max.y),
            ),
            hbar: Rect::from_min_max(
                Pos2::new(content.min.x, body.max.y),
                Pos2::new(body.max.x, rect.max.y),
            ),
            content,
            gutter,
            font: font.clone(),
            cell_w,
            line_h,
            rows_full,
            line_count,
            max_scroll: line_count.saturating_sub(rows_full) as f64,
        }
    };

    // --- Focus ---------------------------------------------------------------
    let pointer_pressed_here =
        ui.input(|i| i.pointer.primary_pressed()) && response.contains_pointer();
    if state.request_focus || pointer_pressed_here {
        response.request_focus();
        state.request_focus = false;
    }
    let focused = response.has_focus();
    if focused {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            )
        });
    }

    // --- Keyboard, clipboard and IME ------------------------------------------
    let page = {
        let line_count = doc_of(editor, buffer_id).line_count();
        geometry(line_count).rows_full.saturating_sub(1).max(1)
    };
    let mut ops = Vec::new();
    if focused {
        for event in ui.input(|i| i.events.clone()) {
            match event {
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => match keymap.lookup(key, modifiers) {
                    Some(EditorCommand::Action(a)) => ops.push(Op::Apply(a.clone())),
                    Some(EditorCommand::PageUp { extend }) => {
                        ops.push(Op::Apply(Action::MoveCursor {
                            motion: Motion::PageUp(page),
                            extend: *extend,
                        }))
                    }
                    Some(EditorCommand::PageDown { extend }) => {
                        ops.push(Op::Apply(Action::MoveCursor {
                            motion: Motion::PageDown(page),
                            extend: *extend,
                        }))
                    }
                    None => {}
                },
                Event::Text(t)
                    if state.preedit.is_none()
                        && !t.is_empty()
                        && !t.chars().any(char::is_control) =>
                {
                    ops.push(Op::Apply(Action::InsertText(t)));
                }
                Event::Paste(t) => {
                    let t = normalize_line_endings(&t).into_owned();
                    let block = ui
                        .ctx()
                        .data(|d| d.get_temp::<String>(rect_clip_id()))
                        .is_some_and(|clip| clip == t);
                    ops.push(Op::Apply(if block {
                        Action::PasteBlock(t)
                    } else {
                        Action::Paste(t)
                    }));
                }
                Event::Copy => ops.push(Op::Copy),
                Event::Cut => ops.push(Op::Cut),
                Event::Ime(ImeEvent::Preedit { text, .. }) => {
                    state.preedit = (!text.is_empty()).then_some(text);
                }
                Event::Ime(ImeEvent::Commit(t)) => {
                    state.preedit = None;
                    if !t.is_empty() {
                        ops.push(Op::Apply(Action::InsertText(t)));
                    }
                }
                _ => {}
            }
        }
    }
    for op in ops {
        match op {
            Op::Apply(a) => {
                let _ = editor.apply(view, a);
            }
            Op::Copy | Op::Cut => {
                if copy_selection(ui.ctx(), editor, view) && matches!(op, Op::Cut) {
                    let _ = editor.apply(view, Action::InsertText(String::new()));
                }
            }
        }
        state.reveal_caret = true;
        state.last_activity = now;
    }

    // From here on the document only changes through mouse selection.
    let f = geometry(doc_of(editor, buffer_id).line_count());
    let (mut scroll, mut x_off) = editor
        .view(view)
        .map(|v| {
            (
                v.scroll.first_line as f64 + v.scroll.line_fraction as f64,
                v.scroll.x_offset,
            )
        })
        .unwrap_or_default();

    // --- Wheel -------------------------------------------------------------------
    if response.contains_pointer() {
        let d = ui.input(|i| i.smooth_scroll_delta);
        if d != Vec2::ZERO {
            scroll -= (d.y / line_h) as f64;
            x_off -= d.x;
            ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        }
    }

    // --- Scrollbars ---------------------------------------------------------------
    let h_max = (state.max_width + 4.0 * cell_w - f.content.width()).max(0.0);
    let (v_thumb, h_thumb) = thumbs(&f, scroll, x_off, h_max, state.max_width);
    let (pressed, down, pointer, dt) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.pointer.interact_pos(),
            i.stable_dt,
        )
    });
    if pressed
        && let Some(p) = pointer
        && response.contains_pointer()
    {
        if f.vbar.contains(p) {
            let grab = if v_thumb.contains(p) {
                p.y - v_thumb.min.y
            } else {
                v_thumb.height() / 2.0
            };
            state.bar_drag = Some(BarDrag::Vertical { grab });
        } else if f.hbar.contains(p) && h_max > 0.0 {
            let grab = if h_thumb.contains(p) {
                p.x - h_thumb.min.x
            } else {
                h_thumb.width() / 2.0
            };
            state.bar_drag = Some(BarDrag::Horizontal { grab });
        }
    }
    if !down {
        state.bar_drag = None;
    }
    if let (Some(bar), Some(p)) = (state.bar_drag, pointer) {
        match bar {
            BarDrag::Vertical { grab } => {
                let track = f.vbar.height() - v_thumb.height();
                let t = ((p.y - grab - f.vbar.min.y) / track.max(1.0)).clamp(0.0, 1.0);
                scroll = t as f64 * f.max_scroll;
            }
            BarDrag::Horizontal { grab } => {
                let track = f.hbar.width() - h_thumb.width();
                let t = ((p.x - grab - f.hbar.min.x) / track.max(1.0)).clamp(0.0, 1.0);
                x_off = t * h_max;
            }
        }
    }
    scroll = scroll.clamp(0.0, f.max_scroll);
    x_off = x_off.clamp(0.0, h_max.max(x_off.min(state.max_width)));

    // --- Mouse in the text area ------------------------------------------------
    if pressed
        && state.bar_drag.is_none()
        && let Some(p) = pointer
        && response.contains_pointer()
        && (f.content.contains(p) || f.gutter.contains(p))
    {
        let count = match state.last_press {
            Some((t, lp, c)) if now - t < MULTI_CLICK_TIME && lp.distance(p) < 4.0 => c % 3 + 1,
            _ => 1,
        };
        state.last_press = Some((now, p, count));
        let hit = hit_test(
            editor,
            buffer_id,
            &mut state.cache,
            &f,
            scroll,
            x_off,
            tab_width,
            ui,
            p,
        );
        let mods = ui.input(|i| i.modifiers);
        let shift = mods.shift;
        // A click on a line number selects the whole line (and dragging
        // extends by lines); Shift+click extends the selection by lines.
        let in_gutter = f.gutter.contains(p);
        let prev = editor.view(view).map(|v| v.selections.primary().clone());
        let mode = match count {
            _ if in_gutter => DragMode::Line,
            _ if mods.alt => {
                let (line, col) = line_col_at(&f, scroll, x_off, p);
                DragMode::Column(line, col)
            }
            1 => DragMode::Char,
            2 => DragMode::Word,
            _ => DragMode::Line,
        };
        let anchor = if mode == DragMode::Char && shift {
            editor
                .view(view)
                .map_or(hit, |v| v.selections.primary().anchor)
        } else {
            hit
        };
        if let DragMode::Column(line, col) = mode {
            let _ = editor.apply(
                view,
                Action::ColumnSelect {
                    anchor_line: line,
                    anchor_col: col,
                    head_line: line,
                    head_col: col,
                },
            );
        } else if mods.command && count == 1 && !in_gutter {
            // Ctrl+click adds a caret; it becomes the primary.
            let mut ranges = editor
                .view(view)
                .map(|v| v.selections.ranges().to_vec())
                .unwrap_or_default();
            ranges.push(Selection::caret(hit));
            let primary = ranges.len() - 1;
            let _ = editor.apply(
                view,
                Action::SetSelections(SelectionSet::new(ranges, primary)),
            );
        } else {
            let _ = editor.apply(
                view,
                Action::SetSelections(SelectionSet::single(Selection::new(anchor, hit))),
            );
        }
        match mode {
            DragMode::Word => {
                let _ = editor.apply(view, Action::SelectWord);
            }
            DragMode::Line => {
                let _ = editor.apply(view, Action::SelectLine);
            }
            DragMode::Char | DragMode::Column(..) => {}
        }
        let sel = editor.view(view).map(|v| v.selections.primary().clone());
        let mut range = sel.clone().map_or((anchor, anchor), |s| {
            if mode == DragMode::Char {
                (anchor, anchor)
            } else {
                (s.start(), s.end())
            }
        });
        if in_gutter
            && shift
            && let (Some(prev), Some(line)) = (prev, sel)
        {
            // Extend from where the previous selection started: if it is
            // whole lines, from the line at its anchor end (as a line drag
            // keeps the line it started on), else from its anchor.
            let doc = doc_of(editor, buffer_id);
            let at_line_start = |b: ByteOffset| b == doc.line_start(doc.line_of(b));
            let whole_lines = !prev.is_empty()
                && at_line_start(prev.start())
                && (at_line_start(prev.end()) || prev.end() == doc.len());
            let a = prev.anchor;
            let (a0, a1) = if !whole_lines {
                (a, a)
            } else if a < prev.head {
                (a, next_line_start(doc, doc.line_of(a)))
            } else {
                (doc.line_start(doc.line_of(ByteOffset(a.0 - 1))), a)
            };
            let new = if line.start() >= a0 {
                Selection::new(a0, line.end().max(a1))
            } else {
                Selection::new(a1, line.start())
            };
            let _ = editor.apply(view, Action::SetSelections(SelectionSet::single(new)));
            range = (a0, a1);
        }
        // A Ctrl+click only adds a caret; it does not start a drag.
        state.drag = (in_gutter || !(mods.command && count == 1 && !mods.alt)).then_some(Drag {
            mode,
            anchor: range,
        });
        state.last_activity = now;
    }
    if !down {
        state.drag = None;
    }
    if let (Some(drag), Some(p)) = (&state.drag, pointer) {
        // Autoscroll when dragging past the edges.
        let speed = |dist: f32| (4.0 + 20.0 * dist / line_h) * dt;
        if p.y < f.content.min.y {
            scroll -= speed(f.content.min.y - p.y) as f64;
        } else if p.y > f.content.max.y {
            scroll += speed(p.y - f.content.max.y) as f64;
        }
        if p.x > f.content.max.x {
            x_off += speed(p.x - f.content.max.x) * cell_w;
        } else if p.x < f.content.min.x && x_off > 0.0 {
            x_off = (x_off - speed(f.content.min.x - p.x) * cell_w).max(0.0);
        }
        if !f.content.contains(p) {
            ui.ctx().request_repaint();
        }
        scroll = scroll.clamp(0.0, f.max_scroll);
        let clamped = p.clamp(f.content.min, f.content.max - Vec2::splat(0.5));
        let head = hit_test(
            editor,
            buffer_id,
            &mut state.cache,
            &f,
            scroll,
            x_off,
            tab_width,
            ui,
            clamped,
        );
        if let DragMode::Column(anchor_line, anchor_col) = drag.mode {
            let (head_line, head_col) = line_col_at(&f, scroll, x_off, clamped);
            let rect = editor.view(view).and_then(|v| v.selections.rect().cloned());
            if rect.is_none_or(|r| (r.end_line, r.end_col) != (head_line, head_col)) {
                let _ = editor.apply(
                    view,
                    Action::ColumnSelect {
                        anchor_line,
                        anchor_col,
                        head_line,
                        head_col,
                    },
                );
                state.last_activity = now;
            }
        }
        let doc = doc_of(editor, buffer_id);
        let (a0, a1) = drag.anchor;
        let sel = match drag.mode {
            DragMode::Column(..) => None,
            DragMode::Char => Some(Selection::new(a0, head)),
            DragMode::Word if head < a0 => Some(Selection::new(a1, head)),
            DragMode::Word if head > a1 => Some(Selection::new(a0, head)),
            DragMode::Word => Some(Selection::new(a0, a1)),
            DragMode::Line => {
                let line = doc.line_of(head);
                Some(if head >= a0 {
                    Selection::new(a0, next_line_start(doc, line).max(a1))
                } else {
                    Selection::new(a1, doc.line_start(line))
                })
            }
        };
        let current = editor.view(view).map(|v| v.selections.ranges().to_vec());
        if let Some(sel) = sel
            && current.as_deref() != Some(std::slice::from_ref(&sel))
        {
            let _ = editor.apply(view, Action::SetSelections(SelectionSet::single(sel)));
            state.last_activity = now;
        }
    }

    // --- Keep the primary caret in view after keyboard actions -----------------
    if state.reveal_caret {
        state.reveal_caret = false;
        if let Some(head) = editor.view(view).map(|v| v.selections.primary().head) {
            let doc = doc_of(editor, buffer_id);
            let line = doc.line_of(head);
            if (line as f64) < scroll {
                scroll = line as f64;
            } else if line as f64 + 1.0 > scroll + f.rows_full as f64 {
                scroll = line as f64 + 1.0 - f.rows_full as f64;
            }
            scroll = scroll.clamp(0.0, f.max_scroll);
            let layout = line_layout(
                editor,
                buffer_id,
                &mut state.cache,
                line,
                &f,
                tab_width,
                ui,
                Styling::Any,
            );
            let x = layout.x_of(head.0 - doc.line_start(line).0);
            let margin = 4.0 * cell_w;
            let visible = f.content.width() - TEXT_PAD - cell_w;
            if x < x_off {
                x_off = (x - margin).max(0.0);
            } else if x > x_off + visible {
                x_off = x - visible + margin;
            }
        }
    }

    if let Some(v) = editor.view_mut(view) {
        v.scroll.first_line = scroll.floor() as usize;
        v.scroll.line_fraction = scroll.fract() as f32;
        v.scroll.x_offset = x_off;
    }

    if response.hovered()
        && ui
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|p| f.content.contains(p))
    {
        ui.ctx().set_cursor_icon(CursorIcon::Text);
    }

    paint(
        ui,
        editor,
        view,
        buffer_id,
        state,
        &f,
        Scroll {
            scroll,
            x_off,
            h_max,
        },
        focused,
        now,
        tab_width,
        decorations.styler,
        decorations.find_highlight,
    );
    response
}

/// Draws a line's text from its layout with its first cell at `x0`, row top
/// at `y`. Pieces outside the painter's clip rect are skipped; non-ASCII
/// graphemes are clipped to their cells. Shared by the editor and the
/// compare view so there is one text renderer.
pub(crate) fn paint_line_text(
    painter: &egui::Painter,
    layout: &LineLayout,
    x0: f32,
    y: f32,
    font: &FontId,
    line_h: f32,
) {
    let clip = painter.clip_rect();
    for piece in &layout.pieces {
        let px = x0 + piece.x;
        if px > clip.max.x {
            break;
        }
        if px + piece.width < clip.min.x {
            continue;
        }
        let galley = painter.layout_no_wrap(
            layout.text[piece.range.clone()].to_owned(),
            font.clone(),
            piece.color,
        );
        if piece.clip {
            let slot = Rect::from_min_size(Pos2::new(px, y), Vec2::new(piece.width, line_h));
            painter.with_clip_rect(slot.intersect(clip)).galley(
                Pos2::new(px, y),
                galley,
                piece.color,
            );
        } else {
            painter.galley(Pos2::new(px, y), galley, piece.color);
        }
    }
}

/// The egui id of the editor widget for `view` (it holds keyboard focus).
pub fn editor_id(view: ViewId) -> Id {
    Id::new(("ochrepad-editor", view.0))
}

fn doc_of(editor: &Editor, buffer: ochrepad_core::BufferId) -> &Document {
    editor.buffer(buffer).expect("view's buffer exists").doc()
}

fn next_line_start(doc: &Document, line: usize) -> ByteOffset {
    if line + 1 < doc.line_count() {
        doc.line_start(line + 1)
    } else {
        doc.len()
    }
}

/// Selected text joined with newlines: every non-empty selection, or for a
/// rectangle every line of it (short lines contribute empty lines).
pub fn selected_text(editor: &Editor, view: ViewId) -> String {
    let Some(v) = editor.view(view) else {
        return String::new();
    };
    let Some(b) = editor.buffer(v.buffer()) else {
        return String::new();
    };
    let rect = v.selections.rect().is_some();
    if rect && v.selections.ranges().iter().all(Selection::is_empty) {
        return String::new();
    }
    v.selections
        .ranges()
        .iter()
        .filter(|s| rect || !s.is_empty())
        .map(|s| b.doc().slice(s.range()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Where the text of the last rectangular copy is remembered, so pasting it
/// back pastes a block (the clipboard itself carries no such flag in egui).
fn rect_clip_id() -> Id {
    Id::new("ochrepad-rect-clip")
}

/// Copies the selection to the clipboard; returns whether anything was
/// copied. Rectangular copies are remembered for block paste.
pub fn copy_selection(ctx: &egui::Context, editor: &Editor, view: ViewId) -> bool {
    let text = selected_text(editor, view);
    if text.is_empty() {
        return false;
    }
    let rect = editor
        .view(view)
        .is_some_and(|v| v.selections.rect().is_some());
    ctx.data_mut(|d| {
        if rect {
            d.insert_temp(rect_clip_id(), text.clone());
        } else {
            d.remove::<String>(rect_clip_id());
        }
    });
    ctx.copy_text(text);
    true
}

/// Line and visual column under `p` (columns may lie past the line end).
fn line_col_at(f: &Frame, scroll: f64, x_off: f32, p: Pos2) -> (usize, ochrepad_core::VisualCol) {
    let row = ((p.y - f.content.min.y) / f.line_h) as f64 + scroll;
    let line = (row.max(0.0).floor() as usize).min(f.line_count - 1);
    (line, visual_col_at_x(p.x - f.text_left() + x_off, f.cell_w))
}

#[allow(clippy::too_many_arguments)]
fn line_layout(
    editor: &Editor,
    buffer: ochrepad_core::BufferId,
    cache: &mut LayoutCache,
    line: usize,
    f: &Frame,
    tab_width: usize,
    ui: &Ui,
    styling: Styling<'_>,
) -> std::sync::Arc<LineLayout> {
    let b = editor.buffer(buffer).expect("view's buffer exists");
    cache.get(
        b.doc(),
        (buffer, b.revision()),
        line,
        f.cell_w,
        tab_width,
        ui.visuals().text_color(),
        styling,
    )
}

/// Pixel -> byte offset, through the same `LineLayout` painting uses.
#[allow(clippy::too_many_arguments)]
fn hit_test(
    editor: &Editor,
    buffer: ochrepad_core::BufferId,
    cache: &mut LayoutCache,
    f: &Frame,
    scroll: f64,
    x_off: f32,
    tab_width: usize,
    ui: &Ui,
    p: Pos2,
) -> ByteOffset {
    let doc = doc_of(editor, buffer);
    let row = ((p.y - f.content.min.y) / f.line_h) as f64 + scroll;
    let line = (row.max(0.0).floor() as usize).min(f.line_count - 1);
    let layout = line_layout(editor, buffer, cache, line, f, tab_width, ui, Styling::Any);
    let x = p.x - f.text_left() + x_off;
    ByteOffset(doc.line_start(line).0 + layout.byte_at_x(x))
}

fn thumbs(f: &Frame, scroll: f64, x_off: f32, h_max: f32, max_width: f32) -> (Rect, Rect) {
    let total = (f.line_count as f32).max(1.0);
    let vh = (f.vbar.height() * (f.rows_full as f32 / total).min(1.0))
        .max(20.0)
        .min(f.vbar.height());
    let vt = if f.max_scroll > 0.0 {
        (scroll / f.max_scroll) as f32
    } else {
        0.0
    };
    let v = Rect::from_min_size(
        Pos2::new(f.vbar.min.x, f.vbar.min.y + (f.vbar.height() - vh) * vt),
        Vec2::new(SCROLLBAR_W, vh),
    );
    let total_w = (max_width + 4.0 * f.cell_w).max(f.content.width());
    let hw = (f.hbar.width() * f.content.width() / total_w)
        .max(20.0)
        .min(f.hbar.width());
    let ht = if h_max > 0.0 { x_off / h_max } else { 0.0 };
    let h = Rect::from_min_size(
        Pos2::new(
            f.hbar.min.x + (f.hbar.width() - hw) * ht.min(1.0),
            f.hbar.min.y,
        ),
        Vec2::new(hw, SCROLLBAR_W),
    );
    (v, h)
}

struct Scroll {
    scroll: f64,
    x_off: f32,
    h_max: f32,
}

#[allow(clippy::too_many_arguments)]
fn paint(
    ui: &mut Ui,
    editor: &Editor,
    view: ViewId,
    buffer: ochrepad_core::BufferId,
    state: &mut EditorViewState,
    f: &Frame,
    Scroll {
        scroll,
        x_off,
        h_max,
    }: Scroll,
    focused: bool,
    now: f64,
    tab_width: usize,
    styler: Option<&mut dyn LineStyler>,
    find_highlight: Option<&ochrepad_core::Searcher>,
) {
    let Some(v) = editor.view(view) else { return };
    let doc = doc_of(editor, buffer);
    let sels = v.selections.ranges();
    let visuals = ui.visuals().clone();
    let painter = ui.painter().clone();
    let text_painter = painter.with_clip_rect(f.content.intersect(painter.clip_rect()));
    let gutter_painter = painter.with_clip_rect(f.gutter.intersect(painter.clip_rect()));

    painter.rect_filled(f.content, 0.0, visuals.extreme_bg_color);
    painter.rect_filled(f.gutter, 0.0, visuals.extreme_bg_color);
    painter.rect_filled(f.vbar.union(f.hbar), 0.0, visuals.extreme_bg_color);

    let first = scroll.floor() as usize;
    let y0 = f.content.min.y - scroll.fract() as f32 * f.line_h;
    let last = (first + f.rows_full + 2).min(f.line_count);
    let x0 = f.text_left() - x_off;
    let text_color = visuals.text_color();
    let current_line_bg = visuals.faint_bg_color;
    let sel_bg = visuals.selection.bg_fill;
    let spans = styler.map(|s| s.line_spans(doc, first..last));
    // Every match of the find panel's pattern on the visible lines.
    let matches = match find_highlight {
        Some(s) if first < last => s.find_all(doc, doc.line_start(first)..doc.line_end(last - 1)),
        _ => Vec::new(),
    };
    let match_bg = crate::theme::match_bg(&visuals);
    let accent = crate::theme::accent(&visuals);

    for line in first..last {
        let y = y0 + (line - first) as f32 * f.line_h;
        let styling = Styling::Exact(
            spans
                .as_ref()
                .and_then(|s| s.get(line - first))
                .and_then(Option::as_ref),
        );
        let layout = line_layout(
            editor,
            buffer,
            &mut state.cache,
            line,
            f,
            tab_width,
            ui,
            styling,
        );
        state.max_width = state.max_width.max(layout.width);
        let line_start = doc.line_start(line);
        let line_end = doc.line_end(line);
        let row = Rect::from_min_size(
            Pos2::new(f.content.min.x, y),
            Vec2::new(f.content.width(), f.line_h),
        );

        // Selections overlapping this line (they are sorted).
        let from = sels.partition_point(|s| s.end() < line_start);
        let on_line = sels[from..].iter().take_while(|s| s.start() <= line_end);
        let mut has_caret = false;
        for s in on_line.clone() {
            has_caret |= s.head >= line_start && s.head <= line_end;
        }
        if has_caret {
            text_painter.rect_filled(row, 0.0, current_line_bg);
        }
        let from = matches.partition_point(|m| m.end <= line_start && m.start < line_start);
        for m in matches[from..].iter().take_while(|m| m.start <= line_end) {
            if m.start == m.end {
                continue;
            }
            let a = m.start.max(line_start).0 - line_start.0;
            let b = m.end.min(line_end).0 - line_start.0;
            let r = Rect::from_min_max(
                Pos2::new(x0 + layout.x_of(a), y),
                Pos2::new(x0 + layout.x_of(b).max(layout.x_of(a) + 2.0), y + f.line_h),
            );
            text_painter.rect_filled(r, 2.0, match_bg);
        }
        for s in on_line {
            if s.is_empty() {
                continue;
            }
            let a = s.start().max(line_start).0 - line_start.0;
            let b = s.end().min(line_end).0 - line_start.0;
            let mut x1 = x0 + layout.x_of(b);
            if s.end() > line_end {
                x1 += f.cell_w; // the selected newline
            }
            let r = Rect::from_min_max(
                Pos2::new(x0 + layout.x_of(a), y),
                Pos2::new(x1, y + f.line_h),
            );
            text_painter.rect_filled(r, 0.0, sel_bg);
        }

        paint_line_text(&text_painter, &layout, x0, y, &f.font, f.line_h);

        gutter_painter.text(
            Pos2::new(f.gutter.max.x - f.cell_w, y),
            Align2::RIGHT_TOP,
            (line + 1).to_string(),
            f.font.clone(),
            if has_caret {
                accent
            } else {
                visuals.weak_text_color()
            },
        );
    }

    // Carets and IME pre-edit.
    let caret_pos = |head: ByteOffset, cache: &mut LayoutCache| {
        let line = doc.line_of(head);
        let layout = line_layout(editor, buffer, cache, line, f, tab_width, ui, Styling::Any);
        let x = x0 + layout.x_of(head.0 - doc.line_start(line).0);
        Pos2::new(x, y0 + (line as f32 - first as f32) * f.line_h)
    };
    if focused {
        let since = now - state.last_activity;
        let on = ((since / BLINK_HALF_PERIOD) as u64).is_multiple_of(2);
        if on {
            let caret_stroke = Stroke::new(2.0, visuals.text_cursor.stroke.color);
            for s in sels {
                let line = doc.line_of(s.head);
                if line < first || line >= last {
                    continue;
                }
                let p = caret_pos(s.head, &mut state.cache);
                text_painter.line_segment([p, p + Vec2::new(0.0, f.line_h)], caret_stroke);
            }
        }
        let to_toggle = BLINK_HALF_PERIOD - since.rem_euclid(BLINK_HALF_PERIOD);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(to_toggle));

        let p = caret_pos(v.selections.primary().head, &mut state.cache);
        if let Some(pre) = &state.preedit {
            let galley = text_painter.layout_no_wrap(pre.clone(), f.font.clone(), text_color);
            let r = Rect::from_min_size(p, Vec2::new(galley.size().x, f.line_h));
            text_painter.rect_filled(r, 0.0, visuals.extreme_bg_color);
            text_painter.galley(p, galley, text_color);
            text_painter.line_segment(
                [r.left_bottom(), r.right_bottom()],
                Stroke::new(1.0, text_color),
            );
        }
        ui.ctx().output_mut(|o| {
            o.ime = Some(egui::output::IMEOutput {
                purpose: egui::IMEPurpose::Normal,
                rect: f.content,
                cursor_rect: Rect::from_min_size(p, Vec2::new(2.0, f.line_h)),
                should_interrupt_composition: false,
            })
        });
    }

    // Scrollbars.
    let (v_thumb, h_thumb) = thumbs(f, scroll, x_off, h_max, state.max_width);
    let thumb_color = visuals.widgets.hovered.bg_fill;
    painter.rect_filled(v_thumb.shrink(2.0), 3.0, thumb_color);
    if h_max > 0.0 {
        painter.rect_filled(h_thumb.shrink(2.0), 3.0, thumb_color);
    }
    state.cache.retain_near(first, last.max(first + 1));
}
