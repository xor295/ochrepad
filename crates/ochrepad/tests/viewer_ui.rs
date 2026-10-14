//! 0.3.0 P7: files too large to edit open read-only in a viewer.

use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::{Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;
use ochrepad::app::Tab;

struct Dirs {
    tmp: tempfile::TempDir,
}

impl Dirs {
    fn new() -> Self {
        Self {
            tmp: tempfile::tempdir().unwrap(),
        }
    }
    fn file(&self, name: &str, text: &str) -> PathBuf {
        let p = self.tmp.path().join(name);
        std::fs::write(&p, text).unwrap();
        p
    }
    fn config(&self) -> PathBuf {
        self.tmp.path().join("config")
    }
    fn data(&self) -> PathBuf {
        self.tmp.path().join("data")
    }
}

fn app_with(app: OchrepadApp, size: Vec2) -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(size)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), app);
    h.run();
    h
}

fn harness() -> Harness<'static, OchrepadApp> {
    app_with(OchrepadApp::new(), Vec2::new(900.0, 500.0))
}

fn key(h: &mut Harness<'_, OchrepadApp>, m: Modifiers, k: Key) {
    h.key_press_modifiers(m, k);
    h.run();
}

/// Steps (in real time) until `done`, for the background index and
/// searches; about 10 s at most.
fn wait_for(
    h: &mut Harness<'_, OchrepadApp>,
    done: impl Fn(&mut Harness<'_, OchrepadApp>) -> bool,
) {
    for _ in 0..1000 {
        h.step();
        if done(h) {
            h.run();
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out");
}

/// A log of `lines` lines: "line N: ..." with an ERROR every 100th line.
fn log(lines: usize) -> String {
    (1..=lines)
        .map(|i| {
            let level = if i % 100 == 0 { "ERROR" } else { "INFO" };
            format!("2026-10-14 01:02:03 {level} line {i}: something happened\n")
        })
        .collect()
}

fn focused_tab(h: &mut Harness<'_, OchrepadApp>) -> Option<Tab> {
    h.state_mut().dock.find_active_focused().map(|(_, t)| *t)
}

fn viewer_ready(h: &mut Harness<'_, OchrepadApp>) -> bool {
    let id = match focused_tab(h) {
        Some(Tab::Viewer(id)) => id,
        _ => return false,
    };
    h.state()
        .viewers
        .get(id)
        .is_some_and(|v| v.file.progress().done)
}

fn open_viewer(h: &mut Harness<'_, OchrepadApp>, path: &Path) {
    h.state_mut().open_viewer(path).expect("opens");
    h.run();
    wait_for(h, viewer_ready);
}

#[test]
fn files_over_the_threshold_open_read_only_in_a_viewer() {
    let d = Dirs::new();
    let text = log(30_000); // about 1.7 MB
    let path = d.file("big.log", &text);
    let small = d.file("small.log", "small\n");
    let mut h = harness();
    h.state_mut().settings.viewer_threshold_mb = 1;
    h.state_mut().open_path(&small);
    h.run();
    assert!(matches!(focused_tab(&mut h), Some(Tab::Editor(_))));
    h.state_mut().open_path(&path);
    h.run();
    assert!(matches!(focused_tab(&mut h), Some(Tab::Viewer(_))));
    wait_for(&mut h, viewer_ready);
    h.get_by_label("Viewer: big.log");
    h.get_by_label("big.log (read-only)");
    h.get_by_label("30001 lines");
    h.get_by_label_contains("Read-only");
    h.get_by_label("Ln 1, Col 1");
    // Opening it again switches to it.
    h.state_mut().open_path(&path);
    h.run();
    let viewers = h
        .state()
        .dock
        .iter_all_tabs()
        .filter(|(_, t)| matches!(t, Tab::Viewer(_)))
        .count();
    assert_eq!(viewers, 1);
    // Saving is not possible.
    key(&mut h, Modifiers::COMMAND, Key::S);
    h.get_by_label_contains("open read-only");
    // Ctrl+W closes it.
    key(&mut h, Modifiers::COMMAND, Key::W);
    assert!(h.state().viewers.iter().next().is_none());
}

#[test]
fn keys_and_go_to_line_move_the_caret() {
    let d = Dirs::new();
    let path = d.file("lines.log", &log(5000));
    let mut h = harness();
    open_viewer(&mut h, &path);
    key(&mut h, Modifiers::COMMAND, Key::G);
    h.event(Event::Text("1234".into()));
    key(&mut h, Modifiers::NONE, Key::Enter);
    wait_for(&mut h, |h| h.query_by_label("Ln 1234, Col 1").is_some());
    key(&mut h, Modifiers::NONE, Key::ArrowDown);
    h.get_by_label("Ln 1235, Col 1");
    key(&mut h, Modifiers::NONE, Key::End);
    let len = "2026-10-14 01:02:03 INFO line 1235: something happened".len();
    h.get_by_label(&format!("Ln 1235, Col {}", len + 1));
    key(&mut h, Modifiers::COMMAND, Key::End);
    h.get_by_label("Ln 5001, Col 1");
    key(&mut h, Modifiers::COMMAND, Key::Home);
    h.get_by_label("Ln 1, Col 1");
    // A line past the end goes to the last one.
    key(&mut h, Modifiers::COMMAND, Key::G);
    h.event(Event::Text("99999".into()));
    key(&mut h, Modifiers::NONE, Key::Enter);
    wait_for(&mut h, |h| h.query_by_label("Ln 5001, Col 1").is_some());
}

#[test]
fn find_next_previous_and_count_run_in_the_background() {
    let d = Dirs::new();
    let path = d.file("find.log", &log(5000));
    let mut h = harness();
    open_viewer(&mut h, &path);
    key(&mut h, Modifiers::COMMAND, Key::F);
    h.event(Event::Text("ERROR".into()));
    h.run();
    key(&mut h, Modifiers::NONE, Key::Enter);
    wait_for(&mut h, |h| h.query_by_label("Ln 100, Col 26").is_some());
    key(&mut h, Modifiers::NONE, Key::F3);
    wait_for(&mut h, |h| h.query_by_label("Ln 200, Col 26").is_some());
    key(&mut h, Modifiers::SHIFT, Key::F3);
    wait_for(&mut h, |h| h.query_by_label("Ln 100, Col 26").is_some());
    h.get_by_label("Count").click();
    wait_for(&mut h, |h| h.query_by_label("50 matches").is_some());
    // Wrapping around from the last match.
    let id = match focused_tab(&mut h) {
        Some(Tab::Viewer(id)) => id,
        other => panic!("{other:?}"),
    };
    h.state_mut()
        .viewers
        .get_mut(id)
        .unwrap()
        .state
        .go_to_line(4999, 1);
    h.run();
    wait_for(&mut h, |h| h.query_by_label("Ln 4999, Col 1").is_some());
    key(&mut h, Modifiers::NONE, Key::F3);
    wait_for(&mut h, |h| h.query_by_label("Ln 5000, Col 26").is_some());
    key(&mut h, Modifiers::NONE, Key::F3);
    wait_for(&mut h, |h| h.query_by_label("Ln 100, Col 26").is_some());
    h.get_by_label("Wrapped around");
}

#[test]
fn find_all_lists_the_matches_and_goes_to_them() {
    let d = Dirs::new();
    let path = d.file("all.log", &log(1000));
    let mut h = harness();
    open_viewer(&mut h, &path);
    key(&mut h, Modifiers::COMMAND, Key::F);
    h.event(Event::Text("ERROR".into()));
    h.run();
    h.get_by_label("Find All").click();
    h.run();
    h.get_by_label("In Current Document").click();
    wait_for(&mut h, |h| {
        h.state()
            .results
            .searches
            .front()
            .is_some_and(|s| s.done && s.matches == 10)
    });
    let hit = h.get_by_label_contains("Line 500:");
    hit.click();
    wait_for(&mut h, |h| h.query_by_label("Ln 500, Col 26").is_some());
}

#[test]
fn copy_takes_the_selection_with_lf_line_breaks() {
    let d = Dirs::new();
    let path = d.file("copy.log", "first line\r\nsecond line\r\nthird\r\n");
    let mut h = harness();
    open_viewer(&mut h, &path);
    key(&mut h, Modifiers::SHIFT, Key::ArrowDown);
    key(&mut h, Modifiers::SHIFT, Key::End);
    h.event(Event::Copy);
    h.step();
    let copied: Vec<String> = h
        .output()
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(copied, vec!["first line\nsecond line".to_owned()]);
}

#[test]
fn open_as_editable_asks_then_opens_an_editor() {
    let d = Dirs::new();
    let path = d.file("edit.log", "editable text\n");
    let mut h = harness();
    open_viewer(&mut h, &path);
    h.get_by_label("View").click();
    h.run();
    h.get_by_label("Open as Editable…").click();
    h.run();
    h.get_by_label_contains("Open it for editing?");
    h.get_by_label("Open for Editing").click();
    h.run();
    assert!(matches!(focused_tab(&mut h), Some(Tab::Editor(_))));
    assert_eq!(h.state().buffer().doc().to_string(), "editable text\n");
    assert!(h.state().viewers.iter().next().is_none());
}

#[test]
fn the_session_reopens_viewers_where_they_were() {
    let d = Dirs::new();
    let path = d.file("session.log", &log(3000));
    let mut app = OchrepadApp::new();
    app.set_storage(&d.config(), &d.data());
    let mut h = app_with(app, Vec2::new(900.0, 500.0));
    open_viewer(&mut h, &path);
    let id = match focused_tab(&mut h) {
        Some(Tab::Viewer(id)) => id,
        other => panic!("{other:?}"),
    };
    h.state_mut()
        .viewers
        .get_mut(id)
        .unwrap()
        .state
        .scroll_to_line(1500);
    h.run();
    let session = h.state().session();
    assert_eq!(session.viewers.len(), 1);
    assert_eq!(session.viewers[0].top_line, 1500);
    // Exit writes the session; a new start brings the viewer back.
    h.state_mut().request_exit();
    h.run();
    h.state_mut().flush_disk();
    drop(h);
    let mut app = OchrepadApp::new();
    app.set_storage(&d.config(), &d.data());
    let mut h = app_with(app, Vec2::new(900.0, 500.0));
    let (id, viewer) = h.state().viewers.iter().next().expect("restored");
    assert_eq!(viewer.state.top_line(), 1500);
    assert!(h.state().dock.find_tab(&Tab::Viewer(id)).is_some());
    h.run();
}

#[test]
fn utf16_files_are_refused_with_a_message() {
    let d = Dirs::new();
    let path = d.tmp.path().join("wide.log");
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend("hello\n".encode_utf16().flat_map(u16::to_le_bytes));
    std::fs::write(&path, bytes).unwrap();
    let mut h = harness();
    assert!(h.state_mut().open_viewer(&path).is_none());
    h.run();
    assert!(h.state().message().unwrap().contains("UTF-16"));
}

#[test]
fn the_viewer_looks_right() {
    let d = Dirs::new();
    let text = format!(
        "a very long line: {}\nbell \u{7} and tab\there\n{}",
        "0123456789".repeat(1500),
        log(40)
    );
    let path = d.file("look.log", &text);
    let mut h = Harness::builder()
        .with_size(Vec2::new(640.0, 300.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .wgpu()
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.run();
    open_viewer(&mut h, &path);
    // Select part of the third line.
    let id = match focused_tab(&mut h) {
        Some(Tab::Viewer(id)) => id,
        other => panic!("{other:?}"),
    };
    {
        let v = h.state_mut().viewers.get_mut(id).unwrap();
        v.state.anchor = ochrepad::viewer::Pos { line: 2, col: 20 };
        v.state.head = ochrepad::viewer::Pos { line: 2, col: 25 };
    }
    h.run();
    h.snapshot("viewer");
}
