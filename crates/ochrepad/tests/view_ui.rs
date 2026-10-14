//! 0.3.0 P3: zoom and View > Show Symbol.

mod util;

use egui::{Event, FontId, Key, Modifiers, MouseWheelUnit, Pos2, TouchPhase, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;

fn harness(size: Vec2) -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(size)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .wgpu()
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.run();
    h
}

fn key(h: &mut Harness<'_, OchrepadApp>, m: Modifiers, k: Key) {
    h.key_press_modifiers(m, k);
    h.run();
}

fn wheel(h: &mut Harness<'_, OchrepadApp>, notches: f32, modifiers: Modifiers) {
    h.event(Event::MouseWheel {
        unit: MouseWheelUnit::Line,
        delta: Vec2::new(0.0, notches),
        phase: TouchPhase::Move,
        modifiers,
    });
    h.run();
}

/// Screen position of (line, column) in the editor at 100% zoom, at the
/// cell's center.
fn cell_pos(h: &Harness<'_, OchrepadApp>, line: usize, col: f32) -> Pos2 {
    let rect = h.get_by_label("Editor: new 1").rect();
    let font = FontId::monospace(14.0);
    let (cell_w, line_h) = h
        .ctx
        .fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
    let x = rect.min.x + 5.0 * cell_w + 8.0 + (col + 0.5) * cell_w;
    Pos2::new(x, rect.min.y + (line as f32 + 0.5) * line_h)
}

#[test]
fn keys_and_ctrl_wheel_zoom_the_editors() {
    let mut h = harness(Vec2::new(700.0, 300.0));
    h.event(Event::Paste("some text\n".into()));
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::Equals);
    assert_eq!(h.state().settings.zoom, 110);
    // The status bar shows it, and a click there resets it.
    h.get_by_label("110%");
    key(&mut h, Modifiers::COMMAND, Key::Minus);
    key(&mut h, Modifiers::COMMAND, Key::Minus);
    assert_eq!(h.state().settings.zoom, 90);
    h.get_by_label("90%").click();
    h.run();
    assert_eq!(h.state().settings.zoom, 100);
    assert!(h.query_by_label("100%").is_none());

    // Ctrl+wheel over an editor: a step per notch, and no scrolling.
    h.get_by_label("Editor: new 1").hover();
    h.run();
    wheel(&mut h, 2.0, Modifiers::COMMAND);
    assert_eq!(h.state().settings.zoom, 125);
    wheel(&mut h, -1.0, Modifiers::COMMAND);
    assert_eq!(h.state().settings.zoom, 110);
    // Without Ctrl the wheel does not zoom.
    wheel(&mut h, -3.0, Modifiers::NONE);
    assert_eq!(h.state().settings.zoom, 110);
    key(&mut h, Modifiers::COMMAND, Key::Num0);
    assert_eq!(h.state().settings.zoom, 100);
    // The menus keep their size: egui's own Ctrl+plus zoom is off.
    key(&mut h, Modifiers::COMMAND, Key::Plus);
    assert_eq!(h.ctx.zoom_factor(), 1.0);
    assert_eq!(h.state().settings.zoom, 110);
}

#[test]
fn ctrl_shift_8_toggles_all_characters() {
    let mut h = harness(Vec2::new(700.0, 300.0));
    assert!(!h.state().settings.show_all_characters());
    key(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::Num8);
    let s = &h.state().settings;
    assert!(s.show_space_tab && s.show_eol && s.show_non_printing);
    key(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::Num8);
    assert!(!h.state().settings.show_all_characters());
}

#[test]
fn show_all_characters_looks_right() {
    let mut h = harness(Vec2::new(560.0, 300.0));
    h.event(Event::Paste(
        "fn main() {\n\tlet  a = 1;\n\tif a {\n\t\tgo(a);\u{A0}\n\n\t\tx\u{200B}y\0z\u{1b}[0m\n\t}\n}\n// a comment long enough to wrap at the right edge of this window"
            .into(),
    ));
    h.run();
    {
        let s = &mut h.state_mut().settings;
        s.set_show_all_characters(true);
        s.show_indent_guides = true;
        s.show_wrap_symbol = true;
        s.word_wrap = true;
        s.edge_column = Some(40);
    }
    key(&mut h, Modifiers::COMMAND, Key::Home);
    util::snapshot(&mut h, "show_symbols");
}

#[test]
fn control_characters_are_boxes_with_a_name_on_hover() {
    let mut h = harness(Vec2::new(600.0, 240.0));
    h.event(Event::Paste("a\0b".into()));
    h.run();
    // Hovering the NUL's cell names it, even with Show Symbol off.
    h.event(Event::PointerMoved(cell_pos(&h, 0, 1.0)));
    for _ in 0..90 {
        h.step();
    }
    h.get_by_label_contains("U+0000 NULL");
    // A zero-width space is only named while non-printing characters show.
    h.event(Event::PointerMoved(Pos2::ZERO));
    h.run();
    h.event(Event::Text("\u{200B}".into()));
    h.run();
    h.event(Event::PointerMoved(cell_pos(&h, 0, 3.0)));
    for _ in 0..90 {
        h.step();
    }
    assert!(h.query_by_label_contains("ZERO WIDTH SPACE").is_none());
    h.state_mut().settings.show_non_printing = true;
    h.event(Event::PointerMoved(
        cell_pos(&h, 0, 3.0) + Vec2::new(1.0, 0.0),
    ));
    for _ in 0..90 {
        h.step();
    }
    h.get_by_label_contains("U+200B ZERO WIDTH SPACE");
}
