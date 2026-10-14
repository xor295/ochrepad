//! Find in Files: searching a folder on disk from the find panel.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;
use ochrepad::search_results::Source;

fn harness() -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(1000.0, 700.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.run();
    h
}

/// A folder with text in two encodings, a binary file, an ignored folder,
/// a file too large to search and a file without the word.
fn folder() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("proj");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::create_dir_all(root.join("build")).unwrap();
    std::fs::write(root.join("a.txt"), "one needle\ntwo\n").unwrap();
    // "café needle" in Windows-1252.
    std::fs::write(root.join("sub").join("latin.txt"), b"caf\xe9 needle\n").unwrap();
    std::fs::write(root.join("bin.dat"), b"needle\0\0\0").unwrap();
    std::fs::write(root.join("build").join("out.txt"), "needle\n").unwrap();
    std::fs::write(root.join(".gitignore"), "build/\n").unwrap();
    std::fs::write(root.join("none.txt"), "nothing\n").unwrap();
    // Sparse, so the test stays quick.
    let big = std::fs::File::create(root.join("huge.log")).unwrap();
    big.set_len(51 * 1024 * 1024).unwrap();
    (tmp, root)
}

fn wait_done(h: &mut Harness<'_, OchrepadApp>) {
    let start = Instant::now();
    while h.state().results.searches.front().is_none_or(|s| !s.done) {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "search did not finish"
        );
        std::thread::sleep(Duration::from_millis(10));
        h.step();
    }
    h.run();
}

fn find_in(h: &mut Harness<'_, OchrepadApp>, root: &Path, pattern: &str) {
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::F);
    h.run();
    h.event(Event::Text(pattern.to_owned()));
    h.run();
    h.state_mut().find.folder = root.display().to_string();
    h.run();
    h.get_by_label("Find in Files").click();
    h.run();
    wait_done(h);
}

#[test]
fn find_in_files_reads_encodings_skips_binary_ignored_and_huge_files() {
    let (_tmp, root) = folder();
    let mut h = harness();
    // a.txt is open with an unsaved edit: the buffer's text is searched.
    h.state_mut().open_path(&root.join("a.txt"));
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::End);
    h.event(Event::Text("needle".into()));
    h.run();
    find_in(&mut h, &root, "needle");

    let s = h.state().results.searches.front().unwrap();
    let files: Vec<(String, usize, bool)> = s
        .files
        .iter()
        .map(|f| {
            (
                f.name.replace('\\', "/"),
                f.matches,
                matches!(f.source, Source::Buffer(_)),
            )
        })
        .collect();
    assert_eq!(
        files,
        vec![
            ("a.txt".into(), 2, true),
            ("sub/latin.txt".into(), 1, false)
        ]
    );
    assert_eq!(s.files[1].hits[0].text, "café needle");
    assert_eq!(s.skipped_large, 1);
    // .gitignore, a.txt, bin.dat, huge.log, none.txt, sub/latin.txt.
    assert_eq!(s.files_searched, Some(6));
    h.get_by_label_contains("1 file over 50 MB skipped");
}

#[test]
fn a_result_in_a_closed_file_opens_it() {
    let (_tmp, root) = folder();
    let mut h = harness();
    find_in(&mut h, &root, "needle");
    h.get_by_label_contains("café needle").click();
    h.run();
    assert_eq!(h.state().file_name(), "latin.txt");
    let app = h.state();
    let sel = app
        .editor
        .view(app.active_view())
        .unwrap()
        .selections
        .primary()
        .clone();
    let doc = app.buffer().doc();
    assert_eq!(doc.slice(sel.range()), "needle");
}

#[test]
fn filters_and_a_bad_folder() {
    let (_tmp, root) = folder();
    let mut h = harness();
    h.state_mut().find.filters = "*.log; none.*".into();
    find_in(&mut h, &root, "nothing");
    let s = h.state().results.searches.front().unwrap();
    assert_eq!(s.files.len(), 1);
    assert_eq!(s.files_searched, Some(2));

    h.state_mut().find.folder = root.join("missing").display().to_string();
    h.get_by_label("Find in Files").click();
    h.run();
    assert!(
        h.state()
            .find
            .message()
            .unwrap()
            .contains("is not a folder")
    );
}

#[test]
fn stopping_keeps_what_was_found() {
    let tmp = tempfile::tempdir().unwrap();
    for i in 0..2000 {
        std::fs::write(tmp.path().join(format!("f{i}.txt")), "needle\n").unwrap();
    }
    let mut h = harness();
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::F);
    h.event(Event::Text("needle".into()));
    h.run();
    h.state_mut().find.folder = tmp.path().display().to_string();
    h.state_mut().find_in_files();
    h.state_mut().results.stop();
    let s = h.state().results.searches.front().unwrap();
    assert!(s.done && s.stopped);
    h.run();
}
