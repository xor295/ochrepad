//! 0.3.0 P2 in the app: per-document indentation, line endings and
//! encodings in the status bar.

use egui::{Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;

struct Dirs {
    tmp: tempfile::TempDir,
}

impl Dirs {
    fn new() -> Self {
        Self {
            tmp: tempfile::tempdir().unwrap(),
        }
    }
    fn file(&self, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = self.tmp.path().join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }
}

fn app(d: &Dirs) -> Harness<'static, OchrepadApp> {
    let mut app = OchrepadApp::new();
    app.set_storage(&d.tmp.path().join("config"), &d.tmp.path().join("data"));
    let mut h = Harness::builder()
        .with_size(Vec2::new(1000.0, 400.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), app);
    h.run();
    h
}

fn key(h: &mut Harness<'_, OchrepadApp>, m: Modifiers, k: Key) {
    h.key_press_modifiers(m, k);
    h.run();
}

fn click(h: &mut Harness<'_, OchrepadApp>, label: &str) {
    h.get_by_label(label).click();
    h.run();
}

fn text(h: &Harness<'_, OchrepadApp>) -> String {
    h.state().buffer().doc().to_string()
}

fn save(h: &mut Harness<'_, OchrepadApp>) {
    key(h, Modifiers::COMMAND, Key::S);
    h.state_mut().flush_disk();
    h.run();
}

#[test]
fn indentation_is_detected_on_open_and_can_be_changed() {
    let d = Dirs::new();
    let file = d.file("a.js", b"a {\n  b {\n    c;\n  }\n}\n");
    let mut h = app(&d);
    h.state_mut().open_path(&file);
    h.run();
    // Two spaces were found, so Tab indents by two spaces.
    h.get_by_label("Spaces: 2");
    key(&mut h, Modifiers::NONE, Key::Tab);
    assert!(text(&h).starts_with("  a {"), "{:?}", text(&h));

    click(&mut h, "Spaces: 2");
    click(&mut h, "Width 4");
    h.get_by_label("Spaces: 4");
    click(&mut h, "Spaces: 4");
    click(&mut h, "Indent Using Tabs");
    h.get_by_label("Tabs: 4");
}

#[test]
fn mixed_indentation_is_reported_and_converted() {
    let d = Dirs::new();
    let file = d.file("m.py", b"def f():\n    a()\n    b()\n\tc()\n");
    let mut h = app(&d);
    h.state_mut().open_path(&file);
    h.run();
    click(&mut h, "Mixed, Spaces: 4");
    click(&mut h, "Convert Indentation to Spaces");
    assert_eq!(text(&h), "def f():\n    a()\n    b()\n    c()\n");
    h.get_by_label("Spaces: 4");
    // One undo step puts the tab back.
    key(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(text(&h), "def f():\n    a()\n    b()\n\tc()\n");
}

#[test]
fn choosing_a_line_ending_changes_the_saved_bytes() {
    let d = Dirs::new();
    let file = d.file("e.txt", b"a\r\nb\r\n");
    let mut h = app(&d);
    h.state_mut().open_path(&file);
    h.run();
    click(&mut h, "Windows (CR LF)");
    click(&mut h, "Unix (LF)");
    assert!(h.state().buffer().is_dirty());
    h.get_by_label("Unix (LF)");
    save(&mut h);
    assert_eq!(std::fs::read(&file).unwrap(), b"a\nb\n");
    assert!(!h.state().buffer().is_dirty());
}

#[test]
fn save_with_encoding_writes_it_or_names_the_character_it_cannot_hold() {
    let d = Dirs::new();
    let file = d.file("u.txt", "x \u{2192} y\n\u{e9}\u{2192}\u{2603}\n".as_bytes());
    let mut h = app(&d);
    h.state_mut().open_path(&file);
    h.run();

    // Arrows and snowmen have no place in Windows-1252 (é does): refused,
    // listing each once, and nothing changes.
    h.state_mut().save_with_encoding("windows-1252", false);
    h.run();
    let msg = h.state().message().unwrap().to_owned();
    assert!(msg.contains("U+2192) on line 1"), "{msg}");
    assert!(msg.contains("U+2603) on line 2"), "{msg}");
    assert_eq!(msg.matches("U+").count(), 2, "{msg}");
    assert_eq!(h.state().buffer().meta.encoding, "UTF-8");
    assert!(!h.state().buffer().is_dirty());
    click(&mut h, "OK");

    h.state_mut().save_with_encoding("UTF-16LE", true);
    h.run();
    h.get_by_label("UTF-16LE BOM");
    save(&mut h);
    let mut want = vec![0xFF, 0xFE];
    for u in "x \u{2192} y\n\u{e9}\u{2192}\u{2603}\n".encode_utf16() {
        want.extend(u.to_le_bytes());
    }
    assert_eq!(std::fs::read(&file).unwrap(), want);
}

#[test]
fn reopen_with_encoding_reads_the_file_again() {
    let d = Dirs::new();
    let file = d.file("c.txt", "caf\u{e9}\n".as_bytes());
    let mut h = app(&d);
    h.state_mut().open_path(&file);
    h.run();
    assert_eq!(text(&h), "caf\u{e9}\n");

    h.state_mut().reopen_with_encoding("windows-1252");
    h.run();
    assert_eq!(text(&h), "caf\u{c3}\u{a9}\n");
    assert_eq!(h.state().buffer().meta.encoding, "windows-1252");
    assert!(!h.state().buffer().is_dirty());

    // Not while there are changes the file does not have.
    h.event(Event::Text("!".into()));
    h.run();
    h.state_mut().reopen_with_encoding("UTF-8");
    h.run();
    assert!(h.state().message().unwrap().contains("Save or undo"));
    assert!(text(&h).starts_with('!'));
}
