//! M10: backup snapshots, session restore, write-through and background saving.

use std::path::{Path, PathBuf};

use egui::{Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;
use ochrepad_core::Action;

struct Dirs {
    _tmp: tempfile::TempDir,
    config: PathBuf,
    data: PathBuf,
    files: PathBuf,
}

fn dirs() -> Dirs {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let files = root.join("files");
    std::fs::create_dir_all(&files).unwrap();
    Dirs {
        config: root.join("config"),
        data: root.join("data"),
        files,
        _tmp: tmp,
    }
}

/// An app using `d` for its session and backups, with a short autosave
/// interval (0.5 s) so the timer is quick to cross at 60 frames a second.
fn app(d: &Dirs) -> Harness<'static, OchrepadApp> {
    let mut app = OchrepadApp::new();
    app.set_storage(&d.config, &d.data);
    app.settings.autosave_secs = 0.5;
    let mut h = Harness::builder()
        .with_size(Vec2::new(900.0, 600.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), app);
    h.run();
    h
}

fn typ(h: &mut Harness<'_, OchrepadApp>, s: &str) {
    h.event(Event::Text(s.to_owned()));
    h.run();
}

fn key(h: &mut Harness<'_, OchrepadApp>, m: Modifiers, k: Key) {
    h.key_press_modifiers(m, k);
    h.run();
}

/// Lets the autosave timer fire and waits for the disk.
fn tick(h: &mut Harness<'_, OchrepadApp>) {
    h.run_steps(40);
    h.state_mut().flush_disk();
    h.run();
}

fn backups(d: &Dirs) -> Vec<String> {
    let dir = d.data.join("backup");
    let mut out: Vec<String> = std::fs::read_dir(&dir)
        .map(|r| {
            r.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

fn backup_texts(d: &Dirs) -> Vec<String> {
    backups(d)
        .into_iter()
        .filter(|n| n.ends_with(".txt"))
        .map(|n| std::fs::read_to_string(d.data.join("backup").join(n)).unwrap())
        .collect()
}

fn write(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
}

fn names(h: &Harness<'_, OchrepadApp>) -> Vec<String> {
    let app = h.state();
    app.tabs()
        .into_iter()
        .map(|v| app.buffer_name(app.editor.view(v).unwrap().buffer()))
        .collect()
}

fn text(h: &Harness<'_, OchrepadApp>) -> String {
    h.state().buffer().doc().to_string()
}

#[test]
fn dirty_buffers_are_backed_up_on_the_timer() {
    let d = dirs();
    let mut h = app(&d);
    typ(&mut h, "draft one");
    assert!(backups(&d).is_empty(), "nothing before the timer");
    tick(&mut h);
    assert_eq!(backup_texts(&d), vec!["draft one"]);
    typ(&mut h, " two");
    tick(&mut h);
    assert_eq!(
        backup_texts(&d),
        vec!["draft one two"],
        "one backup per buffer"
    );
}

#[test]
fn exit_keeps_unsaved_work_and_the_next_launch_restores_it() {
    let d = dirs();
    let a = d.files.join("a.txt");
    let b = d.files.join("b.txt");
    write(&a, "file a\n");
    write(&b, "file b\n");
    {
        let mut h = app(&d);
        h.state_mut().open_path(&a);
        h.state_mut().open_path(&b);
        h.run();
        // Edit b without saving, and leave a draft in a new tab.
        key(&mut h, Modifiers::COMMAND, Key::End);
        typ(&mut h, "edited");
        key(&mut h, Modifiers::COMMAND, Key::N);
        typ(&mut h, "untitled draft");
        h.state_mut().request_exit();
        h.run();
        assert!(h.state().may_close(), "no prompt with backups on");
        assert!(h.state().confirming().is_none());
    }
    assert_eq!(
        std::fs::read_to_string(&b).unwrap(),
        "file b\n",
        "b itself untouched"
    );

    let h = app(&d);
    assert_eq!(names(&h), vec!["a.txt", "b.txt", "new 2"]);
    let app = h.state();
    let doc_of = |i: usize| {
        let v = app.tabs()[i];
        let buf = app
            .editor
            .buffer(app.editor.view(v).unwrap().buffer())
            .unwrap();
        (buf.doc().to_string(), buf.is_dirty())
    };
    assert_eq!(doc_of(0), ("file a\n".into(), false));
    assert_eq!(doc_of(1), ("file b\nedited".into(), true));
    assert_eq!(doc_of(2), ("untitled draft".into(), true));
    assert_eq!(app.file_name(), "new 2", "the active tab comes back");
    let caret = app
        .editor
        .view(app.active_view())
        .unwrap()
        .selections
        .primary()
        .head
        .0;
    assert_eq!(caret, "untitled draft".len());
}

#[test]
fn split_layouts_are_restored() {
    let d = dirs();
    let a = d.files.join("a.txt");
    write(&a, "shared\n");
    {
        let mut h = app(&d);
        h.state_mut().open_path(&a);
        h.state_mut().clone_to_other_view();
        h.run();
        h.state_mut().request_exit();
        h.run();
    }
    let h = app(&d);
    let app = h.state();
    assert_eq!(app.dock.iter_leaves().count(), 2);
    let tabs = app.tabs();
    assert_eq!(tabs.len(), 2);
    let buffer = |v| app.editor.view(v).unwrap().buffer();
    assert_eq!(buffer(tabs[0]), buffer(tabs[1]), "two views of one buffer");
}

#[test]
fn saving_or_closing_removes_the_backup() {
    let d = dirs();
    let mut h = app(&d);
    typ(&mut h, "keep me");
    tick(&mut h);
    assert_eq!(backups(&d).len(), 2, "text and info");
    let path = d.files.join("kept.txt");
    assert!(h.state_mut().save_to(&path));
    h.state_mut().flush_disk();
    h.run();
    h.state_mut().flush_disk();
    assert!(backups(&d).is_empty());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep me");

    key(&mut h, Modifiers::COMMAND, Key::N);
    typ(&mut h, "throwaway");
    tick(&mut h);
    assert_eq!(backups(&d).len(), 2);
    key(&mut h, Modifiers::COMMAND, Key::W);
    h.get_by_label("Don't Save").click();
    h.run();
    h.state_mut().flush_disk();
    assert!(backups(&d).is_empty());
}

#[test]
fn a_missing_file_is_reported_and_the_rest_restored() {
    let d = dirs();
    let a = d.files.join("a.txt");
    let gone = d.files.join("gone.txt");
    write(&a, "a");
    write(&gone, "soon gone");
    {
        let mut h = app(&d);
        h.state_mut().open_path(&a);
        h.state_mut().open_path(&gone);
        h.run();
        h.state_mut().request_exit();
        h.run();
    }
    std::fs::remove_file(&gone).unwrap();
    let h = app(&d);
    assert_eq!(names(&h), vec!["a.txt"]);
    assert!(h.state().message().unwrap().contains("gone.txt"));
}

#[test]
fn write_through_saves_files_on_the_timer() {
    let d = dirs();
    let a = d.files.join("a.txt");
    write(&a, "start");
    let mut h = app(&d);
    h.state_mut().settings.write_through = true;
    h.state_mut().open_path(&a);
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::End);
    typ(&mut h, "!");
    tick(&mut h);
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "start!");
    assert!(!h.state().buffer().is_dirty());
}

#[test]
fn typing_during_a_background_save_keeps_the_buffer_dirty() {
    let d = dirs();
    let mut h = app(&d);
    typ(&mut h, "v1");
    let path = d.files.join("race.txt");
    let app = h.state_mut();
    assert!(app.save_to(&path));
    // An edit lands before the save's result is processed.
    let view = app.active_view();
    app.editor
        .apply(view, Action::InsertText("+".into()))
        .unwrap();
    app.flush_disk();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "v1");
    assert!(app.buffer().is_dirty(), "the newer text is not on disk");
    assert_eq!(text(&h), "v1+");
}

#[test]
fn with_backups_off_exit_asks_as_before() {
    let d = dirs();
    let mut h = app(&d);
    h.state_mut().settings.backups = false;
    typ(&mut h, "unsaved");
    h.state_mut().request_exit();
    h.run();
    assert!(!h.state().may_close());
    h.get_by_label("Save changes to new 1?");
}

#[test]
fn folds_come_back_with_the_session() {
    let d = dirs();
    let a = d.files.join("a.py");
    write(
        &a,
        "def f():
    return 1

def g():
    return 2
",
    );
    {
        let mut h = app(&d);
        h.state_mut().open_path(&a);
        h.run();
        key(&mut h, Modifiers::ALT, Key::Num0);
        h.state_mut().request_exit();
        h.run();
    }
    let h = app(&d);
    let app = h.state();
    let folds: Vec<usize> = app
        .editor
        .view(app.active_view())
        .unwrap()
        .folds()
        .headers()
        .collect();
    assert_eq!(folds, vec![0, 3]);
}

#[test]
fn the_open_folder_comes_back_with_the_session() {
    let d = dirs();
    let project = d.files.join("project");
    std::fs::create_dir_all(project.join("src")).unwrap();
    {
        let mut h = app(&d);
        h.state_mut().open_folder(&project);
        h.run();
        h.state_mut().request_exit();
        h.run();
    }
    let h = app(&d);
    let tree = h.state().file_tree().expect("a folder is open");
    assert_eq!(tree.root(), project.as_path());
}

#[test]
fn a_crash_backup_brings_unsaved_work_back() {
    let d = dirs();
    let mut h = app(&d);
    // Typed just now: no autosave tick has run.
    typ(&mut h, "typed right before a crash");
    assert!(h.state_mut().emergency_backup());
    drop(h);
    let h = app(&d);
    assert_eq!(
        h.state().buffer().doc().to_string(),
        "typed right before a crash"
    );
    assert!(h.state().buffer().is_dirty());
}

fn undo(h: &mut Harness<'_, OchrepadApp>) {
    key(h, Modifiers::COMMAND, Key::Z);
}

fn histories(d: &Dirs) -> usize {
    std::fs::read_dir(d.data.join("history")).map_or(0, |r| r.count())
}

#[test]
fn undo_reaches_back_past_closing_and_reopening_a_file() {
    let d = dirs();
    let file = d.files.join("notes.txt");
    write(&file, "first\n");
    let mut h = app(&d);
    h.state_mut().open_path(&file);
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::End);
    typ(&mut h, "second");
    key(&mut h, Modifiers::COMMAND, Key::S);
    h.state_mut().flush_disk();
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::W);
    h.state_mut().flush_disk();
    assert_eq!(histories(&d), 1);

    h.state_mut().open_path(&file);
    h.run();
    assert_eq!(text(&h), "first\nsecond");
    assert!(!h.state().buffer().is_dirty());
    undo(&mut h);
    assert_eq!(text(&h), "first\n");
    assert!(h.state().buffer().is_dirty());
    key(&mut h, Modifiers::COMMAND, Key::Y);
    assert!(
        !h.state().buffer().is_dirty(),
        "redo back to the saved text"
    );
}

#[test]
fn a_file_changed_since_gets_no_history() {
    let d = dirs();
    let file = d.files.join("notes.txt");
    write(&file, "first\n");
    let mut h = app(&d);
    h.state_mut().open_path(&file);
    h.run();
    typ(&mut h, "x");
    key(&mut h, Modifiers::COMMAND, Key::S);
    h.state_mut().flush_disk();
    key(&mut h, Modifiers::COMMAND, Key::W);
    h.state_mut().flush_disk();
    // Another program changes it while it is closed.
    write(&file, "xfirst\nchanged\n");
    h.state_mut().open_path(&file);
    h.run();
    undo(&mut h);
    assert_eq!(text(&h), "xfirst\nchanged\n");
}

#[test]
fn unsaved_work_comes_back_with_its_undo_history() {
    let d = dirs();
    {
        let mut h = app(&d);
        typ(&mut h, "one");
        key(&mut h, Modifiers::NONE, Key::Enter);
        typ(&mut h, "two");
        h.state_mut().request_exit();
        h.run();
    }
    let mut h = app(&d);
    assert_eq!(text(&h), "one\ntwo");
    assert!(h.state().buffer().is_dirty());
    undo(&mut h);
    assert_eq!(text(&h), "one\n");
    undo(&mut h);
    undo(&mut h);
    assert_eq!(text(&h), "");
}

#[test]
fn undo_history_can_be_turned_off_or_cleared() {
    let d = dirs();
    let file = d.files.join("notes.txt");
    write(&file, "first\n");
    let mut h = app(&d);
    h.state_mut().open_path(&file);
    h.run();
    typ(&mut h, "x");
    key(&mut h, Modifiers::COMMAND, Key::S);
    h.state_mut().flush_disk();
    h.run();
    assert_eq!(histories(&d), 1);
    // Cleared: gone here and on disk.
    h.state_mut().clear_undo_history();
    h.state_mut().flush_disk();
    h.run();
    assert_eq!(histories(&d), 0);
    undo(&mut h);
    assert_eq!(text(&h), "xfirst\n");
    // Off: nothing is written.
    h.state_mut().settings.undo_history = false;
    h.run();
    typ(&mut h, "y");
    key(&mut h, Modifiers::COMMAND, Key::S);
    key(&mut h, Modifiers::COMMAND, Key::W);
    h.state_mut().flush_disk();
    assert_eq!(histories(&d), 0);
}
