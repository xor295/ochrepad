//! Files that used to crash or hang the app (the 0.3.0 security review).

use egui::{Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use ochrepad::OchrepadApp;

fn harness() -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(800.0, 400.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .wgpu()
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.run();
    h
}

#[test]
fn a_line_of_megabytes_renders() {
    // One galley for the whole line overflowed the GPU's vertex buffer
    // (more than 256 MB) and the app exited.
    let mut h = harness();
    h.event(Event::Paste("x".repeat(4_000_000)));
    h.run();
    h.render().expect("renders");
    // Scrolled to its end too (the caret is there after the paste), and back.
    h.key_press_modifiers(Modifiers::NONE, Key::Home);
    h.run();
    h.render().expect("renders");
}

#[test]
fn a_bracket_beside_megabytes_of_non_ascii_text() {
    // The bracket search window's edge fell inside a character and panicked.
    let mut h = harness();
    h.event(Event::Paste(format!("{{{}", "漢😀".repeat(30_000))));
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::Home);
    h.run();
    h.render().expect("renders");
}

#[test]
fn one_grapheme_of_megabytes_renders() {
    // A letter with millions of combining marks is one grapheme, laid out
    // as one piece: it overflowed the GPU's vertex buffer.
    let mut h = harness();
    h.event(Event::Paste(format!("a{}", "\u{301}".repeat(4_000_000))));
    h.run();
    h.render().expect("renders");
}

#[test]
fn ctrl_f_on_a_selection_of_megabytes_renders() {
    // The find field took the whole selection as its text and laid it out.
    let mut h = harness();
    h.event(Event::Paste("a".repeat(4_000_000)));
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::F);
    h.run();
    h.render().expect("renders");
    assert!(h.state().find.pattern.len() <= ochrepad::find_panel::MAX_PATTERN);
}

#[test]
fn a_line_of_flag_letters_lays_out_in_linear_time() {
    // Each grapheme's cursor looked back over every flag letter before it.
    let line: String = "\u{1F1FA}\u{1F1F8}".repeat(32_000);
    let t = std::time::Instant::now();
    let layout = ochrepad::layout::LineLayout::build(line, 8.0, 4, egui::Color32::WHITE, None);
    assert_eq!(layout.width, 32_000.0 * 2.0 * 8.0);
    if !cfg!(debug_assertions) {
        assert!(t.elapsed() < std::time::Duration::from_millis(100));
    }
}
