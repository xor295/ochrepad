//! File > Save Session… and Load Session…: a session file lists the open
//! files with their carets and first lines, the active one and the open
//! folder; loading it opens them again beside the open ones.

use std::path::{Path, PathBuf};
use std::time::Duration;

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

fn file(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

/// The editor tabs' names, in tab order.
fn names(h: &Harness<'_, OchrepadApp>) -> Vec<String> {
    let app = h.state();
    app.tabs()
        .into_iter()
        .map(|v| app.buffer_name(app.editor.view(v).unwrap().buffer()))
        .collect()
}

/// Each editor tab's caret (as a byte offset) and first line on screen.
fn places(h: &Harness<'_, OchrepadApp>) -> Vec<(usize, usize)> {
    let app = h.state();
    app.tabs()
        .into_iter()
        .map(|v| {
            let v = app.editor.view(v).unwrap();
            (v.selections.primary().head.0, v.scroll.first_line)
        })
        .collect()
}

/// Steps (in real time) until the viewer's background index is done.
fn wait_for_viewer(h: &mut Harness<'_, OchrepadApp>) {
    for _ in 0..1000 {
        h.step();
        let indexed = h
            .state()
            .viewers
            .iter()
            .next()
            .is_some_and(|(_, v)| v.file.progress().done);
        if indexed {
            h.run();
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out");
}

#[test]
fn a_saved_session_opens_its_files_again() {
    let dir = tempfile::tempdir().unwrap();
    let a = file(dir.path(), "a.txt", "first line\nsecond line\n");
    let long: String = (1..=300).map(|i| format!("line {i}\n")).collect();
    let b = file(dir.path(), "b.rs", &long);
    let c = file(dir.path(), "c.md", "# Title\n\nText\n");
    let mut h = harness();
    h.state_mut().open_folder(dir.path());
    h.state_mut().open_path(&a);
    h.run();
    key(&mut h, Modifiers::NONE, Key::ArrowDown);
    key(&mut h, Modifiers::NONE, Key::End);
    h.state_mut().open_path(&b);
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::End);
    h.state_mut().open_path(&c);
    h.run();
    // A document never saved has no file to list.
    h.state_mut().new_file();
    h.run();
    h.event(Event::Text("draft".into()));
    h.run();
    let b_view = h.state().tabs()[1];
    h.state_mut().activate(b_view);
    h.run();
    let saved = places(&h);
    assert_eq!(saved[0], (22, 0));
    assert_eq!(saved[1].0, long.len());
    assert!(saved[1].1 > 250, "b is scrolled to its end: {saved:?}");

    let session = dir.path().join("work.json");
    h.state_mut().save_session_to(&session);
    h.state_mut().flush_disk();
    h.run();
    h.get_by_label_contains("1 new document without a name left out");
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&session).unwrap()).unwrap();
    assert_eq!(json["version"], 1);
    let listed: Vec<&str> = json["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    let path = |p: &Path| p.to_str().unwrap().to_owned();
    assert_eq!(listed, [path(&a), path(&b), path(&c)]);
    assert_eq!(json["active"], 1);
    assert_eq!(json["folder"].as_str(), Some(path(dir.path()).as_str()));

    // Close everything (the draft without saving), then load the session.
    h.state_mut().close_folder();
    h.state_mut().close_all();
    h.run();
    h.get_by_label("Don't Save").click();
    h.run();
    assert_eq!(names(&h).len(), 1);
    h.state_mut().load_session_from(&session);
    h.run();
    assert!(h.state().message().is_none(), "{:?}", h.state().message());
    assert_eq!(names(&h), ["a.txt", "b.rs", "c.md"]);
    assert_eq!(places(&h)[..], saved[..3]);
    assert_eq!(h.state().file_name(), "b.rs");
    let tree = h.state().file_tree().expect("the folder is open again");
    assert_eq!(tree.root(), dir.path());
    h.get_by_label_contains("Loaded work.json (3 files)");

    // Loading it again switches to the open files instead of opening
    // them twice.
    h.state_mut().load_session_from(&session);
    h.run();
    assert_eq!(names(&h), ["a.txt", "b.rs", "c.md"]);
}

#[test]
fn files_that_cannot_be_opened_are_listed_and_the_rest_open() {
    let dir = tempfile::tempdir().unwrap();
    let keep = file(dir.path(), "keep.txt", "kept\n");
    // é is two bytes, 1..3.
    file(dir.path(), "a.txt", "héllo\n");
    file(dir.path(), "short.txt", "one\ntwo\n");
    let session = file(
        dir.path(),
        "work.json",
        r#"{
            "version": 1,
            "files": [
                {"path": "a.txt", "caret": 2},
                {"path": "gone.txt"},
                {"caret": 3},
                {"path": "\\\\host\\share\\x.txt"},
                {"path": "short.txt", "caret": 1000, "first_line": 1000}
            ],
            "active": 0,
            "folder": "moved away"
        }"#,
    );
    let mut h = harness();
    h.state_mut().open_path(&keep);
    h.run();
    h.event(Event::Text("x".into()));
    h.run();
    h.state_mut().load_session_from(&session);
    h.run();
    // Nothing else is closed; the files that could be opened are, with
    // their carets clamped onto a char boundary and into the text, and
    // the first line to the last.
    assert_eq!(names(&h), ["keep.txt", "a.txt", "short.txt"]);
    assert_eq!(places(&h), [(1, 0), (1, 0), (8, 2)]);
    assert_eq!(h.state().file_name(), "a.txt");
    assert!(h.state().file_tree().is_none());
    let msg = h
        .state()
        .message()
        .expect("the problems are listed")
        .to_owned();
    for what in [
        "gone.txt",
        "Entry 3: missing field `path`",
        r"\\host\share\x.txt",
        "moved away",
    ] {
        assert!(msg.contains(what), "{what}: {msg}");
    }
    if cfg!(windows) {
        assert!(
            msg.contains(r"\\host\share\x.txt: not on this computer"),
            "{msg}"
        );
    }
    // A line for each of the four, after the first.
    assert_eq!(msg.lines().count(), 5, "{msg}");
    h.get_by_label("OK").click();
    h.run();
    assert!(h.state().message().is_none());
}

#[test]
fn a_session_opens_at_most_two_hundred_files() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..205 {
        file(dir.path(), &format!("f{i:03}.txt"), "x\n");
    }
    let entries: Vec<String> = (0..205)
        .map(|i| format!(r#"{{"path": "f{i:03}.txt"}}"#))
        .collect();
    let session = file(
        dir.path(),
        "many.json",
        &format!(r#"{{"version": 1, "files": [{}]}}"#, entries.join(",")),
    );
    let mut h = harness();
    h.state_mut().load_session_from(&session);
    h.run();
    assert_eq!(h.state().tabs().len(), 200);
    let msg = h.state().message().unwrap_or_default().to_owned();
    assert!(msg.contains("5 files not opened"), "{msg}");
}

#[test]
fn a_file_too_large_or_not_a_session_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let a = file(dir.path(), "a.txt", "a\n");
    file(dir.path(), "b.txt", "b\n");
    let mut h = harness();
    h.state_mut().open_path(&a);
    h.run();
    // A valid session listing b.txt, but padded to over 1 MB.
    let large = format!(
        r#"{{"version": 1, "files": [{{"path": "b.txt"}}]}}{}"#,
        " ".repeat(1024 * 1024)
    );
    let cases = [
        ("large.json", large, "over 1 MB"),
        (
            "broken.json",
            r#"{"version": 1, "files": ["#.to_owned(),
            "not a session file",
        ),
        (
            "settings.json",
            r#"{"theme": "Dark"}"#.to_owned(),
            "not a session file",
        ),
        (
            "newer.json",
            r#"{"version": 2, "files": [{"path": "b.txt"}]}"#.to_owned(),
            "newer version",
        ),
    ];
    for (name, text, why) in cases {
        let path = file(dir.path(), name, &text);
        h.state_mut().load_session_from(&path);
        h.run();
        let msg = h.state().message().unwrap_or_default().to_owned();
        assert!(msg.contains(why), "{name}: {msg}");
        assert_eq!(names(&h), ["a.txt"], "{name}");
        h.get_by_label("OK").click();
        h.run();
    }
}

#[test]
fn a_large_file_comes_back_in_the_viewer_at_its_line() {
    let dir = tempfile::tempdir().unwrap();
    let log: String = (1..=60_000)
        .map(|i| format!("2026-10-14 INFO line {i}\n"))
        .collect();
    let path = file(dir.path(), "big.log", &log);
    let mut h = harness();
    h.state_mut().settings.viewer_threshold_mb = 1;
    h.state_mut().open_path(&path);
    h.run();
    let id = h.state_mut().focused_viewer().expect("in the viewer");
    wait_for_viewer(&mut h);
    let viewer = &mut h.state_mut().viewers.get_mut(id).unwrap().state;
    viewer.scroll_to_line(30_000);
    h.run();
    let session = dir.path().join("logs.json");
    h.state_mut().save_session_to(&session);
    h.state_mut().flush_disk();
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&session).unwrap()).unwrap();
    assert_eq!(json["files"][0]["first_line"], 30_000);
    assert_eq!(json["active"], 0);
    h.state_mut().close_viewer(id);
    h.run();
    assert!(h.state().viewers.iter().next().is_none());

    h.state_mut().load_session_from(&session);
    h.run();
    let id = h.state_mut().focused_viewer().expect("in the viewer again");
    wait_for_viewer(&mut h);
    assert_eq!(h.state().viewers.get(id).unwrap().state.top_line(), 30_000);
}

#[test]
fn the_file_menu_has_load_and_save_session() {
    let mut h = harness();
    h.get_by_label("File").click();
    h.run();
    h.get_by_label("Load Session…");
    h.get_by_label("Save Session…");
}
