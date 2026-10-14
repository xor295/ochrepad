//! More Notepad++ conveniences: the selection's size in the status bar,
//! Select and Find Next, Restore Recent Closed File, middle-click to close
//! a tab, Paste from History, Multi-select and Read-Only.

use egui::{Event, Key, Modifiers, PointerButton, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;

fn harness() -> Harness<'static, OchrepadApp> {
    // Tall enough for the whole Edit menu.
    let mut h = Harness::builder()
        .with_size(Vec2::new(900.0, 700.0))
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

fn text(h: &Harness<'_, OchrepadApp>) -> String {
    h.state().buffer().doc().to_string()
}

fn selected(h: &Harness<'_, OchrepadApp>) -> String {
    let app = h.state();
    let v = app.editor.view(app.active_view()).unwrap();
    app.buffer().doc().slice(v.selections.primary().range())
}

fn primary(h: &Harness<'_, OchrepadApp>) -> (usize, usize) {
    let app = h.state();
    let s = app
        .editor
        .view(app.active_view())
        .unwrap()
        .selections
        .primary()
        .clone();
    (s.start().0, s.end().0)
}

fn click(h: &mut Harness<'_, OchrepadApp>, label: &str) {
    h.get_by_label(label).click();
    h.run();
}

fn submenu(h: &mut Harness<'_, OchrepadApp>, menu: &str, submenu: &str) {
    click(h, menu);
    h.get_by_label_contains(submenu).hover();
    h.run();
}

#[test]
fn the_status_bar_shows_how_much_is_selected() {
    let mut h = harness();
    h.event(Event::Paste("héllo\nworld\nthree".into()));
    h.run();
    assert!(h.state().selection_status().is_none());
    key(&mut h, Modifiers::SHIFT, Key::ArrowLeft);
    h.get_by_label("1 selected");
    key(&mut h, Modifiers::COMMAND, Key::A);
    h.get_by_label("17 selected, 3 lines");
    // Whole lines end at the start of the next: that one doesn't count.
    key(&mut h, Modifiers::COMMAND, Key::Home);
    key(&mut h, Modifiers::SHIFT, Key::ArrowDown);
    h.get_by_label("6 selected");
}

#[test]
fn select_and_find_next_takes_the_word_at_the_caret() {
    let mut h = harness();
    h.event(Event::Paste("total = price * total;\nprint(total)".into()));
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::Home);
    key(&mut h, Modifiers::COMMAND, Key::F3);
    assert_eq!(selected(&h), "total");
    assert_eq!(primary(&h), (16, 21));
    // F3 goes on with it, without the find panel.
    key(&mut h, Modifiers::NONE, Key::F3);
    assert_eq!(primary(&h), (29, 34));
    assert!(!h.state().find.open);
    key(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::F3);
    assert_eq!(primary(&h), (16, 21));
}

#[test]
fn a_closed_file_comes_back_with_ctrl_shift_t() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    std::fs::write(&a, "a").unwrap();
    std::fs::write(&b, "b").unwrap();
    let mut h = harness();
    h.state_mut().open_path(&a);
    h.state_mut().open_path(&b);
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::W);
    assert_eq!(h.state().file_name(), "a.txt");
    key(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::T);
    assert_eq!(h.state().file_name(), "b.txt");
    // A middle click on a tab closes it.
    h.get_by_label("a.txt").click_button(PointerButton::Middle);
    h.run();
    assert!(h.query_by_label("a.txt").is_none());
    key(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::T);
    assert_eq!(h.state().file_name(), "a.txt");
}

#[test]
fn paste_from_history_offers_earlier_copies() {
    let mut h = harness();
    h.event(Event::Paste("one two".into()));
    h.run();
    key(
        &mut h,
        Modifiers::COMMAND | Modifiers::SHIFT,
        Key::ArrowLeft,
    );
    h.event(Event::Copy);
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::Home);
    key(
        &mut h,
        Modifiers::COMMAND | Modifiers::SHIFT,
        Key::ArrowRight,
    );
    h.event(Event::Copy);
    h.run();
    key(&mut h, Modifiers::NONE, Key::End);
    submenu(&mut h, "Edit", "Paste from History");
    click(&mut h, "two");
    assert_eq!(text(&h), "one twotwo");
}

#[test]
fn multi_select_next_from_the_edit_menu() {
    let mut h = harness();
    h.event(Event::Paste("cat Cat cat".into()));
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::Home);
    for _ in 0..3 {
        submenu(&mut h, "Edit", "Multi-select Next");
        click(&mut h, "Match Case & Whole Word");
    }
    h.event(Event::Text("dog".into()));
    h.run();
    assert_eq!(text(&h), "dog Cat dog");
}

#[test]
fn a_read_only_document_refuses_edits() {
    let mut h = harness();
    h.event(Event::Paste("keep".into()));
    h.run();
    click(&mut h, "Edit");
    click(&mut h, "Read-Only");
    assert!(h.state().buffer().read_only, "the menu item set it");
    key(&mut h, Modifiers::NONE, Key::Escape);
    h.event(Event::Text("x".into()));
    h.run();
    assert_eq!(text(&h), "keep");
    h.get_by_label("new 1 * (read-only)");
}
