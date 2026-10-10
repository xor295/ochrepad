//! M5: tabs, split views, and closing with save prompts.

use egui::{Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;
use ochrepad_core::{Action, ByteOffset, SelectionSet, ViewId};

fn harness() -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(900.0, 600.0))
        .with_step_dt(1.0 / 60.0)
        // Background highlighting runs on a wall-clock budget; let `run`
        // wait for it even on a loaded machine.
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
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

fn names(h: &Harness<'_, OchrepadApp>) -> Vec<String> {
    let app = h.state();
    app.tabs()
        .into_iter()
        .map(|v| app.buffer_name(app.editor.view(v).unwrap().buffer()))
        .collect()
}

fn active_name(h: &Harness<'_, OchrepadApp>) -> String {
    h.state().file_name()
}

fn text_of(h: &Harness<'_, OchrepadApp>, view: ViewId) -> String {
    let app = h.state();
    let b = app.editor.view(view).unwrap().buffer();
    app.editor.buffer(b).unwrap().doc().to_string()
}

fn write_files(names: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (n, t) in names {
        std::fs::write(dir.path().join(n), t).unwrap();
    }
    dir
}

#[test]
fn new_file_opens_a_new_tab_and_takes_the_keyboard() {
    let mut h = harness();
    typ(&mut h, "one");
    key(&mut h, Modifiers::COMMAND, Key::N);
    assert_eq!(names(&h), vec!["new 1", "new 2"]);
    assert_eq!(active_name(&h), "new 2");
    typ(&mut h, "two");
    let tabs = h.state().tabs();
    assert_eq!(text_of(&h, tabs[0]), "one");
    assert_eq!(text_of(&h, tabs[1]), "two");
    // The dirty tab shows a marker.
    h.get_by_label("new 1 *");
}

#[test]
fn opening_files_replaces_the_untouched_new_tab_and_reuses_open_ones() {
    let dir = write_files(&[("a.txt", "alpha"), ("b.txt", "beta")]);
    let mut h = harness();
    h.state_mut().open_path(&dir.path().join("a.txt"));
    h.state_mut().open_path(&dir.path().join("b.txt"));
    h.run();
    assert_eq!(names(&h), vec!["a.txt", "b.txt"]);
    assert_eq!(active_name(&h), "b.txt");
    h.state_mut().open_path(&dir.path().join("a.txt"));
    h.run();
    assert_eq!(names(&h), vec!["a.txt", "b.txt"], "no duplicate tab");
    assert_eq!(active_name(&h), "a.txt");
}

#[test]
fn ctrl_tab_cycles_through_the_tabs_of_the_pane() {
    let mut h = harness();
    key(&mut h, Modifiers::COMMAND, Key::N);
    key(&mut h, Modifiers::COMMAND, Key::N);
    assert_eq!(active_name(&h), "new 3");
    key(&mut h, Modifiers::COMMAND, Key::Tab);
    assert_eq!(active_name(&h), "new 1");
    key(&mut h, Modifiers::COMMAND, Key::Tab);
    assert_eq!(active_name(&h), "new 2");
    key(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::Tab);
    key(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::Tab);
    assert_eq!(active_name(&h), "new 3");
    typ(&mut h, "x");
    assert_eq!(text_of(&h, h.state().active_view()), "x");
}

#[test]
fn clone_to_other_view_splits_and_shares_the_buffer() {
    let mut h = harness();
    typ(&mut h, "hello world");
    let original = h.state().active_view();
    let clone = h.state_mut().clone_to_other_view();
    h.run();
    assert_ne!(clone, original);
    assert_eq!(
        h.state().dock.iter_leaves().count(),
        2,
        "split into two panes"
    );
    assert_eq!(h.state().active_view(), clone);
    // Both editors are on screen.
    assert_eq!(h.get_all_by_label("Editor: new 1").count(), 2);

    // Put the clone's caret at the start and type: the edit appears in the
    // shared buffer, and the original view's caret moves with its text.
    let app = h.state_mut();
    app.editor
        .apply(
            clone,
            Action::SetSelections(SelectionSet::caret(ByteOffset(0))),
        )
        .unwrap();
    typ(&mut h, ">> ");
    assert_eq!(text_of(&h, original), ">> hello world");
    let app = h.state();
    let caret = |v| app.editor.view(v).unwrap().selections.primary().head.0;
    assert_eq!(caret(clone), 3);
    assert_eq!(caret(original), 14, "original caret stays after 'world'");

    // Closing one of the two views of a dirty buffer does not ask.
    key(&mut h, Modifiers::COMMAND, Key::W);
    assert!(h.state().confirming().is_none());
    assert_eq!(h.state().tabs(), vec![original]);
}

#[test]
fn closing_a_dirty_tab_asks_and_cancel_keeps_it() {
    let mut h = harness();
    typ(&mut h, "draft");
    key(&mut h, Modifiers::COMMAND, Key::W);
    h.get_by_label("Save changes to new 1?");
    h.get_by_label("Cancel").click();
    h.run();
    assert_eq!(names(&h), vec!["new 1"]);
    assert_eq!(text_of(&h, h.state().active_view()), "draft");

    key(&mut h, Modifiers::COMMAND, Key::W);
    h.get_by_label("Don't Save").click();
    h.run();
    // Closing the last tab leaves a fresh empty one, as in Notepad++.
    assert_eq!(names(&h), vec!["new 2"]);
    assert!(!h.state().buffer().is_dirty());
}

#[test]
fn save_in_the_close_prompt_writes_the_file() {
    let dir = write_files(&[("a.txt", "alpha")]);
    let mut h = harness();
    h.state_mut().open_path(&dir.path().join("a.txt"));
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::End);
    typ(&mut h, "!");
    key(&mut h, Modifiers::COMMAND, Key::W);
    h.get_by_label("Save changes to a.txt?");
    h.get_by_label("Save").click();
    h.run();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "alpha!"
    );
    assert!(!names(&h).contains(&"a.txt".to_owned()));
}

#[test]
fn close_others_asks_about_each_dirty_tab() {
    let mut h = harness();
    typ(&mut h, "dirty one");
    key(&mut h, Modifiers::COMMAND, Key::N);
    key(&mut h, Modifiers::COMMAND, Key::N);
    typ(&mut h, "keep me");
    let keep = h.state().active_view();
    h.state_mut().close_others(keep);
    h.run();
    h.get_by_label("Save changes to new 1?");
    h.get_by_label("Don't Save").click();
    h.run();
    assert_eq!(h.state().tabs(), vec![keep]);
}

#[test]
fn cancel_during_close_all_stops_closing() {
    let mut h = harness();
    typ(&mut h, "a");
    key(&mut h, Modifiers::COMMAND, Key::N);
    typ(&mut h, "b");
    h.state_mut().close_all();
    h.run();
    h.get_by_label("Save changes to new 1?");
    h.get_by_label("Cancel").click();
    h.run();
    assert_eq!(names(&h), vec!["new 1", "new 2"]);
    assert!(h.state().confirming().is_none());
}

#[test]
fn exit_asks_about_unsaved_buffers() {
    let mut h = harness();
    h.state_mut().request_exit();
    h.run();
    assert!(h.state().may_close(), "nothing to save: exit at once");

    let mut h = harness();
    typ(&mut h, "unsaved");
    h.state_mut().request_exit();
    h.run();
    assert!(!h.state().may_close());
    h.get_by_label("Save changes to new 1?");
    h.get_by_label("Don't Save").click();
    h.run();
    assert!(h.state().may_close());
}

#[test]
fn each_view_keeps_its_own_scroll_position() {
    let lines: Vec<String> = (0..500).map(|i| format!("line {i}")).collect();
    let mut h = harness();
    h.event(Event::Paste(lines.join("\n")));
    h.run();
    let original = h.state().active_view();
    let clone = h.state_mut().clone_to_other_view();
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::Home);
    let app = h.state();
    let first = |v| app.editor.view(v).unwrap().scroll.first_line;
    assert_eq!(first(clone), 0);
    assert!(first(original) > 400, "original still shows the end");
}

fn double_click(h: &mut Harness<'_, OchrepadApp>, pos: egui::Pos2) {
    h.event(Event::PointerMoved(pos));
    for _ in 0..2 {
        for pressed in [true, false] {
            h.event(Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
            h.step();
        }
    }
    h.run();
}

#[test]
fn double_clicking_the_empty_tab_bar_opens_a_new_tab() {
    let mut h = harness();
    let tab = h.get_by_label("new 1").rect();
    // On the tab itself: nothing new.
    double_click(&mut h, tab.center());
    assert_eq!(names(&h), vec!["new 1"]);
    // Let egui's multi-click window pass so the next pair is a fresh double-click.
    h.run_steps(60);
    // On the empty bar to its right: a new tab, which takes the keyboard.
    double_click(&mut h, egui::pos2(tab.max.x + 150.0, tab.center().y));
    assert_eq!(names(&h), vec!["new 1", "new 2"]);
    assert_eq!(active_name(&h), "new 2");
    typ(&mut h, "x");
    assert_eq!(text_of(&h, h.state().active_view()), "x");
}

#[test]
fn double_clicking_a_panes_tab_bar_adds_the_tab_to_that_pane() {
    let mut h = harness();
    let left = h.state().active_view();
    h.state_mut().clone_to_other_view();
    h.run();
    // Double-click the left pane's empty tab bar.
    let left_tab = h
        .get_all_by_label("new 1")
        .map(|n| n.rect())
        .min_by(|a, b| a.min.x.total_cmp(&b.min.x))
        .unwrap();
    double_click(
        &mut h,
        egui::pos2(left_tab.max.x + 60.0, left_tab.center().y),
    );
    let new = h.state().active_view();
    let app = h.state();
    let pane_of = |v| {
        app.dock
            .find_tab(&ochrepad::app::Tab::Editor(v))
            .map(|p| p.node)
    };
    assert_eq!(pane_of(new), pane_of(left), "new tab is in the left pane");
    assert_eq!(app.buffer_name(app.buffer_id()), "new 2");
}
