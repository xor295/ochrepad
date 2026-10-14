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
