//! M7: the find/replace panel.

use egui::{Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;

fn with_text(text: &str) -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(1000.0, 600.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.run();
    h.event(Event::Paste(text.to_owned()));
    h.run();
    h
}

fn text(h: &Harness<'_, OchrepadApp>) -> String {
    h.state().buffer().doc().to_string()
}

fn sels(h: &Harness<'_, OchrepadApp>) -> Vec<(usize, usize)> {
    let app = h.state();
    app.editor
        .view(app.active_view())
        .unwrap()
        .selections
        .ranges()
        .iter()
        .map(|s| (s.anchor.0, s.head.0))
        .collect()
}

fn key(h: &mut Harness<'_, OchrepadApp>, m: Modifiers, k: Key) {
    h.key_press_modifiers(m, k);
    h.run();
}

fn typ(h: &mut Harness<'_, OchrepadApp>, s: &str) {
    h.event(Event::Text(s.to_owned()));
    h.run();
}

fn click(h: &mut Harness<'_, OchrepadApp>, label: &str) {
    h.get_by_label(label).click();
    h.run();
}

/// Opens the panel with Ctrl+F and types a pattern into the focused field.
fn find_for(h: &mut Harness<'_, OchrepadApp>, pattern: &str) {
    key(h, Modifiers::COMMAND, Key::F);
    assert!(h.state().find.open);
    typ(h, pattern);
}

#[test]
fn enter_f3_and_shift_f3_move_between_matches() {
    let mut h = with_text("cat dog cat bird cat");
    key(&mut h, Modifiers::COMMAND, Key::Home);
    find_for(&mut h, "cat");
    key(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(sels(&h), vec![(0, 3)]);
    // Enter keeps the field focused, so typing Enter again finds the next.
    key(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(sels(&h), vec![(8, 11)]);
    key(&mut h, Modifiers::NONE, Key::F3);
    assert_eq!(sels(&h), vec![(17, 20)]);
    key(&mut h, Modifiers::NONE, Key::F3);
    assert_eq!(sels(&h), vec![(0, 3)], "wraps by default");
    h.get_by_label("Wrapped to the start");
    key(&mut h, Modifiers::SHIFT, Key::F3);
    assert_eq!(sels(&h), vec![(17, 20)]);
}

#[test]
fn not_found_and_invalid_patterns_are_reported_inline() {
    let mut h = with_text("abc");
    find_for(&mut h, "zzz");
    key(&mut h, Modifiers::NONE, Key::Enter);
    h.get_by_label("Not found");

    h.state_mut().find.options.regex = true;
    h.state_mut().find.pattern = "a(".into();
    h.run();
    assert!(
        h.query_by_label_contains("Invalid pattern").is_some(),
        "error shows while typing, before searching"
    );
}

#[test]
fn ctrl_f_takes_the_selection_as_the_pattern() {
    let mut h = with_text("hello world");
    key(&mut h, Modifiers::COMMAND, Key::Home);
    for _ in 0..5 {
        key(&mut h, Modifiers::SHIFT, Key::ArrowRight);
    }
    key(&mut h, Modifiers::COMMAND, Key::F);
    assert_eq!(h.state().find.pattern, "hello");
}

#[test]
fn count_select_all_and_replace_all() {
    let mut h = with_text("one two one three one");
    find_for(&mut h, "one");
    click(&mut h, "Count");
    h.get_by_label("3 matches");
    click(&mut h, "Select All");
    assert_eq!(sels(&h), vec![(0, 3), (8, 11), (18, 21)]);
    h.get_by_label("3 matches selected");

    key(&mut h, Modifiers::COMMAND, Key::H);
    h.state_mut().find.replacement = "1".into();
    click(&mut h, "Replace All");
    assert_eq!(text(&h), "1 two 1 three 1");
    h.get_by_label("Replaced 3 occurrences");
    // Escape from the panel's buttons closes it and returns to the editor,
    // where the replacement is one undo step.
    key(&mut h, Modifiers::NONE, Key::Escape);
    assert!(!h.state().find.open);
    key(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(text(&h), "one two one three one");
}

#[test]
fn replace_steps_through_matches_with_captures() {
    let mut h = with_text("a=1 b=2");
    key(&mut h, Modifiers::COMMAND, Key::Home);
    key(&mut h, Modifiers::COMMAND, Key::H);
    let f = &mut h.state_mut().find;
    f.options.regex = true;
    f.pattern = r"(\w)=(\d)".into();
    f.replacement = "$2=$1".into();
    h.run();
    click(&mut h, "Replace"); // finds the first
    click(&mut h, "Replace"); // replaces it, selects the next
    assert_eq!(text(&h), "1=a b=2");
    assert_eq!(sels(&h), vec![(4, 7)]);
    click(&mut h, "Replace");
    assert_eq!(text(&h), "1=a 2=b");
}

#[test]
fn escape_closes_the_panel_and_returns_to_the_editor() {
    let mut h = with_text("abc");
    find_for(&mut h, "b");
    key(&mut h, Modifiers::NONE, Key::Escape);
    assert!(!h.state().find.open);
    typ(&mut h, "!");
    assert_eq!(text(&h), "abc!", "typing goes to the editor again");
}

#[test]
fn matches_are_highlighted_on_screen() {
    let mut h = Harness::builder()
        .with_size(Vec2::new(620.0, 300.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .wgpu()
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.event(Event::Paste(
        "let total = price * qty;\nlet tax = total * rate;\nprint(total + tax);\n".into(),
    ));
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::Home);
    find_for(&mut h, "total");
    key(&mut h, Modifiers::NONE, Key::Enter);
    h.snapshot("find_highlights");
}
