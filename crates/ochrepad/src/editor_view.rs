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

use crate::doc_map::{MAP_W, MapView};
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

/// What View > Show Symbol draws over the text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Symbols {
    /// Spaces as dots and tabs as arrows.
    pub space_tab: bool,
    /// Each line's ending, named, after its text.
    pub eol: bool,
    /// Format characters and unusual spaces as labelled boxes (control
    /// characters always are).
    pub non_printing: bool,
    /// Dotted lines at each indent level inside the indentation.
    pub indent_guides: bool,
    /// A mark at the end of each row that a wrapped line continues from.
    pub wrap: bool,
}

/// How the app wants every editor drawn: the (zoomed) font size and the
/// View menu's symbols.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub font_size: f32,
    pub symbols: Symbols,
    /// A vertical line after this column.
    pub edge_column: Option<usize>,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            font_size: 14.0,
            symbols: Symbols::default(),
            edge_column: None,
        }
    }
}

pub struct EditorViewState {
    cache: LayoutCache,
    pub font_size: f32,
    pub symbols: Symbols,
    pub edge_column: Option<usize>,
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
    /// Show the document map beside the text.
    pub document_map: bool,
    /// The last error from an action a key ran (shown by the app).
    pub last_error: Option<ochrepad_core::ActionError>,
    map_cache: crate::doc_map::MapCache,
    /// Dragging the map's viewport marker: where it was grabbed, from its top.
    map_drag: Option<f32>,
    /// The line the last right-click was on (the context menu acts on it).
    pub context_line: Option<usize>,
    /// The file the context menu offers to open, once it has looked.
    pub context_file: Option<Option<crate::editor_menu::FileAt>>,
    /// Paste to (Replace) Bookmarked Lines asked for the clipboard at this
    /// time: the paste that answers goes to the bookmarked lines.
    paste_to_bookmarks: Option<f64>,
}

impl Default for EditorViewState {
    fn default() -> Self {
        Self {
            fold_margin: true,
            word_wrap: false,
            document_map: true,
            last_error: None,
            map_cache: Default::default(),
            map_drag: None,
            context_line: None,
            context_file: None,
            paste_to_bookmarks: None,
            cache: LayoutCache::default(),
            font_size: 14.0,
            symbols: Symbols::default(),
            edge_column: None,
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
    /// Takes the app's font size and symbols.
    pub fn set_look(&mut self, look: &Look) {
        self.font_size = look.font_size;
        self.symbols = look.symbols;
        self.edge_column = look.edge_column;
    }

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

    /// The next paste (asked for now, at `now`) replaces the text of each
    /// bookmarked line instead of going to the carets.
    pub fn paste_to_bookmarks(&mut self, now: f64) {
        self.paste_to_bookmarks = Some(now);
        self.request_focus = true;
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
    /// The document map strip, when shown.
    map: Option<Rect>,
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

/// The tab width of `buffer` (each document has its own).
fn tab_width_of(editor: &Editor, buffer: ochrepad_core::BufferId) -> usize {
    editor.buffer(buffer).map_or(4, |b| b.indent.width)
}

/// The display rows of `view`.
fn display_map(editor: &Editor, view: ViewId) -> DisplayMap<'_> {
    let v = editor.view(view).expect("view exists");
    let b = editor.buffer(v.buffer()).expect("view's buffer exists");
    DisplayMap::new(b, v)
}

enum Op {
    Apply(Action),
    Copy,
    Cut,
    ToggleOvertype,
}

/// Per-row colours from outside the editor: a compare view's diff, or a
/// log's highlight rules.
pub trait RowStyle {
    /// The background of `row` (`blank` for a padding row).
    fn background(&self, row: RowPos, blank: bool) -> Option<egui::Color32>;
    /// Byte ranges (within the line) to tint on `line`, each with its colour.
    fn spans(&self, line: usize) -> Vec<(std::ops::Range<usize>, egui::Color32)>;
    /// How opaque `line`'s text is drawn (below 1 dims it).
    fn text_opacity(&self, _line: usize) -> f32 {
        1.0
    }
}

/// Optional extras drawn over the text.
#[derive(Default)]
pub struct Decorations<'a> {
    /// Syntax highlighting.
    pub styler: Option<&'a mut dyn LineStyler>,
    /// Highlights every match of the find panel's pattern.
    pub find_highlight: Option<&'a ochrepad_core::Searcher>,
    /// Row backgrounds and spans (the compare view).
    pub row_style: Option<&'a dyn RowStyle>,
    /// The lines changed since the file was saved.
    pub changes: Option<&'a crate::change_history::LineMarks>,
    /// Search > Style All Occurrences of Token: what to highlight, in which
    /// colour.
    pub token_styles: &'a [(&'a Searcher, egui::Color32)],
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
    let show_map = state.document_map;
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
        let mut content = Rect::from_min_max(Pos2::new(gutter.max.x, body.min.y), body.max);
        // The map takes the right of the text area while 40 columns of text
        // still fit beside it.
        let map = (show_map && content.width() - MAP_W >= 40.0 * cell_w).then(|| {
            content.max.x -= MAP_W;
            Rect::from_min_max(Pos2::new(content.max.x, content.min.y), body.max)
        });
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
            map,
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
                    Some(EditorCommand::ToggleOvertype) => ops.push(Op::ToggleOvertype),
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
                    // Asked for by Paste to (Replace) Bookmarked Lines just
                    // now (an empty clipboard sends nothing, so a later
                    // paste must not count).
                    let to_marks = state
                        .paste_to_bookmarks
                        .take()
                        .is_some_and(|asked| now - asked < 2.0);
                    ops.push(Op::Apply(if to_marks {
                        Action::ReplaceBookmarkedLines(t)
                    } else if block {
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
                if let Err(e) = editor.apply(view, a) {
                    state.last_error = Some(e);
                }
            }
            Op::Copy | Op::Cut => {
                if copy_selection(ui.ctx(), editor, view) && matches!(op, Op::Cut) {
                    let _ = editor.apply(view, Action::InsertText(String::new()));
                }
            }
            Op::ToggleOvertype => editor.settings.overtype = !editor.settings.overtype,
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
        // Ctrl+wheel zooms (egui gives no scroll delta while Ctrl is held).
        let zoom = ui.input(|i| wheel_zoom_steps(&i.events));
        if zoom != 0.0 {
            ui.ctx()
                .data_mut(|d| *d.get_temp_mut_or_default::<f32>(zoom_id()) += zoom);
        }
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

    // --- Document map --------------------------------------------------------------
    if !down {
        state.map_drag = None;
    }
    if let Some(strip) = f.map {
        let rows = display_map(editor, view);
        let map = MapView::new(
            &rows,
            strip,
            top.row,
            top.offset(&rows),
            max_offset,
            f.rows_full,
        );
        if pressed
            && state.bar_drag.is_none()
            && let Some(p) = pointer
            && response.contains_pointer()
            && strip.contains(p)
        {
            let marker = map.viewport_rect(strip);
            if marker.contains(p) {
                state.map_drag = Some(p.y - marker.min.y);
            } else if let Some(row) = map.row_at(strip, p.y) {
                // Centre the editor on the clicked row; keep dragging from
                // the middle of the marker.
                let half = f.rows_full as isize / 2;
                top = Top {
                    row: rows.advance(row, -half),
                    frac: 0.0,
                }
                .clamped(&rows, f.rows_full);
                state.map_drag = Some(marker.height() / 2.0);
            }
        } else if let (Some(grab), Some(p)) = (state.map_drag, pointer) {
            let t = ((p.y - grab - strip.min.y) / map.track).clamp(0.0, 1.0);
            top = Top::at_offset(&rows, t as f64 * max_offset).clamped(&rows, f.rows_full);
        }
    }

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

    // --- Right click: the context menu acts where it was opened -----------------
    // As in Scintilla: a click outside the selection moves the caret there
    // (on a line number, to the line's start); on the selection it stays.
    let secondary = ui.input(|i| i.pointer.secondary_pressed());
    if secondary && response.contains_pointer() {
        // On a scrollbar or the map: the menu has no line to act on.
        state.context_line = None;
        state.context_file = None;
    }
    if secondary
        && state.bar_drag.is_none()
        && let Some(p) = pointer
        && response.contains_pointer()
        && (f.content.contains(p) || f.gutter.contains(p))
    {
        let hit = hit_test(editor, view, &mut state.cache, &f, top, x_off, ui, p);
        let doc = doc_of(editor, buffer_id);
        let line = doc.line_of(hit);
        let in_gutter = f.gutter.contains(p);
        let on_selection = editor.view(view).is_some_and(|v| {
            v.selections.ranges().iter().any(|s| {
                !s.is_empty()
                    && if in_gutter {
                        doc.line_of(s.start()) <= line && line <= doc.line_of(s.end())
                    } else {
                        s.start() <= hit && hit <= s.end()
                    }
            })
        });
        if !on_selection {
            let at = if in_gutter { doc.line_start(line) } else { hit };
            let _ = editor.apply(view, Action::SetSelections(SelectionSet::caret(at)));
        }
        state.context_line = Some(line);
        state.last_activity = now;
        response.request_focus();
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
    // The name of a special character under the pointer, while it is drawn.
    let tip = hover.filter(|p| f.content.contains(*p)).and_then(|p| {
        let (layout, x) = layout_at(editor, view, &mut state.cache, &f, top, x_off, ui, p);
        let cell = layout
            .specials
            .iter()
            .find(|s| s.x <= x && x < s.x + s.width)?;
        let s = crate::invisible::special(&layout.text[cell.range.clone()])?;
        (s.control || state.symbols.non_printing).then(|| crate::invisible::describe(&s))
    });

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
        decorations.row_style,
        decorations.changes,
        decorations.token_styles,
    );
    match tip {
        Some(tip) => response.on_hover_text_at_pointer(tip),
        None => response,
    }
}

/// Where Ctrl+wheel zoom steps wait for the app (see [`take_zoom_steps`]).
pub(crate) fn zoom_id() -> Id {
    Id::new("ochrepad-zoom-steps")
}

/// Zoom steps in this frame's Ctrl+wheel events: one per notch of a mouse
/// wheel, or per 50 points of a trackpad; positive zooms in.
pub(crate) fn wheel_zoom_steps(events: &[Event]) -> f32 {
    events
        .iter()
        .map(|e| match e {
            Event::MouseWheel {
                unit,
                delta,
                modifiers,
                ..
            } if modifiers.command => match unit {
                egui::MouseWheelUnit::Line => delta.y,
                egui::MouseWheelUnit::Point => delta.y / 50.0,
                egui::MouseWheelUnit::Page => delta.y * 3.0,
            },
            _ => 0.0,
        })
        .sum()
}

/// The whole zoom steps Ctrl+wheel over an editor asked for since the last
/// call; a trackpad's fraction of a step is kept for next time.
pub fn take_zoom_steps(ctx: &egui::Context) -> i32 {
    ctx.data_mut(|d| {
        let acc = d.get_temp_mut_or_default::<f32>(zoom_id());
        let whole = acc.trunc();
        *acc -= whole;
        whole as i32
    })
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

/// The change history's bar for `line` on the row at `y`, at the gutter's
/// right edge, and on its first row a wedge if lines were deleted above it.
fn paint_change_marks(
    painter: &egui::Painter,
    f: &Frame,
    marks: &crate::change_history::LineMarks,
    line: usize,
    first_row: bool,
    y: f32,
) {
    use crate::change_history::Mark;
    use ochrepad_core::diff::RowKind;
    if let Some(mark) = marks.mark(line) {
        let kind = match mark {
            Mark::Added => RowKind::Added,
            Mark::Changed => RowKind::Changed,
        };
        let (r, g, b) = crate::theme::diff_rgb(kind);
        let bar = Rect::from_min_max(
            Pos2::new(f.gutter.max.x - 4.0, y),
            Pos2::new(f.gutter.max.x - 1.0, y + f.line_h),
        );
        painter.rect_filled(bar, 0.0, egui::Color32::from_rgb(r, g, b));
    }
    if first_row && marks.deleted_before(line) {
        paint_deleted_wedge(painter, f, y);
    }
}

/// A red wedge pointing at the text, on the boundary at `y` where lines
/// were deleted.
fn paint_deleted_wedge(painter: &egui::Painter, f: &Frame, y: f32) {
    let (r, g, b) = crate::theme::diff_rgb(ochrepad_core::diff::RowKind::Removed);
    let s = (f.line_h * 0.22).round().max(3.0);
    let tip = Pos2::new(f.gutter.max.x - 1.0, y);
    painter.add(egui::Shape::convex_polygon(
        vec![
            tip + Vec2::new(-s * 1.4, -s),
            tip,
            tip + Vec2::new(-s * 1.4, s),
        ],
        egui::Color32::from_rgb(r, g, b),
        Stroke::NONE,
    ));
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
            || ochrepad_core::fold::is_header(b.doc(), row.line, b.fold_strategy, b.indent.width);
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
        let mut px = x0 + piece.x;
        if px > clip.max.x {
            break;
        }
        if px + piece.width < clip.min.x {
            continue;
        }
        let mut range = piece.range.clone();
        if !piece.clip {
            // An ASCII run: one byte per cell. Lay out only the part on
            // screen; a long line would otherwise produce more vertices
            // than the GPU buffer holds.
            let cell = piece.width / range.len() as f32;
            if px < clip.min.x {
                let skip = (((clip.min.x - px) / cell).floor() as usize).min(range.len());
                range.start += skip;
                px += skip as f32 * cell;
            }
            let keep = ((clip.max.x - px) / cell).ceil().max(0.0) as usize + 1;
            range.end = range.end.min(range.start + keep);
        }
        let galley =
            painter.layout_no_wrap(layout.text[range].to_owned(), font.clone(), piece.color);
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

/// Colours and sizes for Show Symbol's marks.
struct MarkStyle {
    /// Whitespace dots and arrows, line endings, the wrap mark.
    symbol: egui::Color32,
    /// Indent guides and the vertical edge.
    guide: egui::Color32,
    special_bg: egui::Color32,
    special_text: egui::Color32,
    /// The line-ending pills' text.
    small: FontId,
}

impl MarkStyle {
    fn new(visuals: &egui::Visuals, f: &Frame) -> Self {
        let weak = visuals.weak_text_color();
        let warn = visuals.warn_fg_color;
        let alpha = if visuals.dark_mode { 80 } else { 70 };
        Self {
            symbol: weak.gamma_multiply(0.8),
            guide: weak.gamma_multiply(0.5),
            special_bg: egui::Color32::from_rgba_unmultiplied(warn.r(), warn.g(), warn.b(), alpha),
            special_text: visuals.text_color(),
            small: FontId::proportional(f.font.size * 0.62),
        }
    }
}

/// Show Symbol's marks over a row's text: dots for spaces and arrows for
/// tabs, and the boxes of special characters (control characters always,
/// the others with Show Non-Printing Characters). `x0` is the screen x of
/// the line's first cell.
fn paint_marks(
    painter: &egui::Painter,
    layout: &LineLayout,
    x0: f32,
    y: f32,
    f: &Frame,
    symbols: Symbols,
    m: &MarkStyle,
) {
    let clip = painter.clip_rect();
    let mid = y + f.line_h / 2.0;
    if symbols.space_tab {
        let bytes = layout.text.as_bytes();
        let dot = (f.cell_w * 0.09).max(1.0);
        for (at, x, next) in layout.graphemes_between(clip.min.x - x0, clip.max.x - x0) {
            match bytes[at] {
                b' ' => {
                    painter.circle_filled(Pos2::new(x0 + (x + next) / 2.0, mid), dot, m.symbol);
                }
                b'\t' => paint_tab_arrow(painter, x0 + x, x0 + next, mid, f.cell_w, m.symbol),
                _ => {}
            }
        }
    }
    for cell in &layout.specials {
        let left = x0 + cell.x;
        if left > clip.max.x || left + cell.width < clip.min.x {
            continue;
        }
        let Some(s) = crate::invisible::special(&layout.text[cell.range.clone()]) else {
            continue;
        };
        if s.control || symbols.non_printing {
            let r = Rect::from_min_size(
                Pos2::new(left + 0.5, y + 1.5),
                Vec2::new(cell.width - 1.0, f.line_h - 3.0),
            );
            paint_special(painter, r, s.abbrev, f.font.size, m);
        }
    }
}

/// A tab's arrow across its cells.
fn paint_tab_arrow(
    painter: &egui::Painter,
    start: f32,
    end: f32,
    y: f32,
    cell_w: f32,
    color: egui::Color32,
) {
    let pad = cell_w * 0.2;
    let (a, b) = (start + pad, end - pad);
    if b <= a {
        return;
    }
    let stroke = Stroke::new(1.0, color);
    let head = (cell_w * 0.3).min(b - a);
    painter.line_segment([Pos2::new(a, y), Pos2::new(b, y)], stroke);
    painter.line(
        vec![
            Pos2::new(b - head, y - head),
            Pos2::new(b, y),
            Pos2::new(b - head, y + head),
        ],
        stroke,
    );
}

/// Smallest text drawn in a special character's box, in points; below it
/// the box stays empty and only its tooltip names the character.
const MIN_LABEL_SIZE: f32 = 7.0;

/// A special character's box, with its abbreviation in one row or two when
/// that is still legible (a box is only as wide as the character's cells).
fn paint_special(painter: &egui::Painter, r: Rect, abbrev: &str, font_size: f32, m: &MarkStyle) {
    painter.rect_filled(r, 2.0, m.special_bg);
    let layout = |text: &str, size: f32| {
        painter.layout_no_wrap(text.to_owned(), FontId::proportional(size), m.special_text)
    };
    let avail = r.width() - 1.0;
    let max = font_size * 0.62;
    let one = layout(abbrev, max);
    let size = max * (avail / one.size().x).min(1.0);
    if size >= max * 0.8 || (size >= MIN_LABEL_SIZE && abbrev.len() <= 2) {
        let g = if size < max {
            layout(abbrev, size)
        } else {
            one
        };
        painter.galley(r.center() - g.size() / 2.0, g, m.special_text);
        return;
    }
    let (upper, lower) = abbrev.split_at(abbrev.len().div_ceil(2));
    let wide = layout(upper, max).size().x.max(layout(lower, max).size().x);
    let size = (max * (avail / wide).min(1.0)).min(r.height() / 2.4);
    if size < MIN_LABEL_SIZE {
        return;
    }
    let (gu, gl) = (layout(upper, size), layout(lower, size));
    let top = r.center().y - (gu.size().y + gl.size().y) / 2.0;
    let (cx, h) = (r.center().x, gu.size().y);
    painter.galley(Pos2::new(cx - gu.size().x / 2.0, top), gu, m.special_text);
    painter.galley(
        Pos2::new(cx - gl.size().x / 2.0, top + h),
        gl,
        m.special_text,
    );
}

/// The line ending's name in a pill after the line's text.
fn paint_eol(painter: &egui::Painter, x_end: f32, y: f32, f: &Frame, label: &str, m: &MarkStyle) {
    let g = painter.layout_no_wrap(label.to_owned(), m.small.clone(), m.symbol);
    let h = (f.line_h - 6.0).max(g.size().y);
    let r = Rect::from_min_size(
        Pos2::new(x_end + 2.0, y + (f.line_h - h) / 2.0),
        Vec2::new(g.size().x + 6.0, h),
    );
    painter.rect_filled(r, 3.0, m.symbol.gamma_multiply(0.18));
    painter.galley(r.center() - g.size() / 2.0, g, m.symbol);
}

/// The hooked arrow in the free cell after a row that its line continues
/// from (word wrap).
fn paint_wrap_mark(painter: &egui::Painter, x: f32, y: f32, f: &Frame, color: egui::Color32) {
    let s = f.cell_w * 0.3;
    let (cx, cy) = (x + f.cell_w * 0.5, y + f.line_h * 0.5);
    let stroke = Stroke::new(1.0, color);
    let tip = Pos2::new(cx - s, cy + s * 0.5);
    painter.line(
        vec![
            Pos2::new(cx + s, cy - s),
            Pos2::new(cx + s, cy + s * 0.5),
            tip,
        ],
        stroke,
    );
    painter.line(
        vec![
            tip + Vec2::new(s * 0.6, -s * 0.6),
            tip,
            tip + Vec2::new(s * 0.6, s * 0.6),
        ],
        stroke,
    );
}

/// The width in columns of `line`'s leading whitespace, or `None` when the
/// line is blank (empty or only whitespace).
fn leading_cols(doc: &Document, line: usize, tab_width: usize) -> Option<usize> {
    let end = doc.line_end(line);
    let mut at = doc.line_start(line);
    let mut col = 0;
    while at < end {
        match doc.char_at(at)? {
            ' ' => col += 1,
            '\t' => col += tab_width - col % tab_width,
            _ => return Some(col),
        }
        at = ByteOffset(at.0 + 1);
    }
    None
}

/// The indentation that indent guides follow on `line`: its own, or for a
/// blank line the deeper of the nearest non-blank lines above and below
/// (looking at most 100 lines each way), so guides run through blank lines
/// inside a block.
fn guide_indent(doc: &Document, line: usize, tab_width: usize) -> usize {
    if let Some(cols) = leading_cols(doc, line, tab_width) {
        return cols;
    }
    let above = (line.saturating_sub(100)..line)
        .rev()
        .find_map(|l| leading_cols(doc, l, tab_width));
    let below =
        (line + 1..doc.line_count().min(line + 101)).find_map(|l| leading_cols(doc, l, tab_width));
    above.unwrap_or(0).max(below.unwrap_or(0))
}

/// Dotted indent guides on one row, at every indent level inside the
/// line's indentation from the second level on (the first would sit on the
/// text's left edge), as Notepad++ draws them. `x0` is the screen x of
/// column 0.
#[allow(clippy::too_many_arguments)]
fn paint_indent_guides(
    painter: &egui::Painter,
    doc: &Document,
    rows: &DisplayMap,
    row: RowPos,
    x0: f32,
    y: f32,
    f: &Frame,
    width: usize,
    color: egui::Color32,
) {
    let width = width.max(1);
    let mut limit = guide_indent(doc, row.line, width);
    if rows.text_sub(row) != Some(0) {
        // A wrapped line's later rows are indented at most this much.
        limit = limit.min(rows.row_indent(row));
    }
    let clip = painter.clip_rect();
    let stroke = Stroke::new(1.0, color);
    let mut col = width;
    while col < limit {
        let x = (x0 + col as f32 * f.cell_w).round() + 0.5;
        if x > clip.max.x {
            break;
        }
        if x >= clip.min.x {
            painter.extend(egui::Shape::dashed_line(
                &[Pos2::new(x, y), Pos2::new(x, y + f.line_h)],
                stroke,
                1.0,
                2.0,
            ));
        }
        col += width;
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

/// Puts `text` on the clipboard as ordinary text (not a rectangle to paste
/// as a block).
pub fn copy_text(ctx: &egui::Context, text: String) {
    ctx.data_mut(|d| d.remove::<String>(rect_clip_id()));
    remember_copy(ctx, &text, false);
    ctx.copy_text(text);
}

/// Where the texts copied in ochrepad are remembered, newest first, with
/// whether each was a rectangle (Edit > Paste from History).
fn clip_history_id() -> Id {
    Id::new("ochrepad-clip-history")
}

/// How many copies Paste from History offers.
const MAX_CLIP_HISTORY: usize = 10;
/// Larger copies are not remembered.
const MAX_CLIP_REMEMBERED: usize = 1 << 20;

fn remember_copy(ctx: &egui::Context, text: &str, rect: bool) {
    if text.is_empty() || text.len() > MAX_CLIP_REMEMBERED {
        return;
    }
    ctx.data_mut(|d| {
        let history = d.get_temp_mut_or_default::<Vec<(String, bool)>>(clip_history_id());
        history.retain(|(t, _)| t != text);
        history.insert(0, (text.to_owned(), rect));
        history.truncate(MAX_CLIP_HISTORY);
    });
}

/// The texts copied in ochrepad, newest first, and whether each was a
/// rectangle.
pub fn clip_history(ctx: &egui::Context) -> Vec<(String, bool)> {
    ctx.data(|d| d.get_temp(clip_history_id()))
        .unwrap_or_default()
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
    remember_copy(ctx, &text, rect);
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
        tab_width_of(editor, buffer),
        ui.visuals().text_color(),
        styling,
    )
}

/// The layout of the line under `p`, and `p`'s x in that layout.
#[allow(clippy::too_many_arguments)]
fn layout_at(
    editor: &Editor,
    view: ViewId,
    cache: &mut LayoutCache,
    f: &Frame,
    top: Top,
    x_off: f32,
    ui: &Ui,
    p: Pos2,
) -> (std::sync::Arc<LineLayout>, f32) {
    let rows = display_map(editor, view);
    let row = row_at(&rows, f, top, p);
    let buffer = editor.view(view).expect("view exists").buffer();
    let layout = line_layout(editor, buffer, cache, row.line, f, ui, Styling::Any);
    let x = p.x - f.text_left() + x_off + row_x0(&rows, &layout, row, f.cell_w);
    (layout, x)
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
    row_style: Option<&dyn RowStyle>,
    changes: Option<&crate::change_history::LineMarks>,
    token_styles: &[(&Searcher, egui::Color32)],
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
    let mut styler = styler;
    let spans: Vec<(usize, Vec<Option<ochrepad_syntax::LineSpans>>)> = match styler.as_deref_mut() {
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
    // Styled tokens on the visible lines, with their colours, by start.
    let mut styled: Vec<(std::ops::Range<ByteOffset>, egui::Color32)> = token_styles
        .iter()
        .flat_map(|(s, color)| find_in_runs(s).into_iter().map(move |m| (m, *color)))
        .collect();
    styled.sort_by_key(|(m, _)| m.start);
    let match_bg = crate::theme::match_bg(&visuals);
    // Smart highlighting: other occurrences of the selected text.
    let smart = smart_searcher(&mut state.smart, doc, &v.selections)
        .map(find_in_runs)
        .unwrap_or_default();
    let smart_bg = crate::theme::smart_highlight_bg(&visuals);
    let accent = crate::theme::accent(&visuals);
    // The bracket beside the primary caret and its match.
    let primary = v.selections.primary();
    let bracket = primary
        .is_empty()
        .then(|| ochrepad_core::brackets::bracket_at(doc, primary.head))
        .flatten();
    let tab_width = tab_width_of(editor, buffer);
    let strategy = editor
        .buffer(buffer)
        .map(|b| b.fold_strategy)
        .unwrap_or_default();
    let symbols = state.symbols;
    let marks = MarkStyle::new(&visuals, f);
    let eol_label = editor
        .buffer(buffer)
        .map_or("LF", |b| match b.meta.line_ending {
            ochrepad_core::LineEnding::CrLf => "CRLF",
            ochrepad_core::LineEnding::Lf => "LF",
            ochrepad_core::LineEnding::Cr => "CR",
        });
    let last_line = doc.line_count().saturating_sub(1);
    let bookmarks = editor.buffer(buffer).map(|b| b.bookmarks());

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
        let blank = rows.is_filler(row);
        let row_bg = row_style.and_then(|s| s.background(row, blank));
        if blank {
            // A padding row: no text, no number.
            if let Some(bg) = row_bg {
                text_painter.rect_filled(rect, 0.0, bg);
            }
            continue;
        }

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
        // Over the current line's background, so a diff or log colour still
        // shows on the caret's line.
        if let Some(bg) = row_bg {
            text_painter.rect_filled(rect, 0.0, bg);
        }
        // Tokens are short (one line at most), so those reaching this row
        // start at most a line before it.
        let from = styled.partition_point(|(m, _)| m.start < line_start);
        for (m, color) in styled[from..]
            .iter()
            .take_while(|(m, _)| m.start <= row_end)
        {
            if m.start >= m.end || m.end < row_start {
                continue;
            }
            let a = m.start.max(row_start);
            let b = m.end.min(row_end);
            let r = Rect::from_min_max(Pos2::new(x_at(a), y), Pos2::new(x_at(b), y + f.line_h));
            text_painter.rect_filled(r, 2.0, *color);
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

        if let Some(style) = row_style {
            for (sp, color) in style.spans(line) {
                let a = ByteOffset(line_start.0 + sp.start).max(row_start);
                let b = ByteOffset(line_start.0 + sp.end).min(row_end);
                if a < b {
                    let r =
                        Rect::from_min_max(Pos2::new(x_at(a), y), Pos2::new(x_at(b), y + f.line_h));
                    text_painter.rect_filled(r, 2.0, color);
                }
            }
        }

        if symbols.indent_guides {
            paint_indent_guides(
                &text_painter,
                doc,
                &rows,
                row,
                x0,
                y,
                f,
                tab_width,
                marks.guide,
            );
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
        let mut row_painter = row_painter;
        if let Some(o) = row_style.map(|s| s.text_opacity(line)).filter(|o| *o < 1.0) {
            row_painter.multiply_opacity(o);
        }
        paint_line_text(&row_painter, &layout, rx0, y, &f.font, f.line_h);
        paint_marks(&row_painter, &layout, rx0, y, f, symbols, &marks);
        if let Some(p) = bracket {
            let color = if p.matching.is_some() {
                accent
            } else {
                visuals.error_fg_color
            };
            for b in std::iter::once(p.at).chain(p.matching) {
                if b >= row_start && b < row_end {
                    let r = Rect::from_min_max(
                        Pos2::new(x_at(b), y + 1.0),
                        Pos2::new(x_at(ByteOffset(b.0 + 1)), y + f.line_h - 1.0),
                    );
                    text_painter.rect_stroke(
                        r,
                        2.0,
                        Stroke::new(1.0, color),
                        egui::StrokeKind::Inside,
                    );
                }
            }
        }

        let folded = v.folds().is_folded(line);
        if folded && row_end == line_end {
            paint_fold_pill(&text_painter, f, &visuals, rx0 + layout.width, y);
        }
        if symbols.eol && row_end == line_end && !folded && line < last_line {
            paint_eol(&text_painter, rx0 + layout.width, y, f, eol_label, &marks);
        }
        if symbols.wrap && rows.wraps() && !rows.is_last_in_line(row) {
            paint_wrap_mark(&text_painter, x_at(row_end), y, f, marks.symbol);
        }

        // Change history: a bar beside every row of a changed line, and a
        // wedge where lines were deleted.
        if let Some(c) = changes {
            paint_change_marks(
                &gutter_painter,
                f,
                c,
                line,
                rows.text_sub(row) == Some(0),
                y,
            );
            if line == last_line && rows.is_last_in_line(row) && c.deleted_before(line + 1) {
                paint_deleted_wedge(&gutter_painter, f, y + f.line_h);
            }
        }

        // The line number and fold chevron go on a line's first row. A
        // bookmarked line's number sits on a tag in the accent colour.
        if rows.text_sub(row) == Some(0) {
            let marked = bookmarks.is_some_and(|m| m.contains(line));
            if marked {
                let tag = Rect::from_min_max(
                    Pos2::new(f.gutter.min.x + 2.0, y + 1.0),
                    Pos2::new(
                        f.gutter.max.x - f.fold_w + (f.cell_w * 0.4).round(),
                        y + f.line_h - 1.0,
                    ),
                );
                gutter_painter.rect_filled(tag, 3.0, crate::theme::bookmark_bg(&visuals));
            }
            gutter_painter.text(
                Pos2::new(f.gutter.max.x - f.fold_w, y),
                Align2::RIGHT_TOP,
                (line + 1).to_string(),
                f.font.clone(),
                if marked {
                    crate::theme::bookmark_fg(&visuals)
                } else if has_caret {
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

    // The vertical edge, after its column.
    if let Some(col) = state.edge_column {
        let x = (x0 + col as f32 * f.cell_w).round() + 0.5;
        if x > f.content.min.x {
            text_painter.vline(x, f.content.y_range(), Stroke::new(1.0, marks.guide));
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
            let caret_color = visuals.text_cursor.stroke.color;
            for s in sels {
                let (p, on_screen) = caret_pos(s.head, &mut state.cache);
                if !on_screen {
                    continue;
                }
                if editor.settings.overtype {
                    // A bar under the character that typing replaces (a
                    // cell wide at the line's end), as in Scintilla.
                    let line_end = doc.line_end(doc.line_of(s.head));
                    let next = ochrepad_core::position::next_grapheme_boundary(doc, s.head);
                    let w = if next <= line_end && next > s.head {
                        let (q, _) = caret_pos(next, &mut state.cache);
                        if q.y == p.y { q.x - p.x } else { f.cell_w }
                    } else {
                        f.cell_w
                    };
                    let bar = Rect::from_min_size(
                        Pos2::new(p.x, p.y + f.line_h - 2.0),
                        Vec2::new(w.max(2.0), 2.0),
                    );
                    text_painter.rect_filled(bar, 0.0, caret_color);
                } else {
                    text_painter.line_segment(
                        [p, p + Vec2::new(0.0, f.line_h)],
                        Stroke::new(2.0, caret_color),
                    );
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

    // The document map.
    if let Some(strip) = f.map {
        painter.rect_filled(strip, 0.0, visuals.extreme_bg_color);
        let map = MapView::new(
            &rows,
            strip,
            top.row,
            top.offset(&rows),
            max_offset,
            f.rows_full,
        );
        // Highlighting for the map's lines, asked for per run of lines.
        let mut map_spans: std::collections::HashMap<usize, ochrepad_syntax::LineSpans> =
            Default::default();
        if let Some(s) = styler {
            let mut i = 0;
            while i < map.rows.len() {
                let start = map.rows[i].line;
                let mut end = start + 1;
                while i < map.rows.len() && map.rows[i].line <= end {
                    end = map.rows[i].line + 1;
                    i += 1;
                }
                for (line, sp) in (start..end).zip(s.line_spans(doc, start..end)) {
                    if let Some(sp) = sp {
                        map_spans.insert(line, sp);
                    }
                }
            }
        }
        let hovered = ui
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|p| strip.contains(p));
        crate::doc_map::paint(
            &painter,
            strip,
            &map,
            &rows,
            &mut state.map_cache,
            buffer,
            &|line| map_spans.get(&line).cloned(),
            &visuals,
            tab_width_of(editor, buffer),
            hovered || state.map_drag.is_some(),
        );
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
    let caret_offset = rows.offset_of(rows.row_of(v.selections.primary().head), 0.0);
    paint_overview(
        &painter,
        f,
        &rows,
        editor.buffer(buffer).map(|b| b.bookmarks()),
        changes,
        caret_offset,
        &visuals,
    );
    state.cache.retain_near(first, last.max(first + 1));
}

/// The overview on the vertical scrollbar: the bookmarks (right) and the
/// changed lines (left) at their places in the document, and the caret's
/// line across it. Worked out per pixel row, so a document with a million
/// bookmarks costs no more than one with ten.
fn paint_overview(
    painter: &egui::Painter,
    f: &Frame,
    rows: &DisplayMap,
    bookmarks: Option<&ochrepad_core::Bookmarks>,
    changes: Option<&crate::change_history::LineMarks>,
    caret_offset: f64,
    visuals: &egui::Visuals,
) {
    let bar = f.vbar;
    let visible = rows.visible_line_count();
    // A short document maps to the part of the bar its lines fill.
    let total = visible.max(f.rows_full) as f64;
    let height = bar.height().floor();
    if height < 1.0 || visible == 0 {
        return;
    }
    let y_of = |offset: f64| bar.min.y + (offset / total) as f32 * height;
    let caret_y = y_of(caret_offset).round();
    let caret = Rect::from_min_max(
        Pos2::new(bar.min.x + 1.0, caret_y),
        Pos2::new(bar.max.x - 1.0, caret_y + 2.0),
    );
    painter.rect_filled(caret, 0.0, visuals.text_color().gamma_multiply(0.55));

    let marks = bookmarks.filter(|b| !b.is_empty());
    let changed = changes.filter(|c| !c.is_empty());
    if marks.is_none() && changed.is_none() {
        return;
    }
    let doc_lines = rows.doc().line_count();
    // The document lines pixel row `y` covers, hidden ones included.
    let lines_at = |y: usize| -> Option<std::ops::Range<usize>> {
        let i0 = (y as f64 / height as f64 * total) as usize;
        if i0 >= visible {
            return None;
        }
        let i1 = (((y + 1) as f64 / height as f64 * total) as usize).max(i0 + 1);
        let start = rows.visible_line_at(i0);
        let end = if i1 >= visible {
            doc_lines + 1
        } else {
            rows.visible_line_at(i1)
        };
        Some(start..end.max(start + 1))
    };
    let any_in = |sorted: &[usize], r: &std::ops::Range<usize>| {
        let i = sorted.partition_point(|&l| l < r.start);
        sorted.get(i).is_some_and(|&l| l < r.end)
    };
    let accent = crate::theme::bookmark_bg(visuals);
    let lane = |y: f32, left: bool| {
        let (x0, x1) = if left {
            (bar.min.x + 1.0, bar.min.x + 4.0)
        } else {
            (bar.min.x + 6.0, bar.max.x - 1.0)
        };
        Rect::from_min_max(Pos2::new(x0, y), Pos2::new(x1, y + 2.0))
    };
    for y in 0..height as usize {
        let Some(r) = lines_at(y) else {
            break;
        };
        let top = bar.min.y + y as f32;
        if marks.is_some_and(|m| any_in(m.lines(), &r)) {
            painter.rect_filled(lane(top, false), 0.0, accent);
        }
        if let Some(c) = changed {
            let i = c.lines.partition_point(|m| m.0 < r.start);
            let here = c.lines[i..].iter().take_while(|m| m.0 < r.end);
            // A changed line shows over added ones, and both over deletions.
            let mut kind = None;
            for (_, mark) in here {
                match mark {
                    crate::change_history::Mark::Changed => {
                        kind = Some(ochrepad_core::diff::RowKind::Changed);
                        break;
                    }
                    crate::change_history::Mark::Added => {
                        kind = Some(ochrepad_core::diff::RowKind::Added);
                    }
                }
            }
            if kind.is_none() && any_in(&c.deleted_before, &r) {
                kind = Some(ochrepad_core::diff::RowKind::Removed);
            }
            if let Some(kind) = kind {
                let (red, green, blue) = crate::theme::diff_rgb(kind);
                painter.rect_filled(
                    lane(top, true),
                    0.0,
                    egui::Color32::from_rgb(red, green, blue),
                );
            }
        }
    }
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
