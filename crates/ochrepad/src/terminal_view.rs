//! The terminal widget: draws a [`Terminal`]'s screen on the monospace grid
//! and sends it keys, pasted text and mouse reports. The grid follows the
//! tab's size; the shell starts on the first frame, when that size is known.

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{TermMode, point_to_viewport, viewport_to_point};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use egui::text::{LayoutJob, TextFormat};
use egui::{
    Color32, Event, EventFilter, FontId, Id, ImeEvent, Key, Modifiers, PointerButton, Pos2, Rect,
    Response, Sense, Stroke, Ui, Vec2, WidgetInfo, WidgetType,
};

use crate::terminal::{Ended, GridSize, Terminal, TerminalId};
use crate::terminal_keys::{self, MouseAction, MouseButton};

const SCROLLBAR_W: f32 = 12.0;
/// Space around the grid.
const PAD: f32 = 4.0;
/// Two clicks closer than this (seconds) on one cell make a double click.
const MULTI_CLICK: f64 = 0.4;

/// The widget's state between frames.
#[derive(Default)]
pub struct TerminalViewState {
    request_focus: bool,
    was_focused: bool,
    /// Wheel movement not yet a whole line.
    scroll_rest: f32,
    /// A selection is being dragged.
    selecting: bool,
    /// The button whose press was reported to the program (its release and
    /// drags are reported too).
    reporting: Option<MouseButton>,
    /// The last cell a drag report was sent for.
    reported_cell: Option<(usize, usize)>,
    /// Time, cell and count of the last primary press (double and triple
    /// clicks).
    last_click: Option<(f64, Point, u8)>,
    /// Dragging the scrollbar's thumb: where it was grabbed.
    bar_drag: Option<f32>,
    preedit: Option<String>,
}

impl TerminalViewState {
    pub fn request_focus(&mut self) {
        self.request_focus = true;
    }
}

pub fn terminal_widget_id(id: TerminalId) -> Id {
    Id::new(("ochrepad-terminal", id.0))
}

/// The colours a terminal is drawn with.
struct Colors {
    palette: [Color32; 16],
    fg: Color32,
    bg: Color32,
    bright_fg: Color32,
    cursor: Color32,
    selection: Color32,
}

impl Colors {
    fn new(visuals: &egui::Visuals) -> Self {
        Self {
            palette: crate::theme::terminal_palette(visuals.dark_mode),
            fg: visuals.text_color(),
            bg: visuals.extreme_bg_color,
            bright_fg: visuals.strong_text_color(),
            cursor: crate::theme::accent(visuals),
            selection: visuals.selection.bg_fill,
        }
    }

    /// One of the 256 indexed colours: the palette, the 6×6×6 cube, the
    /// grey ramp.
    fn indexed(&self, i: u8) -> Color32 {
        match i {
            0..=15 => self.palette[usize::from(i)],
            16..=231 => {
                let i = i - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                Color32::from_rgb(level(i / 36), level(i / 6 % 6), level(i % 6))
            }
            _ => {
                let v = 8 + (i - 232) * 10;
                Color32::from_rgb(v, v, v)
            }
        }
    }

    /// The colour at index `i` of the terminal's colour table (what a colour
    /// query answers).
    fn at(&self, i: usize) -> Color32 {
        match i {
            0..=255 => self.indexed(i as u8),
            i if i == NamedColor::Foreground as usize => self.fg,
            i if i == NamedColor::Background as usize => self.bg,
            i if i == NamedColor::Cursor as usize => self.cursor,
            i if i == NamedColor::BrightForeground as usize => self.bright_fg,
            i if i == NamedColor::DimForeground as usize => dim(self.fg, self.bg),
            // DimBlack..=DimWhite.
            i if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize).contains(&i) => {
                dim(self.palette[i - NamedColor::DimBlack as usize], self.bg)
            }
            _ => self.fg,
        }
    }

    fn rgb(&self, i: usize) -> Rgb {
        let c = self.at(i);
        Rgb {
            r: c.r(),
            g: c.g(),
            b: c.b(),
        }
    }

    /// A cell's colour: the program's change to the colour table (OSC 4),
    /// else ours.
    fn resolve(&self, color: Color, table: &alacritty_terminal::term::color::Colors) -> Color32 {
        let from_rgb = |c: Rgb| Color32::from_rgb(c.r, c.g, c.b);
        match color {
            Color::Spec(rgb) => from_rgb(rgb),
            Color::Indexed(i) => table[usize::from(i)].map_or_else(|| self.indexed(i), from_rgb),
            Color::Named(n) => table[n].map_or_else(|| self.at(n as usize), from_rgb),
        }
    }
}

/// `c` moved a third of the way toward `bg`.
fn dim(c: Color32, bg: Color32) -> Color32 {
    let mix = |a: u8, b: u8| ((u16::from(a) * 2 + u16::from(b)) / 3) as u8;
    Color32::from_rgb(mix(c.r(), bg.r()), mix(c.g(), bg.g()), mix(c.b(), bg.b()))
}

/// Bold text in one of the 8 normal colours shows in its bright version, as
/// most terminals do.
fn brighten(color: Color) -> Color {
    match color {
        Color::Named(n) if (n as usize) < 8 => Color::Indexed(n as u8 + 8),
        Color::Indexed(i) if i < 8 => Color::Indexed(i + 8),
        c => c,
    }
}

/// The grid's place on screen.
struct Grid {
    origin: Pos2,
    cell_w: f32,
    line_h: f32,
    cols: usize,
    rows: usize,
}

impl Grid {
    fn rect(&self) -> Rect {
        Rect::from_min_size(
            self.origin,
            Vec2::new(
                self.cols as f32 * self.cell_w,
                self.rows as f32 * self.line_h,
            ),
        )
    }

    /// The screen cell (column, row) under `p`, clamped to the grid, and the
    /// half of the cell it is in.
    fn cell_at(&self, p: Pos2) -> (usize, usize, Side) {
        let x = (p.x - self.origin.x) / self.cell_w;
        let y = (p.y - self.origin.y) / self.line_h;
        let col = (x.max(0.0) as usize).min(self.cols - 1);
        let row = (y.max(0.0) as usize).min(self.rows - 1);
        let side = if x - x.floor() > 0.5 && x >= 0.0 {
            Side::Right
        } else {
            Side::Left
        };
        (col, row, side)
    }

    fn cell_pos(&self, col: usize, row: usize) -> Pos2 {
        self.origin + Vec2::new(col as f32 * self.cell_w, row as f32 * self.line_h)
    }
}

/// What the widget did that the app acts on.
pub struct TerminalOutput {
    pub response: Response,
    /// The program takes the mouse (a right click goes to it, not to a menu).
    pub mouse_reporting: bool,
}

/// Shows `t` filling the available space. `label` names it for
/// accessibility.
pub fn terminal_ui(ui: &mut Ui, t: &mut Terminal, font_size: f32, label: &str) -> TerminalOutput {
    let rect = ui.available_rect_before_wrap();
    ui.advance_cursor_after_rect(rect);
    let widget = terminal_widget_id(t.id);
    let response = ui.interact(rect, widget, Sense::click_and_drag());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::TextEdit, true, label));
    let colors = Colors::new(ui.visuals());
    t.poll(ui.ctx(), &|i| colors.rgb(i));

    // --- The grid follows the tab's size ---------------------------------------
    let font = FontId::monospace(font_size);
    let (cell_w, line_h) = ui.fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
    let bar = Rect::from_min_max(Pos2::new(rect.max.x - SCROLLBAR_W, rect.min.y), rect.max);
    let area = Rect::from_min_max(
        rect.min + Vec2::splat(PAD),
        Pos2::new(bar.min.x - PAD, rect.max.y - PAD),
    );
    let g = Grid {
        origin: area.min,
        cell_w,
        line_h,
        cols: ((area.width() / cell_w).floor() as usize).clamp(2, usize::from(u16::MAX)),
        rows: ((area.height() / line_h).floor() as usize).clamp(1, usize::from(u16::MAX)),
    };
    let size = GridSize {
        cols: g.cols as u16,
        rows: g.rows as u16,
        cell_w: cell_w.round() as u16,
        cell_h: line_h.round() as u16,
    };
    if t.started() {
        t.resize(size);
    } else {
        t.start(ui.ctx(), size);
    }

    // --- Focus ---------------------------------------------------------------------
    let pressed_here = ui.input(|i| i.pointer.any_pressed()) && response.contains_pointer();
    if t.view.request_focus || pressed_here {
        response.request_focus();
        t.view.request_focus = false;
    }
    let focused = response.has_focus();
    if focused {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                widget,
                EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            )
        });
    }
    let mode = t.mode();
    if focused != t.view.was_focused {
        t.view.was_focused = focused;
        t.term().lock().is_focused = focused;
        if mode.contains(TermMode::FOCUS_IN_OUT) {
            t.write(if focused { b"\x1b[I" } else { b"\x1b[O" }.to_vec());
        }
    }

    // --- Keyboard --------------------------------------------------------------------
    if focused {
        keyboard(ui, t, mode);
    }

    // --- Mouse -------------------------------------------------------------------------
    let modifiers = ui.input(|i| i.modifiers);
    let reporting = mode.intersects(TermMode::MOUSE_MODE) && !modifiers.shift;
    mouse(ui, t, &g, &response, bar, mode, reporting);

    // --- Painting ----------------------------------------------------------------------
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, colors.bg);
    paint_grid(ui, &painter, t, &g, &font, &colors, focused);
    paint_scrollbar(ui, &painter, t, bar, g.rows);
    if focused {
        let cursor = t.term().lock().grid().cursor.point;
        let pos = g.cell_pos(
            cursor.column.0.min(g.cols - 1),
            usize::try_from(cursor.line.0).unwrap_or(0).min(g.rows - 1),
        );
        ui.ctx().output_mut(|o| {
            o.ime = Some(egui::output::IMEOutput {
                purpose: egui::IMEPurpose::Normal,
                rect: g.rect(),
                cursor_rect: Rect::from_min_size(pos, Vec2::new(cell_w, line_h)),
                should_interrupt_composition: false,
            })
        });
    }
    TerminalOutput {
        response,
        mouse_reporting: reporting,
    }
}

/// What the terminal's right-click menu asks the app for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuCommand {
    NewTerminal,
    Close,
}

/// The right-click menu: Copy, Paste, Select All, Clear, then New Terminal
/// and Close (left to the app).
pub fn context_menu(ui: &mut Ui, t: &mut Terminal) -> Option<MenuCommand> {
    let shared = t.term().clone();
    let has_selection = shared
        .lock()
        .selection
        .as_ref()
        .is_some_and(|s| !s.is_empty());
    let item = |ui: &mut Ui, label: &str, shortcut: &str, enabled: bool| {
        ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(shortcut))
            .clicked()
    };
    if item(ui, "Copy", "Ctrl+Shift+C", has_selection) {
        let mut term = shared.lock();
        if let Some(text) = term.selection_to_string() {
            ui.ctx().copy_text(text);
        }
        term.selection = None;
    }
    if item(ui, "Paste", "Ctrl+V", t.running()) {
        // The clipboard comes back as a paste event, to the terminal (which
        // gets the keyboard back when the menu closes).
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
        t.view.request_focus();
    }
    if item(ui, "Select All", "", true) {
        let mut term = shared.lock();
        let mut all = Selection::new(
            SelectionType::Lines,
            Point::new(term.topmost_line(), Column(0)),
            Side::Left,
        );
        all.update(
            Point::new(term.bottommost_line(), term.last_column()),
            Side::Right,
        );
        term.selection = Some(all);
    }
    if ui
        .button("Clear")
        .on_hover_text("Clears the scrollback; the program redraws what it shows")
        .clicked()
    {
        let mut term = shared.lock();
        term.grid_mut().clear_history();
        term.selection = None;
        drop(term);
        // Ctrl+L: shells clear the screen and draw the prompt again.
        if t.running() && !t.mode().contains(TermMode::ALT_SCREEN) {
            t.write(vec![0x0c]);
        }
    }
    ui.separator();
    let mut chosen = None;
    if ui.button("New Terminal").clicked() {
        chosen = Some(MenuCommand::NewTerminal);
    }
    if ui.button("Close").clicked() {
        chosen = Some(MenuCommand::Close);
    }
    chosen
}

/// Keys, text, the clipboard and the IME, while the terminal has the
/// keyboard.
fn keyboard(ui: &Ui, t: &mut Terminal, mode: TermMode) {
    let mut input: Vec<Vec<u8>> = Vec::new();
    let modifiers = ui.input(|i| i.modifiers);
    for event in ui.input(|i| i.events.clone()) {
        match event {
            Event::Text(text) => {
                input.push(terminal_keys::text_bytes(
                    &text,
                    modifiers.alt && !modifiers.ctrl,
                ));
            }
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                // Enter after the shell ended starts a new one.
                if key == Key::Enter && matches!(t.ended, Some(Ended::Exited(_))) {
                    t.restart(ui.ctx());
                    continue;
                }
                // Shift+PageUp/PageDown/Home/End scroll the scrollback
                // (outside full-screen programs).
                if modifiers.shift_only() && !mode.contains(TermMode::ALT_SCREEN) {
                    let scroll = match key {
                        Key::PageUp => Some(Scroll::PageUp),
                        Key::PageDown => Some(Scroll::PageDown),
                        Key::Home => Some(Scroll::Top),
                        Key::End => Some(Scroll::Bottom),
                        _ => None,
                    };
                    if let Some(scroll) = scroll {
                        t.term().lock().scroll_display(scroll);
                        continue;
                    }
                }
                if let Some(bytes) = terminal_keys::key_bytes(key, modifiers, mode) {
                    input.push(bytes);
                }
            }
            // Ctrl+C: copy when there is a selection (and always with
            // Shift), else interrupt.
            Event::Copy => {
                let text = {
                    let mut term = t.term().lock();
                    let text = term.selection_to_string().filter(|s| !s.is_empty());
                    if text.is_some() {
                        term.selection = None;
                    }
                    text
                };
                match text {
                    Some(text) => ui.ctx().copy_text(text),
                    None if !modifiers.shift => input.push(vec![0x03]),
                    None => {}
                }
            }
            // Ctrl+X is a control character here; Shift+Delete (Windows'
            // other Cut) is a key.
            Event::Cut => input.push(if modifiers.ctrl {
                vec![0x18]
            } else {
                b"\x1b[3;2~".to_vec()
            }),
            Event::Paste(text) => input.push(terminal_keys::paste_bytes(&text, mode)),
            Event::Ime(ImeEvent::Preedit { text, .. }) => {
                t.view.preedit = (!text.is_empty()).then_some(text);
            }
            Event::Ime(ImeEvent::Commit(text)) => {
                t.view.preedit = None;
                if !text.is_empty() {
                    input.push(text.into_bytes());
                }
            }
            _ => {}
        }
    }
    if !input.is_empty() {
        // Typing goes back to the bottom and drops the selection, as in
        // most terminals.
        {
            let mut term = t.term().lock();
            term.scroll_display(Scroll::Bottom);
            term.selection = None;
        }
        for bytes in input {
            t.write(bytes);
        }
    }
}

/// Clicks, drags and the wheel: selecting and scrolling, or reports to the
/// program when it asks for the mouse.
fn mouse(
    ui: &Ui,
    t: &mut Terminal,
    g: &Grid,
    response: &Response,
    bar: Rect,
    mode: TermMode,
    reporting: bool,
) {
    let now = ui.input(|i| i.time);
    let grid_rect = g.rect().expand2(Vec2::new(0.0, 0.0));
    let in_grid = |p: Pos2| grid_rect.contains(p) || response.rect.contains(p) && !bar.contains(p);
    for event in ui.input(|i| i.events.clone()) {
        match event {
            Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers,
            } => {
                let button = match button {
                    PointerButton::Primary => MouseButton::Left,
                    PointerButton::Middle => MouseButton::Middle,
                    PointerButton::Secondary => MouseButton::Right,
                    _ => continue,
                };
                let (col, row, side) = g.cell_at(pos);
                if reporting {
                    if pressed && in_grid(pos) && response.contains_pointer() {
                        t.view.reporting = Some(button);
                        t.view.reported_cell = Some((col, row));
                        report(
                            t,
                            Some(button),
                            MouseAction::Press,
                            modifiers,
                            col,
                            row,
                            mode,
                        );
                    } else if !pressed && t.view.reporting == Some(button) {
                        t.view.reporting = None;
                        report(
                            t,
                            Some(button),
                            MouseAction::Release,
                            modifiers,
                            col,
                            row,
                            mode,
                        );
                    }
                    continue;
                }
                if button != MouseButton::Left {
                    continue;
                }
                if !pressed {
                    t.view.selecting = false;
                    continue;
                }
                if !response.contains_pointer() || !in_grid(pos) {
                    continue;
                }
                let shared = t.term().clone();
                let mut term = shared.lock();
                let point =
                    viewport_to_point(term.grid().display_offset(), Point::new(row, Column(col)));
                let count = match t.view.last_click {
                    Some((at, p, n)) if now - at < MULTI_CLICK && p == point => n % 3 + 1,
                    _ => 1,
                };
                t.view.last_click = Some((now, point, count));
                t.view.selecting = true;
                if modifiers.shift
                    && let Some(selection) = &mut term.selection
                {
                    selection.update(point, side);
                    continue;
                }
                let kind = match count {
                    2 => SelectionType::Semantic,
                    3 => SelectionType::Lines,
                    _ if modifiers.alt => SelectionType::Block,
                    _ => SelectionType::Simple,
                };
                term.selection = Some(Selection::new(kind, point, side));
            }
            Event::PointerMoved(pos) => {
                let (col, row, side) = g.cell_at(pos);
                if reporting {
                    let motion = mode.contains(TermMode::MOUSE_MOTION);
                    let drag = mode.contains(TermMode::MOUSE_DRAG) || motion;
                    let moved = t.view.reported_cell != Some((col, row));
                    let button = t.view.reporting;
                    if moved
                        && ((button.is_some() && drag)
                            || (button.is_none() && motion && response.contains_pointer()))
                    {
                        t.view.reported_cell = Some((col, row));
                        let mods = ui.input(|i| i.modifiers);
                        report(t, button, MouseAction::Drag, mods, col, row, mode);
                    }
                    continue;
                }
                if !t.view.selecting {
                    continue;
                }
                let mut term = t.term().lock();
                // Dragging above or below the grid scrolls.
                if pos.y < grid_rect.min.y {
                    term.scroll_display(Scroll::Delta(1));
                } else if pos.y > grid_rect.max.y {
                    term.scroll_display(Scroll::Delta(-1));
                }
                let point =
                    viewport_to_point(term.grid().display_offset(), Point::new(row, Column(col)));
                if let Some(selection) = &mut term.selection {
                    selection.update(point, side);
                }
            }
            _ => {}
        }
    }
    if !ui.input(|i| i.pointer.primary_down()) {
        t.view.selecting = false;
    }

    // The wheel: the scrollback, arrow keys in full-screen programs, or
    // reports. Ctrl+wheel zooms, as in the editor.
    if response.contains_pointer() {
        let zoom = ui.input(|i| crate::editor_view::wheel_zoom_steps(&i.events));
        if zoom != 0.0 {
            ui.ctx().data_mut(|d| {
                *d.get_temp_mut_or_default::<f32>(crate::editor_view::zoom_id()) += zoom
            });
        }
        let dy = ui.input(|i| i.smooth_scroll_delta.y);
        if dy != 0.0 && zoom == 0.0 {
            ui.ctx().input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
            t.view.scroll_rest += dy / g.line_h;
            let lines = t.view.scroll_rest.trunc() as i32;
            t.view.scroll_rest -= lines as f32;
            if lines != 0 {
                scroll(ui, t, g, mode, reporting, lines);
            }
        }
    }

    // The scrollbar's thumb.
    let (pointer, down, pressed) = ui.input(|i| {
        (
            i.pointer.interact_pos(),
            i.pointer.primary_down(),
            i.pointer.primary_pressed(),
        )
    });
    let (history, offset) = {
        let term = t.term().lock();
        (term.grid().history_size(), term.grid().display_offset())
    };
    if history == 0 {
        t.view.bar_drag = None;
        return;
    }
    let (thumb, track) = thumb_rect(bar, history, offset, g.rows);
    if pressed
        && let Some(p) = pointer
        && bar.contains(p)
        && response.contains_pointer()
    {
        let grab = if thumb.contains(p) {
            p.y - thumb.min.y
        } else {
            thumb.height() / 2.0
        };
        t.view.bar_drag = Some(grab);
    }
    if !down {
        t.view.bar_drag = None;
    }
    if let (Some(grab), Some(p)) = (t.view.bar_drag, pointer) {
        let along = ((p.y - grab - bar.min.y) / track.max(1.0)).clamp(0.0, 1.0);
        let wanted = ((1.0 - along) * history as f32).round() as i32;
        t.term()
            .lock()
            .scroll_display(Scroll::Delta(wanted - offset as i32));
    }
}

/// Scrolls `lines` (up when positive).
fn scroll(ui: &Ui, t: &mut Terminal, g: &Grid, mode: TermMode, reporting: bool, lines: i32) {
    if reporting {
        let pointer = ui.input(|i| i.pointer.hover_pos()).unwrap_or(g.origin);
        let (col, row, _) = g.cell_at(pointer);
        let button = if lines > 0 {
            MouseButton::WheelUp
        } else {
            MouseButton::WheelDown
        };
        let mods = ui.input(|i| i.modifiers);
        for _ in 0..lines.unsigned_abs().min(20) {
            report(t, Some(button), MouseAction::Press, mods, col, row, mode);
        }
    } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
        let key = if lines > 0 {
            Key::ArrowUp
        } else {
            Key::ArrowDown
        };
        if let Some(bytes) = terminal_keys::key_bytes(key, Modifiers::NONE, mode) {
            for _ in 0..lines.unsigned_abs().min(20) {
                t.write(bytes.clone());
            }
        }
    } else {
        t.term().lock().scroll_display(Scroll::Delta(lines));
    }
}

fn report(
    t: &Terminal,
    button: Option<MouseButton>,
    action: MouseAction,
    mods: Modifiers,
    col: usize,
    row: usize,
    mode: TermMode,
) {
    if let Some(bytes) = terminal_keys::mouse_report(button, action, mods, col, row, mode) {
        t.write(bytes);
    }
}

/// The scrollbar's thumb, and the height it moves along.
fn thumb_rect(bar: Rect, history: usize, offset: usize, rows: usize) -> (Rect, f32) {
    let total = (history + rows) as f32;
    let h = (bar.height() * rows as f32 / total)
        .max(20.0)
        .min(bar.height());
    let track = bar.height() - h;
    let along = 1.0 - offset as f32 / history.max(1) as f32;
    let r = Rect::from_min_size(
        Pos2::new(bar.min.x, bar.min.y + track * along),
        Vec2::new(bar.width(), h),
    );
    (r, track)
}

fn paint_scrollbar(ui: &Ui, painter: &egui::Painter, t: &Terminal, bar: Rect, rows: usize) {
    let (history, offset) = {
        let term = t.term().lock();
        (term.grid().history_size(), term.grid().display_offset())
    };
    if history == 0 {
        return;
    }
    let (thumb, _) = thumb_rect(bar, history, offset, rows);
    painter.rect_filled(thumb.shrink(2.0), 3.0, ui.visuals().widgets.hovered.bg_fill);
}

/// A run of cells drawn as one piece of text.
struct Run {
    row: usize,
    col: usize,
    text: String,
    color: Color32,
    italic: bool,
}

/// Draws the screen: backgrounds, text, decorations, the selection and the
/// cursor.
fn paint_grid(
    ui: &Ui,
    painter: &egui::Painter,
    t: &Terminal,
    g: &Grid,
    font: &FontId,
    colors: &Colors,
    focused: bool,
) {
    let term = t.term().lock();
    let content = term.renderable_content();
    let table = content.colors;
    let offset = content.display_offset;
    let cursor = content.cursor;
    let selection = content.selection;
    let mut runs: Vec<Run> = Vec::new();
    let mut singles: Vec<(usize, usize, usize, String, Color32, bool)> = Vec::new();
    let mut lines: Vec<(Pos2, f32, Color32, bool)> = Vec::new();
    // The character under a block cursor, drawn again over it.
    let mut under_cursor: Option<(String, usize)> = None;
    // Backgrounds as (row, from column, to column, colour), merged along
    // rows; the selection as (row, column, width), drawn over them.
    let mut fills: Vec<(usize, usize, usize, Color32)> = Vec::new();
    let mut selected_cells: Vec<(usize, usize, usize)> = Vec::new();
    for cell in content.display_iter {
        let flags = cell.flags;
        if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            continue;
        }
        let Some(view) = point_to_viewport(offset, cell.point) else {
            continue;
        };
        let (col, row) = (view.column.0, view.line);
        if row >= g.rows || col >= g.cols {
            continue;
        }
        let width = if flags.contains(Flags::WIDE_CHAR) {
            2
        } else {
            1
        };
        let fg_color = if flags.contains(Flags::BOLD) {
            brighten(cell.fg)
        } else {
            cell.fg
        };
        let mut fg = colors.resolve(fg_color, table);
        let mut bg = colors.resolve(cell.bg, table);
        if flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if flags.contains(Flags::DIM) {
            fg = dim(fg, bg);
        }
        if flags.contains(Flags::HIDDEN) {
            fg = bg;
        }
        let selected =
            selection.is_some_and(|s| s.contains_cell(&cell, cursor.point, cursor.shape));

        if bg != colors.bg {
            match fills.last_mut() {
                Some((r, _, to, c)) if *r == row && *to == col && *c == bg => *to += width,
                _ => fills.push((row, col, col + width, bg)),
            }
        }
        if selected {
            selected_cells.push((row, col, width));
        }

        // Underlines and strikethrough.
        let y_under = g.cell_pos(col, row).y + g.line_h - 1.5;
        if flags.intersects(Flags::ALL_UNDERLINES) {
            let color = cell
                .underline_color()
                .map_or(fg, |c| colors.resolve(c, table));
            let x = g.cell_pos(col, row).x;
            lines.push((Pos2::new(x, y_under), width as f32 * g.cell_w, color, false));
            if flags.contains(Flags::DOUBLE_UNDERLINE) {
                lines.push((
                    Pos2::new(x, y_under - 2.0),
                    width as f32 * g.cell_w,
                    color,
                    false,
                ));
            }
        }
        if flags.contains(Flags::STRIKEOUT) {
            let p = g.cell_pos(col, row) + Vec2::new(0.0, g.line_h / 2.0);
            lines.push((p, width as f32 * g.cell_w, fg, true));
        }

        let c = cell.c;
        let is_cursor = cursor.point == cell.point && cursor.shape == CursorShape::Block;
        if is_cursor && focused && c != ' ' && c != '\t' && fg != bg {
            let mut s = c.to_string();
            if let Some(extra) = cell.zerowidth() {
                s.extend(extra);
            }
            under_cursor = Some((s, width));
        }
        if c == ' ' || c == '\t' || c == '\0' || fg == bg {
            continue;
        }
        let italic = flags.contains(Flags::ITALIC);
        if c.is_ascii_graphic() && cell.zerowidth().is_none() {
            match runs.last_mut() {
                Some(run)
                    if run.row == row
                        && run.col + run.text.chars().count() == col
                        && run.color == fg
                        && run.italic == italic =>
                {
                    run.text.push(c);
                }
                _ => runs.push(Run {
                    row,
                    col,
                    text: c.to_string(),
                    color: fg,
                    italic,
                }),
            }
        } else {
            let mut s = c.to_string();
            if let Some(extra) = cell.zerowidth() {
                s.extend(extra);
            }
            singles.push((row, col, width, s, fg, italic));
        }
    }
    let span = |row: usize, col: usize, width: usize| {
        Rect::from_min_size(
            g.cell_pos(col, row),
            Vec2::new(width as f32 * g.cell_w, g.line_h),
        )
    };
    for (row, from, to, color) in fills {
        painter.rect_filled(span(row, from, to - from), 0.0, color);
    }
    for (row, col, width) in selected_cells {
        painter.rect_filled(span(row, col, width), 0.0, colors.selection);
    }

    // Runs of printable ASCII sit on the grid by themselves (the font is
    // monospace); other characters are placed one per cell, clipped to it.
    for run in runs {
        let pos = g.cell_pos(run.col, run.row);
        let galley = layout(ui, run.text, font, run.color, run.italic);
        painter.galley(pos, galley, run.color);
    }
    for (row, col, width, s, color, italic) in singles {
        let cell = Rect::from_min_size(
            g.cell_pos(col, row),
            Vec2::new(width as f32 * g.cell_w, g.line_h),
        );
        // Box drawing and block elements fill their cells as shapes.
        let mut chars = s.chars();
        if let (Some(c), None) = (chars.next(), chars.next())
            && crate::terminal_glyphs::draw(painter, cell, c, color)
        {
            continue;
        }
        let galley = layout(ui, s, font, color, italic);
        let x = cell.min.x + ((cell.width() - galley.size().x) / 2.0).max(0.0);
        painter
            .with_clip_rect(cell.intersect(painter.clip_rect()))
            .galley(Pos2::new(x, cell.min.y), galley, color);
    }
    for (p, w, color, _) in lines {
        painter.line_segment([p, p + Vec2::new(w, 0.0)], Stroke::new(1.0, color));
    }

    // The cursor (only while the screen shows the bottom of the scrollback).
    if let Some(view) = point_to_viewport(offset, cursor.point)
        && view.line < g.rows
        && view.column.0 < g.cols
    {
        let pos = g.cell_pos(view.column.0, view.line);
        let wide = term.grid()[cursor.point].flags.contains(Flags::WIDE_CHAR);
        let w = if wide { 2.0 } else { 1.0 } * g.cell_w;
        let r = Rect::from_min_size(pos, Vec2::new(w, g.line_h));
        match cursor.shape {
            CursorShape::Hidden => {}
            CursorShape::Block if focused => {
                painter.rect_filled(r, 0.0, colors.cursor);
                if let Some((s, _)) = under_cursor {
                    let galley = layout(ui, s, font, colors.bg, false);
                    painter.galley(pos, galley, colors.bg);
                }
            }
            CursorShape::Beam => {
                painter.rect_filled(
                    Rect::from_min_size(pos, Vec2::new(2.0, g.line_h)),
                    0.0,
                    colors.cursor,
                );
            }
            CursorShape::Underline => {
                painter.rect_filled(
                    Rect::from_min_size(
                        Pos2::new(pos.x, pos.y + g.line_h - 2.0),
                        Vec2::new(w, 2.0),
                    ),
                    0.0,
                    colors.cursor,
                );
            }
            // Unfocused, or asked for: an outline.
            _ => {
                painter.rect_stroke(
                    r.shrink(0.5),
                    0.0,
                    Stroke::new(1.0, colors.cursor),
                    egui::StrokeKind::Inside,
                );
            }
        }
        if let Some(pre) = &t.view.preedit {
            let galley = layout(ui, pre.clone(), font, colors.fg, false);
            let r = Rect::from_min_size(pos, Vec2::new(galley.size().x, g.line_h));
            painter.rect_filled(r, 0.0, colors.bg);
            painter.galley(pos, galley, colors.fg);
            painter.line_segment(
                [r.left_bottom(), r.right_bottom()],
                Stroke::new(1.0, colors.fg),
            );
        }
    }
}

fn layout(
    ui: &Ui,
    text: String,
    font: &FontId,
    color: Color32,
    italic: bool,
) -> std::sync::Arc<egui::Galley> {
    if !italic {
        return ui.fonts_mut(|f| f.layout_no_wrap(text, font.clone(), color));
    }
    let mut job = LayoutJob::default();
    job.append(
        &text,
        0.0,
        TextFormat {
            font_id: font.clone(),
            color,
            italics: true,
            ..Default::default()
        },
    );
    ui.fonts_mut(|f| f.layout_job(job))
}
