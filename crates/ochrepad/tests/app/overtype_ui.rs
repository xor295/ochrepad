//! Overtype: Insert switches to typing over the text, the status bar says
//! so, and a click there switches back.

use egui::{Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;

fn harness() -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(900.0, 500.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.run();
    h
}

fn type_text(h: &mut Harness<'_, OchrepadApp>, text: &str) {
    h.event(Event::Text(text.into()));
    h.run();
}

fn text(h: &Harness<'_, OchrepadApp>) -> String {
    h.state().buffer().doc().to_string()
}

#[test]
fn insert_switches_to_typing_over_and_back() {
    let mut h = harness();
    type_text(&mut h, "hello");
    h.key_press_modifiers(Modifiers::NONE, Key::Home);
    h.run();
    assert!(h.query_by_label("OVR").is_none());

    h.key_press_modifiers(Modifiers::NONE, Key::Insert);
    h.run();
    assert!(h.state().editor.settings.overtype);
    type_text(&mut h, "J");
    assert_eq!(text(&h), "Jello");

    // Insert again goes back to inserting.
    h.key_press_modifiers(Modifiers::NONE, Key::Insert);
    h.run();
    type_text(&mut h, "x");
    assert_eq!(text(&h), "Jxello");

    // The status bar's OVR switches it off too, and the editor keeps the
    // keyboard.
    h.key_press_modifiers(Modifiers::NONE, Key::Insert);
    h.run();
    h.get_by_label("OVR").click();
    h.run();
    assert!(!h.state().editor.settings.overtype);
    assert!(h.query_by_label("OVR").is_none());
    type_text(&mut h, "y");
    assert_eq!(text(&h), "Jxyello");
}
