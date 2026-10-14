//! More of the File menu: Save All, Save a Copy As, Rename; and the keys
//! for tabs and comments Notepad++ users know.

use egui::{Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;

fn harness() -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(900.0, 600.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.run();
    h
}

fn key(h: &mut Harness<'_, OchrepadApp>, m: Modifiers, k: Key) {
    h.key_press_modifiers(m, k);
    h.run();
}

fn type_text(h: &mut Harness<'_, OchrepadApp>, text: &str) {
    h.event(Event::Text(text.into()));
    h.run();
}

#[test]
fn save_all_saves_every_named_document() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
    std::fs::write(&a, "a").unwrap();
    std::fs::write(&b, "b").unwrap();
    let mut h = harness();
    h.state_mut().open_path(&a);
    h.run();
    type_text(&mut h, "1");
    h.state_mut().open_path(&b);
    h.run();
    type_text(&mut h, "2");
    // An untitled one stays for Save As.
    key(&mut h, Modifiers::COMMAND, Key::N);
    type_text(&mut h, "new");
    h.get_by_label("File").click();
    h.run();
    h.get_by_label("Save All").click();
    h.run();
    h.state_mut().flush_disk();
    h.run();
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "1a");
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "2b");
    h.get_by_label_contains("1 new document without a name");
}

#[test]
fn a_copy_leaves_the_document_on_its_own_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, "one\r\ntwo\r\n").unwrap();
    let mut h = harness();
    h.state_mut().open_path(&file);
    h.run();
    type_text(&mut h, "zero ");
    let copy = dir.path().join("copy.txt");
    let buffer = h.state().buffer_id();
    h.state_mut().save_copy_to(buffer, &copy);
    h.state_mut().flush_disk();
    h.run();
    // In the file's line endings; the document still unsaved, on its file.
    assert_eq!(std::fs::read(&copy).unwrap(), b"zero one\r\ntwo\r\n");
    assert_eq!(std::fs::read(&file).unwrap(), b"one\r\ntwo\r\n");
    assert!(h.state().buffer().is_dirty());
    assert_eq!(h.state().file_name(), "notes.txt");
}

#[test]
fn rename_moves_the_file_and_the_tab_follows() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("old.txt");
    std::fs::write(&file, "text").unwrap();
    std::fs::write(dir.path().join("taken.txt"), "other").unwrap();
    let mut h = harness();
    h.state_mut().open_path(&file);
    h.run();
    h.get_by_label("File").click();
    h.run();
    h.get_by_label("Rename…").click();
    h.run();
    // The field holds the name, selected for typing over: replace it.
    for _ in 0.."old.txt".len() {
        key(&mut h, Modifiers::NONE, Key::Backspace);
    }
    type_text(&mut h, "new.md");
    key(&mut h, Modifiers::NONE, Key::Enter);
    h.state_mut().flush_disk();
    h.run();
    assert!(!file.exists());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("new.md")).unwrap(),
        "text"
    );
    assert_eq!(h.state().file_name(), "new.md");
    // An existing file is not replaced.
    h.state_mut().open_rename();
    h.run();
    for _ in 0.."new.md".len() {
        key(&mut h, Modifiers::NONE, Key::Backspace);
    }
    type_text(&mut h, "taken.txt");
    key(&mut h, Modifiers::NONE, Key::Enter);
    h.state_mut().flush_disk();
    h.run();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("taken.txt")).unwrap(),
        "other"
    );
    assert_eq!(h.state().file_name(), "new.md");
}

#[test]
fn notepad_plus_plus_keys_for_tabs_and_comments() {
    let mut h = harness();
    key(&mut h, Modifiers::COMMAND, Key::N);
    let second = h.state().active_view();
    key(&mut h, Modifiers::COMMAND, Key::PageUp);
    assert_ne!(h.state().active_view(), second);
    key(&mut h, Modifiers::COMMAND, Key::PageDown);
    assert_eq!(h.state().active_view(), second);
    // Ctrl+Q toggles a line comment (here with a language that has one).
    h.state_mut().set_language("Rust");
    type_text(&mut h, "let x = 1;");
    key(&mut h, Modifiers::COMMAND, Key::Q);
    assert_eq!(h.state().buffer().doc().to_string(), "// let x = 1;");
}

#[test]
fn clicking_the_caret_position_opens_go_to_line() {
    let mut h = harness();
    h.get_by_label("Ln 1, Col 1").click();
    h.run();
    h.get_by_label_contains("Go to line");
}
