//! The editor widget: renders one `View` and turns input into `Action`s.

use egui::{
    Align2, CursorIcon, Event, EventFilter, FontId, Id, ImeEvent, Pos2, Rect, Response, Sense,
    Stroke, Ui, Vec2, WidgetInfo, WidgetType,
};
use ochrepad_core::{
    Action, ByteOffset, DisplayMap, Document, Editor, Motion, RowPos, SearchOptions, SearchQuery,
    Searcher, Selection, SelectionSet, ViewId,
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
    /// The selected text and its searcher, for smart highlighting.
    smart: Option<(String, Option<Searcher>)>,
    /// Show the fold margin (chevrons) in the gutter.
    pub fold_margin: bool,
    /// Wrap lines at the pane's width.
    pub word_wrap: bool,
}

impl Default for EditorViewState {
    fn default() -> Self {
        Self {
            fold_margin: true,
            word_wrap: false,
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
            smart: None,
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
    /// Lines that are not hidden, for the scrollbar.
    visible_lines: usize,
    /// Width of the gutter's right part, after the line numbers: the fold
    /// margin, or one cell of padding without it.
    fold_w: f32,
    fold_margin: bool,
}

impl Frame {
    fn text_left(&self) -> f32 {
        self.content.min.x + TEXT_PAD
    }

    /// Whether `p` is in the fold margin.
    fn in_fold_margin(&self, p: Pos2) -> bool {
        self.fold_margin && self.gutter.contains(p) && p.x >= self.gutter.max.x - self.fold_w
    }

    /// The `⋯` marker after a folded line whose text ends at `x_end`.
    fn fold_pill(&self, x_end: f32, y: f32) -> Rect {
        let h = (self.line_h * 0.7).round();
        Rect::from_min_size(
            Pos2::new(x_end + self.cell_w, y + (self.line_h - h) / 2.0),
            Vec2::new(self.cell_w * 3.0, h),
        )
    }
}

/// The scroll position: the top display row and the fraction of it
/// scrolled past.
#[derive(Clone, Copy, PartialEq)]
struct Top {
    row: RowPos,
    frac: f32,
}

impl Top {
    /// The scrollbar offset (in visible lines).
    fn offset(self, rows: &DisplayMap) -> f64 {
        rows.offset_of(self.row, self.frac)
    }

    fn at_offset(rows: &DisplayMap, offset: f64) -> Self {
        let (row, frac) = rows.pos_at_offset(offset);
        Top { row, frac }
    }

    fn scrolled(self, rows: &DisplayMap, by: f64) -> Self {
        let (row, frac) = rows.scroll_by(self.row, self.frac, by);
        Top { row, frac }
    }

    /// Keeps the last row from scrolling above the bottom of the viewport.
    fn clamped(self, rows: &DisplayMap, rows_full: usize) -> Self {
        let max = rows.max_top(rows_full);
        if self.row > max || (self.row == max && self.frac > 0.0) {
            Top {
                row: max,
                frac: 0.0,
            }
        } else {
            self
        }
    }
}

/// The display rows of `view`.
fn display_map(editor: &Editor, view: ViewId) -> DisplayMap<'_> {
    let v = editor.view(view).expect("view exists");
    let b = editor.buffer(v.buffer()).expect("view's buffer exists");
    DisplayMap::new(b, v, editor.settings.tab_width)
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
    let font = FontId::monospace(state.font_size);
    let (cell_w, line_h) = ui.fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));

    let fold_margin = state.fold_margin;
    let fold_w = if fold_margin { 2.0 } else { 1.0 } * cell_w;
    let wrap = state.word_wrap;
    // No horizontal scrollbar while lines wrap.
    let hbar_h = if wrap { 0.0 } else { SCROLLBAR_W };
    let geometry = |line_count: usize, visible_lines: usize| {
        let digits = line_count.to_string().len().max(3);
        let gutter_w = digits as f32 * cell_w + fold_w + TEXT_PAD;
        let body = Rect::from_min_max(
            rect.min,
            Pos2::new(rect.max.x - SCROLLBAR_W, rect.max.y - hbar_h),
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
            visible_lines,
            fold_w,
            fold_margin,
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

    // The wrap width follows the pane (the core's motions use it). One cell
    // stays free at the right for the caret at the end of a full row.
    let wrap_cols = wrap.then(|| {
        let line_count = doc_of(editor, buffer_id).line_count();
        let content = geometry(line_count, line_count).content;
        ((content.width() - TEXT_PAD - cell_w) / cell_w)
            .floor()
            .max(1.0) as usize
    });
    if let Some(v) = editor.view_mut(view)
        && v.wrap_cols != wrap_cols
    {
        v.wrap_cols = wrap_cols;
    }

    // --- Keyboard, clipboard and IME ------------------------------------------
    let page = {
        let line_count = doc_of(editor, buffer_id).line_count();
        geometry(line_count, line_count)
            .rows_full
            .saturating_sub(1)
            .max(1)
    };
    let mut ops = Vec::new();
    if focused {
        for event in ui.input(|i| i.events.clone()) {
            match event {
                Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    modifiers,
                    ..
                } => match keymap.lookup(key, modifiers).or_else(|| {
                    // With Shift, a digit arrives as its symbol (Alt+Shift+1
                    // as "!"); the physical key still matches Alt+Shift+1.
                    physical_key.and_then(|k| keymap.lookup(k, modifiers))
                }) {
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

    // From here on the document only changes through mouse selection, which
    // does not change the display rows.
    let f = {
        let rows = display_map(editor, view);
        geometry(rows.doc().line_count(), rows.visible_line_count())
    };
    let (mut top, mut x_off) = editor
        .view(view)
        .map(|v| {
            (
                Top {
                    row: RowPos::new(v.scroll.first_line, v.scroll.first_sub),
                    frac: v.scroll.line_fraction,
                },
                v.scroll.x_offset,
            )
        })
        .expect("view exists");
    let rows = display_map(editor, view);
    // The document may have shrunk, or rewrapped, since the last frame.
    top.row = rows.clamp_row(top.row);
    top = top.clamped(&rows, f.rows_full);
    if wrap {
        x_off = 0.0;
    }
    let max_offset = Top {
        row: rows.max_top(f.rows_full),
        frac: 0.0,
    }
    .offset(&rows);

    // --- Wheel -------------------------------------------------------------------
    if response.contains_pointer() {
        let d = ui.input(|i| i.smooth_scroll_delta);
        if d != Vec2::ZERO {
            top = top.scrolled(&rows, -(d.y / line_h) as f64);
            x_off -= d.x;
            ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        }
    }

    // --- Scrollbars ---------------------------------------------------------------
    let h_max = if wrap {
        0.0
    } else {
        (state.max_width + 4.0 * cell_w - f.content.width()).max(0.0)
    };
    let (v_thumb, h_thumb) = thumbs(
        &f,
        top.offset(&rows),
        max_offset,
        x_off,
        h_max,
        state.max_width,
    );
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
                top = Top::at_offset(&rows, t as f64 * max_offset);
            }
            BarDrag::Horizontal { grab } => {
                let track = f.hbar.width() - h_thumb.width();
                let t = ((p.x - grab - f.hbar.min.x) / track.max(1.0)).clamp(0.0, 1.0);
                x_off = t * h_max;
            }
        }
    }
    top = top.clamped(&rows, f.rows_full);
    x_off = x_off.clamp(0.0, h_max.max(x_off.min(state.max_width)));

    // --- Fold margin and folded-line markers ------------------------------------
    let mut fold_click = false;
    if pressed
        && state.bar_drag.is_none()
        && let Some(p) = pointer
        && response.contains_pointer()
        && let Some(line) = fold_target(editor, view, &mut state.cache, &f, top, x_off, ui, p)
    {
        editor.toggle_fold_at(view, line);
        fold_click = true;
        state.last_activity = now;
    }

    // --- Mouse in the text area ------------------------------------------------
    if pressed
        && !fold_click
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
        let hit = hit_test(editor, view, &mut state.cache, &f, top, x_off, ui, p);
        let mods = ui.input(|i| i.modifiers);
        let shift = mods.shift;
        // A click on a line number selects the whole line (and dragging
        // extends by lines); Shift+click extends the selection by lines.
        let in_gutter = f.gutter.contains(p);
        let prev = editor.view(view).map(|v| v.selections.primary().clone());
        let mode = match count {
            _ if in_gutter => DragMode::Line,
            _ if mods.alt => {
                let (line, col) =
                    line_col_at(editor, view, &mut state.cache, &f, top, x_off, ui, p);
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
        let rows = display_map(editor, view);
        if p.y < f.content.min.y {
            top = top.scrolled(&rows, -speed(f.content.min.y - p.y) as f64);
        } else if p.y > f.content.max.y {
            top = top.scrolled(&rows, speed(p.y - f.content.max.y) as f64);
        }
        if p.x > f.content.max.x {
            x_off += speed(p.x - f.content.max.x) * cell_w;
        } else if p.x < f.content.min.x && x_off > 0.0 {
            x_off = (x_off - speed(f.content.min.x - p.x) * cell_w).max(0.0);
        }
        if !f.content.contains(p) {
            ui.ctx().request_repaint();
        }
        top = top.clamped(&rows, f.rows_full);
        let clamped = p.clamp(f.content.min, f.content.max - Vec2::splat(0.5));
        let head = hit_test(editor, view, &mut state.cache, &f, top, x_off, ui, clamped);
        if let DragMode::Column(anchor_line, anchor_col) = drag.mode {
            let (head_line, head_col) =
                line_col_at(editor, view, &mut state.cache, &f, top, x_off, ui, clamped);
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
            let rows = display_map(editor, view);
            let row = rows.row_of(head);
            let line = row.line;
            // The lowest top row that still shows the caret's row.
            let lowest = rows.advance(row, -(f.rows_full as isize - 1));
            if row < top.row || (row == top.row && top.frac > 0.0) {
                top = Top { row, frac: 0.0 };
            } else if top.row < lowest {
                top = Top {
                    row: lowest,
                    frac: 0.0,
                };
            }
            top = top.clamped(&rows, f.rows_full);
            let row_start = rows.row_range(row).start;
            let layout = line_layout(
                editor,
                buffer_id,
                &mut state.cache,
                line,
                &f,
                ui,
                Styling::Any,
            );
            let line_start = doc.line_start(line).0;
            let x = layout.x_of(head.0 - line_start) - layout.x_of(row_start.0 - line_start);
            let margin = 4.0 * cell_w;
            let visible = f.content.width() - TEXT_PAD - cell_w;
            if wrap {
                // Rows fit the pane.
            } else if x < x_off {
                x_off = (x - margin).max(0.0);
            } else if x > x_off + visible {
                x_off = x - visible + margin;
            }
        }
    }

    if let Some(v) = editor.view_mut(view) {
        v.scroll.first_line = top.row.line;
        v.scroll.first_sub = top.row.sub;
        v.scroll.line_fraction = top.frac;
        v.scroll.x_offset = x_off;
    }

    let hover = ui
        .input(|i| i.pointer.hover_pos())
        .filter(|_| response.hovered());
    if let Some(p) = hover {
        if fold_target(editor, view, &mut state.cache, &f, top, x_off, ui, p).is_some() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        } else if f.content.contains(p) {
            ui.ctx().set_cursor_icon(CursorIcon::Text);
        }
    }
    // Fold chevrons for every header show while the pointer is on the gutter.
    let show_chevrons = hover.is_some_and(|p| f.gutter.contains(p));

    paint(
        ui,
        editor,
        view,
        buffer_id,
        state,
        &f,
        Scroll {
            top,
            max_offset,
            x_off,
            h_max,
        },
        focused,
        now,
        show_chevrons,
        decorations.styler,
        decorations.find_highlight,
    );
    response
}

/// A fold chevron in the fold margin of the row at `y`: pointing right when
/// folded, down when open.
fn paint_chevron(
    painter: &egui::Painter,
    f: &Frame,
    visuals: &egui::Visuals,
    y: f32,
    folded: bool,
) {
    let c = Pos2::new(f.gutter.max.x - f.fold_w / 2.0, y + f.line_h / 2.0);
    let s = (f.cell_w * 0.3).round().max(3.0);
    let points = if folded {
        [
            c + Vec2::new(-s / 2.0, -s),
            c + Vec2::new(s / 2.0, 0.0),
            c + Vec2::new(-s / 2.0, s),
        ]
    } else {
        [
            c + Vec2::new(-s, -s / 2.0),
            c + Vec2::new(0.0, s / 2.0),
            c + Vec2::new(s, -s / 2.0),
        ]
    };
    let color = if folded {
        visuals.text_color()
    } else {
        visuals.weak_text_color()
    };
    painter.line(points.to_vec(), Stroke::new(1.5, color));
}

/// The `⋯` marker after a folded line whose text ends at `x_end`.
fn paint_fold_pill(
    painter: &egui::Painter,
    f: &Frame,
    visuals: &egui::Visuals,
    x_end: f32,
    y: f32,
) {
    let r = f.fold_pill(x_end, y);
    painter.rect_filled(r, r.height() / 2.0, visuals.widgets.inactive.bg_fill);
    let dot = (f.cell_w * 0.12).max(1.2);
    for i in [-1.0, 0.0, 1.0] {
        let p = r.center() + Vec2::new(i * f.cell_w * 0.75, 0.0);
        painter.circle_filled(p, dot, visuals.weak_text_color());
    }
}

/// The header line a click at `p` folds or unfolds: a fold header (or a
/// folded line) in the fold margin, or a folded line's `⋯` marker.
#[allow(clippy::too_many_arguments)]
fn fold_target(
    editor: &Editor,
    view: ViewId,
    cache: &mut LayoutCache,
    f: &Frame,
    top: Top,
    x_off: f32,
    ui: &Ui,
    p: Pos2,
) -> Option<usize> {
    let in_margin = f.in_fold_margin(p);
    if !in_margin && !f.content.contains(p) {
        return None;
    }
    let rows = display_map(editor, view);
    let down = ((p.y - f.content.min.y) / f.line_h + top.frac).floor();
    let (row, moved) = rows.step(top.row, down as isize);
    if moved != down as isize {
        return None; // below the last row
    }
    let v = editor.view(view)?;
    let b = editor.buffer(v.buffer())?;
    let folded = v.folds().is_folded(row.line);
    if in_margin {
        let header = folded
            || ochrepad_core::fold::is_header(
                b.doc(),
                row.line,
                b.fold_strategy,
                editor.settings.tab_width,
            );
        return (row.sub == 0 && header).then_some(row.line);
    }
    if !folded || rows.row_range(row).end != b.doc().line_end(row.line) {
        return None;
    }
    let layout = line_layout(editor, v.buffer(), cache, row.line, f, ui, Styling::Any);
    let y = f.content.min.y + (down - top.frac) * f.line_h;
    let x_end = f.text_left() - x_off - row_x0(&rows, &layout, row, f.cell_w) + layout.width;
    f.fold_pill(x_end, y).contains(p).then_some(row.line)
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
/// Longest selection, in bytes, that smart highlighting searches for.
const SMART_MAX_LEN: usize = 1024;

/// The searcher for smart highlighting, as in Notepad++: when the primary
/// selection is non-blank text on one line, every exact (case-sensitive)
/// occurrence of it is highlighted. The searcher is rebuilt only when the
/// selected text changes.
fn smart_searcher<'s>(
    cache: &'s mut Option<(String, Option<Searcher>)>,
    doc: &Document,
    sels: &SelectionSet,
) -> Option<&'s Searcher> {
    let sel = sels.primary();
    let range = sel.range();
    if sel.is_empty()
        || range.end.0 - range.start.0 > SMART_MAX_LEN
        || doc.line_of(range.start) != doc.line_of(range.end)
    {
        *cache = None;
        return None;
    }
    let text = doc.slice(range);
    if text.trim().is_empty() {
        *cache = None;
        return None;
    }
    if cache.as_ref().is_none_or(|(t, _)| *t != text) {
        let query = SearchQuery::new(
            text.clone(),
            SearchOptions {
                match_case: true,
                ..SearchOptions::default()
            },
        );
        *cache = Some((text, Searcher::new(&query).ok()));
    }
    cache.as_ref().and_then(|(_, s)| s.as_ref())
}

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

/// The display row under `p`: rows below the top row, clamped to the
/// document.
fn row_at(rows: &DisplayMap, f: &Frame, top: Top, p: Pos2) -> RowPos {
    let down = ((p.y - f.content.min.y) / f.line_h + top.frac).floor();
    rows.advance(top.row, down as isize)
}

/// The x (in the line's layout) of the start of `row`, less its
/// continuation indent: a wrapped row draws its slice of the line's layout
/// shifted left by this much.
fn row_x0(rows: &DisplayMap, layout: &LineLayout, row: RowPos, cell_w: f32) -> f32 {
    let line_start = rows.doc().line_start(row.line).0;
    layout.x_of(rows.row_range(row).start.0 - line_start) - rows.row_indent(row) as f32 * cell_w
}

/// Line and visual column under `p` (columns may lie past the line end).
#[allow(clippy::too_many_arguments)]
fn line_col_at(
    editor: &Editor,
    view: ViewId,
    cache: &mut LayoutCache,
    f: &Frame,
    top: Top,
    x_off: f32,
    ui: &Ui,
    p: Pos2,
) -> (usize, ochrepad_core::VisualCol) {
    let rows = display_map(editor, view);
    let row = row_at(&rows, f, top, p);
    let buffer = editor.view(view).expect("view exists").buffer();
    let layout = line_layout(editor, buffer, cache, row.line, f, ui, Styling::Any);
    let x = p.x - f.text_left() + x_off + row_x0(&rows, &layout, row, f.cell_w);
    (row.line, visual_col_at_x(x, f.cell_w))
}

fn line_layout(
    editor: &Editor,
    buffer: ochrepad_core::BufferId,
    cache: &mut LayoutCache,
    line: usize,
    f: &Frame,
    ui: &Ui,
    styling: Styling<'_>,
) -> std::sync::Arc<LineLayout> {
    let b = editor.buffer(buffer).expect("view's buffer exists");
    cache.get(
        b.doc(),
        (buffer, b.revision()),
        line,
        f.cell_w,
        editor.settings.tab_width,
        ui.visuals().text_color(),
        styling,
    )
}

/// Pixel -> byte offset, through the same `LineLayout` painting uses.
#[allow(clippy::too_many_arguments)]
fn hit_test(
    editor: &Editor,
    view: ViewId,
    cache: &mut LayoutCache,
    f: &Frame,
    top: Top,
    x_off: f32,
    ui: &Ui,
    p: Pos2,
) -> ByteOffset {
    let rows = display_map(editor, view);
    let row = row_at(&rows, f, top, p);
    let buffer = editor.view(view).expect("view exists").buffer();
    let layout = line_layout(editor, buffer, cache, row.line, f, ui, Styling::Any);
    let x = p.x - f.text_left() + x_off + row_x0(&rows, &layout, row, f.cell_w);
    let range = rows.row_range(row);
    let line_start = rows.doc().line_start(row.line).0;
    // A click past a wrapped row's end stays on that row.
    ByteOffset(line_start + layout.byte_at_x(x)).clamp(range.start, rows.row_end(row))
}

/// The vertical and horizontal scrollbar thumbs. `offset` and `max_offset`
/// are the scroll position and its maximum in visible lines.
fn thumbs(
    f: &Frame,
    offset: f64,
    max_offset: f64,
    x_off: f32,
    h_max: f32,
    max_width: f32,
) -> (Rect, Rect) {
    let total = (f.visible_lines as f32).max(1.0);
    let vh = (f.vbar.height() * (f.rows_full as f32 / total).min(1.0))
        .max(20.0)
        .min(f.vbar.height());
    let vt = if max_offset > 0.0 {
        (offset / max_offset) as f32
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
    top: Top,
    max_offset: f64,
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
        top,
        max_offset,
        x_off,
        h_max,
    }: Scroll,
    focused: bool,
    now: f64,
    show_chevrons: bool,
    styler: Option<&mut dyn LineStyler>,
    find_highlight: Option<&ochrepad_core::Searcher>,
) {
    let Some(v) = editor.view(view) else { return };
    let doc = doc_of(editor, buffer);
    let rows = display_map(editor, view);
    let sels = v.selections.ranges();
    let visuals = ui.visuals().clone();
    let painter = ui.painter().clone();
    let text_painter = painter.with_clip_rect(f.content.intersect(painter.clip_rect()));
    let gutter_painter = painter.with_clip_rect(f.gutter.intersect(painter.clip_rect()));

    painter.rect_filled(f.content, 0.0, visuals.extreme_bg_color);
    painter.rect_filled(f.gutter, 0.0, visuals.extreme_bg_color);
    painter.rect_filled(f.vbar.union(f.hbar), 0.0, visuals.extreme_bg_color);

    // The rows on screen, plus one partly shown below.
    let shown: Vec<RowPos> = rows.rows_from(top.row, f.rows_full + 2).collect();
    let y0 = f.content.min.y - top.frac * f.line_h;
    // The document lines the shown rows come from, as runs of consecutive
    // lines: a fold between two runs may hide any amount of text, which is
    // neither highlighted nor searched.
    let first = shown.first().map_or(0, |r| r.line);
    let last = shown.last().map_or(0, |r| r.line + 1);
    let mut runs: Vec<std::ops::Range<usize>> = Vec::new();
    for r in &shown {
        match runs.last_mut() {
            Some(run) if r.line <= run.end => run.end = r.line + 1,
            _ => runs.push(r.line..r.line + 1),
        }
    }
    let x0 = f.text_left() - x_off;
    let text_color = visuals.text_color();
    let current_line_bg = visuals.faint_bg_color;
    let sel_bg = visuals.selection.bg_fill;
    let spans: Vec<(usize, Vec<Option<ochrepad_syntax::LineSpans>>)> = match styler {
        Some(s) => runs
            .iter()
            .map(|r| (r.start, s.line_spans(doc, r.clone())))
            .collect(),
        None => Vec::new(),
    };
    let find_in_runs = |s: &Searcher| -> Vec<std::ops::Range<ByteOffset>> {
        runs.iter()
            .flat_map(|r| s.find_all(doc, doc.line_start(r.start)..doc.line_end(r.end - 1)))
            .collect()
    };
    // Every match of the find panel's pattern on the visible lines.
    let matches = find_highlight.map(find_in_runs).unwrap_or_default();
    let match_bg = crate::theme::match_bg(&visuals);
    // Smart highlighting: other occurrences of the selected text.
    let smart = smart_searcher(&mut state.smart, doc, &v.selections)
        .map(find_in_runs)
        .unwrap_or_default();
    let smart_bg = crate::theme::smart_highlight_bg(&visuals);
    let accent = crate::theme::accent(&visuals);
    let tab_width = editor.settings.tab_width;
    let strategy = editor
        .buffer(buffer)
        .map(|b| b.fold_strategy)
        .unwrap_or_default();

    for (i, &row) in shown.iter().enumerate() {
        let line = row.line;
        let y = y0 + i as f32 * f.line_h;
        let run = spans.partition_point(|(start, _)| *start <= line);
        let styling = Styling::Exact(
            run.checked_sub(1)
                .and_then(|r| spans[r].1.get(line - spans[r].0))
                .and_then(Option::as_ref),
        );
        let layout = line_layout(editor, buffer, &mut state.cache, line, f, ui, styling);
        state.max_width = state.max_width.max(layout.width);
        let line_start = doc.line_start(line);
        let line_end = doc.line_end(line);
        let range = rows.row_range(row);
        let (row_start, row_end) = (range.start, range.end);
        // Screen x of the line's text on this row, and of a byte in it.
        let rx0 = x0 - row_x0(&rows, &layout, row, f.cell_w);
        let x_at = |b: ByteOffset| rx0 + layout.x_of(b.0 - line_start.0);
        let rect = Rect::from_min_size(
            Pos2::new(f.content.min.x, y),
            Vec2::new(f.content.width(), f.line_h),
        );

        // Selections overlapping this row (they are sorted).
        let from = sels.partition_point(|s| s.end() < row_start);
        let on_row = sels[from..].iter().take_while(|s| s.start() <= row_end);
        // The current line: every row of a line holding a caret.
        let from = sels.partition_point(|s| s.end() < line_start);
        let has_caret = sels[from..]
            .iter()
            .take_while(|s| s.start() <= line_end)
            .any(|s| s.head >= line_start && s.head <= line_end);
        if has_caret {
            text_painter.rect_filled(rect, 0.0, current_line_bg);
        }
        let from = matches.partition_point(|m| m.end <= row_start && m.start < row_start);
        for m in matches[from..].iter().take_while(|m| m.start <= row_end) {
            if m.start == m.end {
                continue;
            }
            let a = m.start.max(row_start);
            let b = m.end.min(row_end);
            let r = Rect::from_min_max(
                Pos2::new(x_at(a), y),
                Pos2::new(x_at(b).max(x_at(a) + 2.0), y + f.line_h),
            );
            text_painter.rect_filled(r, 2.0, match_bg);
        }
        let from = smart.partition_point(|m| m.end <= row_start && m.start < row_start);
        for m in smart[from..].iter().take_while(|m| m.start <= row_end) {
            // The selection itself is already marked, and find matches keep
            // their own colour (an ochre tint over teal turns muddy).
            let under_find = matches[matches.partition_point(|f| f.end <= m.start)..]
                .first()
                .is_some_and(|f| f.start < m.end);
            if under_find || on_row.clone().any(|s| s.range() == *m) {
                continue;
            }
            let a = m.start.max(row_start);
            let b = m.end.min(row_end);
            let r = Rect::from_min_max(Pos2::new(x_at(a), y), Pos2::new(x_at(b), y + f.line_h));
            text_painter.rect_filled(r, 2.0, smart_bg);
        }
        for s in on_row {
            if s.is_empty() {
                continue;
            }
            let a = s.start().max(row_start);
            let b = s.end().min(row_end);
            let mut x1 = x_at(b);
            if s.end() > line_end && row_end == line_end {
                x1 += f.cell_w; // the selected newline
            }
            let r = Rect::from_min_max(Pos2::new(x_at(a), y), Pos2::new(x1, y + f.line_h));
            text_painter.rect_filled(r, 0.0, sel_bg);
        }

        // A wrapped row draws only its own slice of the line.
        let row_painter = if rows.wraps() {
            let right = if rows.is_last_in_line(row) {
                f.content.max.x
            } else {
                x_at(row_end)
            };
            let slice = Rect::from_min_max(
                Pos2::new(x_at(row_start), y),
                Pos2::new(right, y + f.line_h),
            );
            text_painter.with_clip_rect(slice.intersect(text_painter.clip_rect()))
        } else {
            text_painter.clone()
        };
        paint_line_text(&row_painter, &layout, rx0, y, &f.font, f.line_h);

        let folded = v.folds().is_folded(line);
        if folded && row_end == line_end {
            paint_fold_pill(&text_painter, f, &visuals, rx0 + layout.width, y);
        }

        // The line number and fold chevron go on a line's first row.
        if row.sub == 0 {
            gutter_painter.text(
                Pos2::new(f.gutter.max.x - f.fold_w, y),
                Align2::RIGHT_TOP,
                (line + 1).to_string(),
                f.font.clone(),
                if has_caret {
                    accent
                } else {
                    visuals.weak_text_color()
                },
            );
            if f.fold_margin
                && (folded
                    || show_chevrons
                        && ochrepad_core::fold::is_header(doc, line, strategy, tab_width))
            {
                paint_chevron(&gutter_painter, f, &visuals, y, folded);
            }
        }
    }

    // Carets and IME pre-edit. A caret off screen is placed one row above or
    // below the shown rows (the IME still needs a position).
    let caret_pos = |head: ByteOffset, cache: &mut LayoutCache| -> (Pos2, bool) {
        let row = rows.row_of(head);
        let (i, on_screen) = match shown.binary_search(&row) {
            Ok(i) => (i as f32, true),
            Err(_) if row < top.row => (-1.0, false),
            Err(_) => (shown.len() as f32, false),
        };
        let line = row.line;
        let layout = line_layout(editor, buffer, cache, line, f, ui, Styling::Any);
        let x = x0 - row_x0(&rows, &layout, row, f.cell_w)
            + layout.x_of(head.0 - doc.line_start(line).0);
        (Pos2::new(x, y0 + i * f.line_h), on_screen)
    };
    if focused {
        let since = now - state.last_activity;
        let on = ((since / BLINK_HALF_PERIOD) as u64).is_multiple_of(2);
        if on {
            let caret_stroke = Stroke::new(2.0, visuals.text_cursor.stroke.color);
            for s in sels {
                let (p, on_screen) = caret_pos(s.head, &mut state.cache);
                if on_screen {
                    text_painter.line_segment([p, p + Vec2::new(0.0, f.line_h)], caret_stroke);
                }
            }
        }
        let to_toggle = BLINK_HALF_PERIOD - since.rem_euclid(BLINK_HALF_PERIOD);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(to_toggle));

        let (p, _) = caret_pos(v.selections.primary().head, &mut state.cache);
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
    let (v_thumb, h_thumb) = thumbs(
        f,
        top.offset(&rows),
        max_offset,
        x_off,
        h_max,
        state.max_width,
    );
    let thumb_color = visuals.widgets.hovered.bg_fill;
    painter.rect_filled(v_thumb.shrink(2.0), 3.0, thumb_color);
    if h_max > 0.0 {
        painter.rect_filled(h_thumb.shrink(2.0), 3.0, thumb_color);
    }
    state.cache.retain_near(first, last.max(first + 1));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn smart_matches(text: &str, anchor: usize, head: usize) -> Vec<(usize, usize)> {
        let doc = Document::new(text);
        let sels = SelectionSet::new(
            vec![Selection::new(ByteOffset(anchor), ByteOffset(head))],
            0,
        );
        let mut cache = None;
        smart_searcher(&mut cache, &doc, &sels)
            .map(|s| s.find_all(&doc, ByteOffset(0)..ByteOffset(text.len())))
            .unwrap_or_default()
            .into_iter()
            .map(|m| (m.start.0, m.end.0))
            .collect()
    }

    #[test]
    fn smart_highlight_finds_every_exact_occurrence() {
        // Any substring, not just whole words, and case-sensitive.
        assert_eq!(
            smart_matches("cat Cat concat cat", 0, 3),
            vec![(0, 3), (11, 14), (15, 18)]
        );
        // A backward selection works the same; regex characters are literal.
        assert_eq!(smart_matches("a.b axb a.b", 3, 0), vec![(0, 3), (8, 11)]);
    }

    #[test]
    fn smart_highlight_ignores_carets_blanks_and_multi_line_selections() {
        assert!(smart_matches("cat cat", 2, 2).is_empty());
        assert!(smart_matches("a  b  c", 1, 3).is_empty());
        assert!(smart_matches("cat\ncat\n", 0, 4).is_empty());
    }
}
