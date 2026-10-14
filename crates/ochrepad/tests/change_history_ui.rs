//! The change history: the gutter marks the lines changed since the file
//! was saved.

mod util;

use std::time::Duration;

use egui::{Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use ochrepad::OchrepadApp;
use ochrepad::change_history::{LineMarks, Mark};

fn harness() -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(700.0, 300.0))
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

/// Steps in real time until `done` (the marks are worked out on a thread).
fn wait_for(h: &mut Harness<'_, OchrepadApp>, done: impl Fn(&OchrepadApp) -> bool) {
    for _ in 0..500 {
        h.step();
        if done(h.state()) {
            h.run();
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out");
}

fn marks(app: &OchrepadApp) -> Option<LineMarks> {
    app.changes.marks(app.buffer_id()).cloned()
}

const TEXT: &str =
    "fn main() {\n    let a = 1;\n    let b = 2;\n    let c = 3;\n    println!(\"{a}\");\n}\n";

fn open(h: &mut Harness<'_, OchrepadApp>, dir: &tempfile::TempDir) -> std::path::PathBuf {
    let file = dir.path().join("main.rs");
    std::fs::write(&file, TEXT).unwrap();
    h.state_mut().open_path(&file);
    h.run();
    file
}

/// Changes line 2, deletes line 3 and adds a line after line 5.
fn edit(h: &mut Harness<'_, OchrepadApp>) {
    let view = h.state().active_view();
    let ed = &mut h.state_mut().editor;
    for action in [
        ochrepad_core::Action::GoToLine {
            line: 2,
            column: 13,
        },
        ochrepad_core::Action::InsertText("0".into()),
        ochrepad_core::Action::GoToLine { line: 3, column: 1 },
        ochrepad_core::Action::DeleteLine,
        ochrepad_core::Action::GoToLine {
            line: 4,
            column: 22,
        },
        ochrepad_core::Action::InsertNewline,
        ochrepad_core::Action::InsertText("let d = 4;".into()),
    ] {
        ed.apply(view, action).unwrap();
    }
    h.run();
}

#[test]
fn edited_lines_are_marked_until_saved_or_undone() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness();
    let file = open(&mut h, &dir);
    assert!(marks(h.state()).is_none(), "a file just opened has none");
    edit(&mut h);
    wait_for(&mut h, |a| marks(a).is_some_and(|m| m.lines.len() == 2));
    let m = marks(h.state()).unwrap();
    assert_eq!(m.lines, vec![(1, Mark::Changed), (4, Mark::Added)]);
    assert_eq!(m.deleted_before, vec![2]);

    // Undoing everything clears them.
    while h.state().buffer().history().can_undo() {
        key(&mut h, Modifiers::COMMAND, Key::Z);
    }
    assert_eq!(h.state().buffer().doc().to_string(), TEXT);
    wait_for(&mut h, |a| marks(a).is_none());

    // Saving makes the saved text the one compared with.
    edit(&mut h);
    wait_for(&mut h, |a| marks(a).is_some());
    key(&mut h, Modifiers::COMMAND, Key::S);
    h.state_mut().flush_disk();
    h.run();
    assert!(!h.state().buffer().is_dirty());
    wait_for(&mut h, |a| marks(a).is_none());
    h.event(Event::Text("x".into()));
    h.run();
    wait_for(&mut h, |a| marks(a).is_some_and(|m| m.lines.len() == 1));
    assert_ne!(std::fs::read_to_string(&file).unwrap(), TEXT);
}

#[test]
fn view_change_history_turns_them_off() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness();
    h.state_mut().settings.change_history = false;
    open(&mut h, &dir);
    edit(&mut h);
    for _ in 0..30 {
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        marks(h.state()).is_none(),
        "nothing is worked out while off"
    );
    h.state_mut().settings.change_history = true;
    wait_for(&mut h, |a| marks(a).is_some());
}

#[test]
fn change_marks_look_right() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness();
    open(&mut h, &dir);
    edit(&mut h);
    wait_for(&mut h, |a| marks(a).is_some_and(|m| m.lines.len() == 2));
    util::snapshot(&mut h, "change_history");
}
