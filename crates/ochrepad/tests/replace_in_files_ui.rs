//! Replace in Files: every match in a folder's files, after asking; open
//! documents get an undo step, the others are saved in their own encoding.

use std::time::Duration;

use egui::Vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;

fn harness() -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(1100.0, 500.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.run();
    h
}

/// Lets the find panel grow to its new rows: set from the test, they do not
/// ask for another frame, and egui sizes a panel from the frame before.
fn settle(h: &mut Harness<'_, OchrepadApp>) {
    for _ in 0..3 {
        h.step();
    }
    h.run();
}

fn wait_for(
    h: &mut Harness<'_, OchrepadApp>,
    done: impl Fn(&mut Harness<'_, OchrepadApp>) -> bool,
) {
    for _ in 0..500 {
        h.step();
        if done(h) {
            h.run();
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out");
}

#[test]
fn replace_in_files_rewrites_files_and_open_documents() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("a.txt"), "foo bar\nfoo\n").unwrap();
    // Windows-1252 with CR LF: both stay.
    std::fs::write(root.join("b.txt"), b"caf\xe9 foo\r\nfoo\r\n").unwrap();
    // Binary: left alone.
    std::fs::write(root.join("c.bin"), b"foo\0foo").unwrap();
    std::fs::create_dir(root.join("sub")).unwrap();
    std::fs::write(root.join("sub").join("d.txt"), "foo").unwrap();
    std::fs::write(root.join("none.txt"), "nothing here").unwrap();
    // Open, with changes not saved: its text in the editor is what counts.
    std::fs::write(root.join("open.txt"), "old").unwrap();

    let mut h = harness();
    h.state_mut().open_path(&root.join("open.txt"));
    h.run();
    let view = h.state().active_view();
    h.state_mut()
        .editor
        .apply(view, ochrepad_core::Action::InsertText("foo foo ".into()))
        .unwrap();
    {
        let app = h.state_mut();
        app.find.open = true;
        app.find.show_replace = true;
        app.find.show_files = true;
        app.find.pattern = "foo".into();
        app.find.replacement = "baz".into();
        app.find.folder = root.display().to_string();
        app.find.subfolders = true;
    }
    settle(&mut h);
    h.get_by_label("Replace in Files").click();
    wait_for(&mut h, |h| {
        h.query_by_label_contains("Replace 7 matches in 4 files")
            .is_some()
    });
    h.get_by_label_contains("3 files are saved as they are done");
    h.get_by_label_contains("1 open documents get an undo step");
    h.get_by_label("Replace All Matches").click();
    h.run();
    wait_for(&mut h, |h| !h.state().replacing_in_files());
    h.state_mut().flush_disk();
    h.run();

    let read = |name: &str| std::fs::read(root.join(name)).unwrap();
    assert_eq!(read("a.txt"), b"baz bar\nbaz\n");
    assert_eq!(read("b.txt"), b"caf\xe9 baz\r\nbaz\r\n");
    assert_eq!(read("c.bin"), b"foo\0foo");
    assert_eq!(read("sub/d.txt"), b"baz");
    assert_eq!(read("none.txt"), b"nothing here");
    // The open document changed in the editor, not on disk.
    assert_eq!(read("open.txt"), b"old");
    assert_eq!(h.state().buffer().doc().to_string(), "baz baz old");
    assert!(h.state().buffer().is_dirty());
    // In the find panel and the status bar.
    assert!(
        h.get_all_by_label_contains("Replaced 7 matches in 4 files (1 open, not saved yet)")
            .count()
            >= 1
    );
    // Undo brings the open document back.
    h.state_mut()
        .editor
        .apply(view, ochrepad_core::Action::Undo)
        .unwrap();
    assert_eq!(h.state().buffer().doc().to_string(), "foo foo old");
}

#[test]
fn cancelling_leaves_everything_alone() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "foo").unwrap();
    let mut h = harness();
    {
        let app = h.state_mut();
        app.find.open = true;
        app.find.show_replace = true;
        app.find.show_files = true;
        app.find.pattern = "foo".into();
        app.find.replacement = "baz".into();
        app.find.folder = dir.path().display().to_string();
    }
    settle(&mut h);
    h.get_by_label("Replace in Files").click();
    // One small file: counted at once, in the frame of the click.
    wait_for(&mut h, |h| {
        h.query_by_label_contains("Replace 1 matches").is_some()
    });
    h.get_by_label("Cancel").click();
    h.run();
    assert!(!h.state().replacing_in_files());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "foo"
    );
}

#[test]
fn files_that_would_change_beyond_the_matches_are_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    // Mixed line endings: saving would make them all alike.
    let mixed: &[u8] = b"a\r\nfoo\nb\r\nc\r\n";
    std::fs::write(root.join("mixed.txt"), mixed).unwrap();
    // Read-only on disk.
    let locked = root.join("locked.txt");
    std::fs::write(&locked, "foo").unwrap();
    let mut perms = std::fs::metadata(&locked).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&locked, perms).unwrap();
    // Changed after the count: written to between the question and the
    // answer.
    let changing = root.join("changing.txt");
    std::fs::write(&changing, "foo").unwrap();
    std::fs::write(root.join("plain.txt"), "foo").unwrap();

    let mut h = harness();
    {
        let app = h.state_mut();
        app.find.open = true;
        app.find.show_replace = true;
        app.find.show_files = true;
        app.find.pattern = "foo".into();
        app.find.replacement = "baz".into();
        app.find.folder = root.display().to_string();
    }
    settle(&mut h);
    h.get_by_label("Replace in Files").click();
    wait_for(&mut h, |h| {
        h.query_by_label_contains("Replace 2 matches in 2 files")
            .is_some()
    });
    h.get_by_label_contains("2 files with matches are left alone");
    h.get_by_label_contains("mixed line endings");
    h.get_by_label_contains("read-only on disk");
    // Make sure the modification time moves on, then change the file.
    std::thread::sleep(Duration::from_millis(50));
    std::fs::write(&changing, "foo foo").unwrap();
    h.get_by_label("Replace All Matches").click();
    h.run();
    wait_for(&mut h, |h| !h.state().replacing_in_files());
    h.state_mut().flush_disk();
    h.run();

    assert_eq!(std::fs::read(root.join("mixed.txt")).unwrap(), mixed);
    assert_eq!(std::fs::read_to_string(&locked).unwrap(), "foo");
    assert_eq!(std::fs::read_to_string(&changing).unwrap(), "foo foo");
    assert_eq!(
        std::fs::read_to_string(root.join("plain.txt")).unwrap(),
        "baz"
    );
    h.get_by_label_contains("changed since the matches were counted");
    let mut perms = std::fs::metadata(&locked).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&locked, perms).unwrap();
}
