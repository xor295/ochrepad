//! The folder workspace: the sidebar's file tree, opening and renaming
//! files from it, and watching for changes.

mod util;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;

/// The sidebar is 240 px wide; tree rows are the nodes inside it.
const SIDEBAR_W: f32 = 240.0;

fn harness(root: &Path, size: Vec2) -> Harness<'static, OchrepadApp> {
    let mut app = OchrepadApp::new();
    app.open_folder(root);
    let mut h = Harness::builder()
        .with_size(size)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .wgpu()
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), app);
    h.run();
    h
}

/// Steps the app until `done` (the lister and watcher work in real time).
fn wait_for(
    h: &mut Harness<'_, OchrepadApp>,
    what: &str,
    done: impl Fn(&Harness<'_, OchrepadApp>) -> bool,
) {
    let start = Instant::now();
    while !done(h) {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
        h.step();
    }
    h.run();
}

/// The tree row named `name` (a tab may have the same name).
fn row(h: &Harness<'_, OchrepadApp>, name: &str) -> Option<Rect> {
    h.query_all_by_label(name)
        .map(|n| n.rect())
        .find(|r| r.min.x < SIDEBAR_W)
}

fn has_row(h: &Harness<'_, OchrepadApp>, name: &str) -> bool {
    row(h, name).is_some()
}

fn row_pos(h: &Harness<'_, OchrepadApp>, name: &str) -> Pos2 {
    row(h, name).expect("row is shown").center()
}

/// A small project in `<tmp>/demo` (a fixed name, for the snapshot):
/// src/ with two files, a README, and a target/ that .gitignore hides.
fn project() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("demo");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("src").join("main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(root.join("src").join("lib.rs"), "pub fn x() {}\n").unwrap();
    std::fs::write(root.join("README.md"), "# Readme\n").unwrap();
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    (tmp, root)
}

/// Clicks `times` times at `pos`, half a second after anything before, so
/// that clicks on different rows never count as one double-click.
fn click(h: &mut Harness<'_, OchrepadApp>, pos: Pos2, times: usize) {
    for _ in 0..30 {
        h.step();
    }
    h.event(Event::PointerMoved(pos));
    for _ in 0..times {
        for pressed in [true, false] {
            h.event(Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
            h.step();
        }
    }
    h.run();
}

#[test]
fn the_tree_lists_expands_and_opens_files() {
    let (_tmp, root) = project();
    let mut h = harness(&root, Vec2::new(900.0, 500.0));
    wait_for(&mut h, "the root listing", |h| has_row(h, "README.md"));
    assert!(has_row(&h, "src"));
    assert!(!has_row(&h, "target"), "ignored by .gitignore");

    let p = row_pos(&h, "src");
    click(&mut h, p, 1);
    wait_for(&mut h, "src's listing", |h| has_row(h, "main.rs"));
    let p = row_pos(&h, "main.rs");
    click(&mut h, p, 2);
    assert_eq!(h.state().file_name(), "main.rs");
    assert_eq!(h.state().buffer().doc().to_string(), "fn main() {}\n");
}

#[test]
fn renaming_an_open_file_moves_its_tab_with_it() {
    let (_tmp, root) = project();
    let mut h = harness(&root, Vec2::new(900.0, 500.0));
    wait_for(&mut h, "the root listing", |h| has_row(h, "README.md"));
    let p = row_pos(&h, "README.md");
    click(&mut h, p, 2);
    assert_eq!(h.state().file_name(), "README.md");

    // F2 renames the selected row in place.
    let p = row_pos(&h, "README.md");
    click(&mut h, p, 1);
    h.key_press(Key::F2);
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Text("NOTES.md".into()));
    h.key_press(Key::Enter);
    h.run();
    let new = root.join("NOTES.md");
    wait_for(&mut h, "the rename", |h| {
        h.state().buffer().meta.path.as_deref() == Some(new.as_path())
    });
    assert!(new.is_file() && !root.join("README.md").exists());
    assert_eq!(h.state().file_name(), "NOTES.md");
    wait_for(&mut h, "the new row", |h| has_row(h, "NOTES.md"));
}

#[test]
fn new_file_creates_and_opens_it() {
    let (_tmp, root) = project();
    let mut h = harness(&root, Vec2::new(900.0, 500.0));
    wait_for(&mut h, "the root listing", |h| has_row(h, "README.md"));
    h.get_by_label("Folder").click();
    h.run();
    h.get_by_label("New File…").click();
    h.run();
    h.event(Event::Text("todo.txt".into()));
    h.key_press(Key::Enter);
    h.run();
    let path = root.join("todo.txt");
    wait_for(&mut h, "the new file", |h| {
        h.state().file_name() == "todo.txt"
    });
    assert!(path.is_file());
    wait_for(&mut h, "its row", |h| has_row(h, "todo.txt"));
}

#[test]
fn files_created_elsewhere_appear() {
    let (_tmp, root) = project();
    let mut h = harness(&root, Vec2::new(900.0, 500.0));
    wait_for(&mut h, "the root listing", |h| has_row(h, "README.md"));
    std::fs::write(root.join("later.txt"), "").unwrap();
    wait_for(&mut h, "the watcher", |h| has_row(h, "later.txt"));
}

#[test]
fn the_sidebar_looks_right() {
    let (_tmp, root) = project();
    let mut h = harness(&root, Vec2::new(760.0, 360.0));
    wait_for(&mut h, "the root listing", |h| has_row(h, "README.md"));
    // Without the status bar's note about indexing for completion.
    wait_for(&mut h, "the folder's index", |h| {
        h.state()
            .completion
            .folder()
            .is_some_and(|f| f.progress == ochrepad::folder_index::Progress::Ready)
    });
    let p = row_pos(&h, "src");
    click(&mut h, p, 1);
    wait_for(&mut h, "src's listing", |h| has_row(h, "main.rs"));
    let p = row_pos(&h, "lib.rs");
    click(&mut h, p, 2);
    h.event(Event::PointerMoved(Pos2::new(700.0, 300.0)));
    h.run();
    util::snapshot(&mut h, "sidebar");
}
