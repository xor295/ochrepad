//! M9: macros in the app.

use std::path::Path;

use egui::{Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;

fn app_with(text: &str, macro_file: &Path) -> Harness<'static, OchrepadApp> {
    let mut app = OchrepadApp::new();
    app.set_macro_file(macro_file.to_path_buf());
    let mut h = Harness::builder()
        .with_size(Vec2::new(900.0, 600.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), app);
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

fn typ(h: &mut Harness<'_, OchrepadApp>, s: &str) {
    h.event(Event::Text(s.to_owned()));
    h.run();
}

/// Records "go to line end, type ';', move down" with the keyboard.
fn record_semicolons(h: &mut Harness<'_, OchrepadApp>) {
    key(h, Modifiers::COMMAND, Key::Home);
    key(h, Modifiers::COMMAND | Modifiers::SHIFT, Key::R);
    key(h, Modifiers::NONE, Key::End);
    typ(h, ";");
    h.get_by_label("Recording macro (2 actions)");
    key(h, Modifiers::NONE, Key::ArrowDown);
    key(h, Modifiers::COMMAND | Modifiers::SHIFT, Key::R);
    assert!(h.query_by_label_contains("Recording macro").is_none());
}

#[test]
fn record_and_play_back_with_the_keyboard() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = app_with("a\nb\nc\nd", &dir.path().join("macros.json"));
    record_semicolons(&mut h);
    assert_eq!(h.state().macros.recorded.len(), 3);
    key(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    key(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    assert_eq!(text(&h), "a;\nb;\nc;\nd");
}

#[test]
fn saved_macros_persist_and_run_from_their_shortcut() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("macros.json");
    let mut h = app_with("a\nb", &file);
    record_semicolons(&mut h);
    h.state_mut()
        .macros
        .save_recorded("semicolons", "Ctrl+Alt+M")
        .unwrap();
    assert!(file.exists());

    // A fresh app finds it in the file and in the Macro menu.
    let mut h = app_with("x\ny", &file);
    assert_eq!(h.state().macros.saved[0].name, "semicolons");
    h.get_by_label("Macro").click();
    h.run();
    h.get_by_label_contains("semicolons");
    key(&mut h, Modifiers::NONE, Key::Escape);
    key(&mut h, Modifiers::COMMAND, Key::Home);
    key(&mut h, Modifiers::COMMAND | Modifiers::ALT, Key::M);
    assert_eq!(text(&h), "x;\ny");
}

#[test]
fn the_save_dialog_explains_what_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = app_with("a", &dir.path().join("macros.json"));
    record_semicolons(&mut h);
    h.state_mut().macros.open_save_dialog();
    h.run();
    h.get_by_label("Save").click();
    h.run();
    h.get_by_label("Give the macro a name.");
    h.get_by_label("Cancel").click();
    h.run();
    assert!(!h.state().macros.dialog_open());
}

#[test]
fn run_until_the_end_of_the_file_from_the_dialog() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = app_with("a\nb\nc\nd\ne", &dir.path().join("macros.json"));
    record_semicolons(&mut h);
    h.state_mut().macros.open_run_dialog();
    h.run();
    h.get_by_label("Run until the end of the file").click();
    h.run();
    h.get_by_label("Run").click();
    h.run();
    // The recording handled line 1; the run continues from line 2 to the end.
    assert_eq!(text(&h), "a;\nb;\nc;\nd;\ne;");
    // All of it undoes in one step.
    key(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(text(&h), "a;\nb\nc\nd\ne");
}

#[test]
fn a_failing_macro_reports_where_it_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = app_with("one TODO", &dir.path().join("macros.json"));
    key(&mut h, Modifiers::COMMAND, Key::Home);
    // Record: find "TODO" (through the find panel), type over it.
    key(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::R);
    {
        let app = h.state_mut();
        app.find.pattern = "TODO".into();
        let view = app.active_view();
        app.find.find(&mut app.editor, view, false);
    }
    h.run();
    typ(&mut h, "DONE");
    key(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::R);
    h.state_mut().play_macro(
        ochrepad::macro_ui::MacroSource::Recorded,
        ochrepad_core::Repeat::Times(2),
    );
    h.run();
    let msg = h.state().message().unwrap().to_owned();
    assert!(
        msg.contains("stopped at step 1 (find next \"TODO\") of run 1"),
        "{msg}"
    );
}
