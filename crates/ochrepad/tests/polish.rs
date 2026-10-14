//! M13: settings, keymap file, recent files, drag-and-drop, external changes.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use egui::{Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;
use ochrepad::app::DiskChange;

struct Dirs {
    tmp: tempfile::TempDir,
}

impl Dirs {
    fn new() -> Self {
        Self {
            tmp: tempfile::tempdir().unwrap(),
        }
    }
    fn config(&self) -> PathBuf {
        self.tmp.path().join("config")
    }
    fn data(&self) -> PathBuf {
        self.tmp.path().join("data")
    }
    fn file(&self, name: &str, text: &str) -> PathBuf {
        let p = self.tmp.path().join(name);
        std::fs::write(&p, text).unwrap();
        p
    }
}

fn app(d: &Dirs) -> Harness<'static, OchrepadApp> {
    let mut app = OchrepadApp::new();
    app.set_storage(&d.config(), &d.data());
    let mut h = Harness::builder()
        .with_size(Vec2::new(1000.0, 600.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), app);
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

/// Steps (in real time) until `done` or ~5 s pass.
fn wait_for(h: &mut Harness<'_, OchrepadApp>, done: impl Fn(&OchrepadApp) -> bool) -> bool {
    for _ in 0..500 {
        h.step();
        if done(h.state()) {
            h.run();
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn settings_are_saved_and_come_back() {
    let d = Dirs::new();
    {
        let mut h = app(&d);
        let s = &mut h.state_mut().settings;
        s.tab_width = 2;
        s.use_spaces = true;
        s.font_size = 18.0;
        h.run();
        h.state_mut().flush_disk();
    }
    assert!(d.config().join("settings.json").exists());
    let mut h = app(&d);
    assert_eq!(h.state().settings.tab_width, 2);
    assert_eq!(h.state().settings.font_size, 18.0);
    key(&mut h, Modifiers::NONE, Key::Tab);
    assert_eq!(text(&h), "  ", "Tab inserts spaces to the 2-column stop");
}

#[test]
fn the_preferences_window_opens_from_the_menu() {
    let d = Dirs::new();
    let mut h = app(&d);
    h.get_by_label("Settings").click();
    h.run();
    h.get_by_label("Preferences…").click();
    h.run();
    h.get_by_label("Tab width");
    h.get_by_label("Indent with");
    assert!(h.state().show_preferences);
}

#[test]
fn editing_the_keymap_file_changes_the_bindings() {
    let d = Dirs::new();
    let mut h = app(&d);
    h.state_mut().edit_keymap();
    h.run();
    assert_eq!(h.state().file_name(), "keymap.json");
    assert!(
        text(&h).contains("\"keys\": \"Ctrl+D\""),
        "starts from the defaults"
    );
    // Replace it: Ctrl+D selects the line, Ctrl+J does nothing.
    key(&mut h, Modifiers::COMMAND, Key::A);
    h.event(Event::Paste(
        r#"[{"keys": "Ctrl+D", "command": {"Action": "SelectLine"}}, {"keys": "Ctrl+J", "command": null}]"#
            .into(),
    ));
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::S);
    h.state_mut().flush_disk();
    h.run();

    key(&mut h, Modifiers::COMMAND, Key::N);
    h.event(Event::Paste("one\ntwo".into()));
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::D);
    let app = h.state();
    let sel = app
        .editor
        .view(app.active_view())
        .unwrap()
        .selections
        .primary()
        .clone();
    assert_eq!(
        (sel.start().0, sel.end().0),
        (4, 7),
        "Ctrl+D selected the line"
    );
    key(&mut h, Modifiers::COMMAND, Key::J);
    assert_eq!(text(&h), "one\ntwo", "Ctrl+J no longer joins");
}

#[test]
fn a_broken_keymap_file_is_reported_and_ignored() {
    let d = Dirs::new();
    std::fs::create_dir_all(d.config()).unwrap();
    std::fs::write(d.config().join("keymap.json"), "{ nope").unwrap();
    let mut h = app(&d);
    assert!(h.state().message().unwrap().contains("keymap"));
    h.get_by_label("OK").click();
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::N);
    h.event(Event::Paste("a\nb".into()));
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::Home);
    key(&mut h, Modifiers::COMMAND, Key::D);
    assert_eq!(text(&h), "a\na\nb", "default Ctrl+D still duplicates");
}

#[test]
fn recent_files_are_remembered() {
    let d = Dirs::new();
    let a = d.file("a.txt", "a");
    let b = d.file("b.txt", "b");
    {
        let mut h = app(&d);
        h.state_mut().open_path(&a);
        h.state_mut().open_path(&b);
        h.run();
        h.state_mut().flush_disk();
    }
    let mut h = app(&d);
    let names: Vec<String> = h
        .state()
        .settings
        .recent_files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["b.txt", "a.txt"], "most recent first");
    h.get_by_label("File").click();
    h.run();
    h.get_by_label_contains("Recent Files").hover();
    h.run();
    h.get_by_label("a.txt").click();
    h.run();
    assert_eq!(h.state().file_name(), "a.txt");
}

#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

#[test]
fn dropped_files_open() {
    let d = Dirs::new();
    let file = d.file("dropped.txt", "from a drop");
    let mut h = app(&d);
    h.input_mut().dropped_files.push(Arc::new(Dropped(file)));
    h.run();
    assert_eq!(h.state().file_name(), "dropped.txt");
    assert_eq!(text(&h), "from a drop");
}

/// Writes `text` to `path` so that its modified time surely changes.
fn rewrite(path: &Path, text: &str) {
    std::thread::sleep(Duration::from_millis(50));
    std::fs::write(path, text).unwrap();
}

#[test]
fn a_clean_file_changed_elsewhere_can_be_reloaded() {
    let d = Dirs::new();
    let file = d.file("watched.txt", "first\n");
    let mut h = app(&d);
    h.state_mut().open_path(&file);
    h.run();
    rewrite(&file, "second\n");
    assert!(wait_for(&mut h, |a| !a.pending_disk_changes().is_empty()));
    assert_eq!(h.state().pending_disk_changes()[0].1, DiskChange::Modified);
    h.get_by_label_contains("was changed by another program");
    h.get_by_label("Reload").click();
    h.run();
    assert_eq!(text(&h), "second\n");
    assert!(!h.state().buffer().is_dirty());
}

#[test]
fn a_dirty_file_changed_elsewhere_warns_and_can_keep_the_edits() {
    let d = Dirs::new();
    let file = d.file("watched.txt", "base\n");
    let mut h = app(&d);
    h.state_mut().open_path(&file);
    h.run();
    typ(&mut h, "mine ");
    rewrite(&file, "theirs\n");
    assert!(wait_for(&mut h, |a| !a.pending_disk_changes().is_empty()));
    h.get_by_label_contains("you have unsaved changes");
    h.get_by_label("Keep my changes").click();
    h.run();
    assert_eq!(text(&h), "mine base\n");
    assert!(h.state().buffer().is_dirty());
}

#[test]
fn a_deleted_file_can_be_kept_open_as_unsaved() {
    let d = Dirs::new();
    let file = d.file("gone.txt", "text\n");
    let mut h = app(&d);
    h.state_mut().open_path(&file);
    h.run();
    std::fs::remove_file(&file).unwrap();
    assert!(wait_for(&mut h, |a| !a.pending_disk_changes().is_empty()));
    h.get_by_label_contains("no longer exists");
    h.get_by_label("Keep open").click();
    h.run();
    assert!(h.state().buffer().is_dirty());
}

#[test]
fn our_own_saves_do_not_ask() {
    let d = Dirs::new();
    let file = d.file("mine.txt", "v1\n");
    let mut h = app(&d);
    h.state_mut().open_path(&file);
    h.run();
    typ(&mut h, "edit ");
    key(&mut h, Modifiers::COMMAND, Key::S);
    h.state_mut().flush_disk();
    let asked = wait_for(&mut h, |a| !a.pending_disk_changes().is_empty());
    assert!(!asked, "saving our own file is not an external change");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "edit v1\n");
}

#[test]
fn hovering_another_menu_title_switches_menus() {
    let d = Dirs::new();
    let mut h = app(&d);
    h.get_by_label("Edit").hover();
    h.run();
    assert!(
        h.query_by_label_contains("Undo").is_none(),
        "hovering alone opens nothing"
    );
    h.get_by_label("File").click();
    h.run();
    h.get_by_label_contains("Open…");
    h.get_by_label("Edit").hover();
    h.run();
    h.get_by_label_contains("Undo Ctrl+Z");
    assert!(
        h.query_by_label_contains("Open…").is_none(),
        "the File menu closed"
    );
    h.get_by_label("Settings").hover();
    h.run();
    h.get_by_label("Preferences…");
}
