//! M8: text transforms from the keyboard and the Edit menu.

use egui::{Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;

fn with_text(text: &str) -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(900.0, 600.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.run();
    h.event(Event::Paste(text.to_owned()));
    h.run();
    h
}

fn text(h: &Harness<'_, OchrepadApp>) -> String {
    h.state().buffer().doc().to_string()
}

fn key(h: &mut Harness<'_, OchrepadApp>, m: Modifiers, k: Key) {
    h.key_press_modifiers(m, k);
    h.run();
}

#[test]
fn keyboard_shortcuts() {
    let mut h = with_text("first line\nsecond");
    key(&mut h, Modifiers::COMMAND, Key::Home);
    key(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::U);
    assert_eq!(text(&h), "FIRST LINE\nsecond");
    key(&mut h, Modifiers::COMMAND, Key::U);
    assert_eq!(text(&h), "first line\nsecond");
    key(&mut h, Modifiers::ALT, Key::U);
    assert_eq!(text(&h), "First Line\nsecond");
    key(&mut h, Modifiers::COMMAND, Key::D);
    assert_eq!(text(&h), "First Line\nFirst Line\nsecond");
    key(
        &mut h,
        Modifiers::COMMAND | Modifiers::SHIFT,
        Key::ArrowDown,
    );
    key(
        &mut h,
        Modifiers::COMMAND | Modifiers::SHIFT,
        Key::ArrowDown,
    );
    assert_eq!(text(&h), "First Line\nsecond\nFirst Line");
    key(&mut h, Modifiers::COMMAND, Key::Home);
    key(&mut h, Modifiers::COMMAND, Key::J);
    assert_eq!(text(&h), "First Line second\nFirst Line");
    // Each was one undo step.
    for _ in 0..7 {
        key(&mut h, Modifiers::COMMAND, Key::Z);
    }
    assert_eq!(text(&h), "first line\nsecond");
}

#[test]
fn sort_from_the_edit_menu() {
    let mut h = with_text("pear\napple\nfig");
    h.get_by_label("Edit").click();
    h.run();
    h.get_by_label_contains("Line Operations").hover();
    h.run();
    h.get_by_label("Sort Lines Ascending").click();
    h.run();
    assert_eq!(text(&h), "apple\nfig\npear");
}

#[test]
fn remove_lines_from_the_edit_menu() {
    let mut h = with_text("b\n\na\n  \nb\na");
    for item in [
        "Remove Empty Lines (Containing Blank characters)",
        "Remove Duplicate Lines",
    ] {
        h.get_by_label("Edit").click();
        h.run();
        h.get_by_label_contains("Line Operations").hover();
        h.run();
        h.get_by_label(item).click();
        h.run();
    }
    assert_eq!(text(&h), "b\na");
}
