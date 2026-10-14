//! 0.3.0 P1 in the app: Go to Line, Tab/Shift+Tab, comments and brackets.

mod util;

use egui::{Event, Key, Modifiers, Vec2};
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

fn text(h: &Harness<'_, OchrepadApp>) -> String {
    h.state().buffer().doc().to_string()
}

#[test]
fn go_to_line_asks_for_a_line_and_column() {
    let mut h = harness(Vec2::new(700.0, 400.0));
    h.event(Event::Paste("one\ntwo\nthree\n".into()));
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::G);
    h.event(Event::Text("2:3".into()));
    key(&mut h, Modifiers::NONE, Key::Enter);
    h.get_by_label("Ln 2, Col 3");
}

#[test]
fn comments_follow_the_language_and_say_when_unknown() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("a.rs");
    std::fs::write(&file, "let x = 1;\n").unwrap();
    let mut h = harness(Vec2::new(700.0, 400.0));
    h.state_mut().open_path(&file);
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::Slash);
    assert_eq!(text(&h), "// let x = 1;\n");
    key(&mut h, Modifiers::COMMAND, Key::Slash);
    assert_eq!(text(&h), "let x = 1;\n");

    // Plain text has no comment markers: a note in the status bar.
    key(&mut h, Modifiers::COMMAND, Key::N);
    h.event(Event::Text("x".into()));
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::Slash);
    assert_eq!(text(&h), "x");
    h.get_by_label_contains("No comment markers are known");
}

#[test]
fn tab_and_shift_tab_indent_lines() {
    let mut h = harness(Vec2::new(700.0, 400.0));
    h.event(Event::Paste("a\nb\nc".into()));
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::A);
    key(&mut h, Modifiers::NONE, Key::Tab);
    assert_eq!(text(&h), "\ta\n\tb\n\tc");
    key(&mut h, Modifiers::SHIFT, Key::Tab);
    assert_eq!(text(&h), "a\nb\nc");
}

#[test]
fn matching_brackets_look_right() {
    let mut h = harness(Vec2::new(500.0, 220.0));
    h.event(Event::Paste("fn f(a: [u8; 2]) {\n    g((a));\n}\n".into()));
    h.run();
    // The caret right after "f(".
    key(&mut h, Modifiers::COMMAND, Key::Home);
    for _ in 0..5 {
        key(&mut h, Modifiers::NONE, Key::ArrowRight);
    }
    util::snapshot(&mut h, "brackets");
}
