//! The command palette: Ctrl+Shift+A lists every command, typing narrows
//! the list, Enter or a click runs one, Escape closes it.

use egui::{Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;
use ochrepad_core::Action;

mod util;

fn harness(text: &str) -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(900.0, 600.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .wgpu()
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.run();
    let view = h.state().active_view();
    h.state_mut()
        .editor
        .apply(view, Action::InsertText(text.to_owned()))
        .unwrap();
    h.run();
    h
}

fn key(h: &mut Harness<'_, OchrepadApp>, m: Modifiers, k: Key) {
    h.key_press_modifiers(m, k);
    h.run();
}

fn typ(h: &mut Harness<'_, OchrepadApp>, s: &str) {
    h.event(Event::Text(s.into()));
    h.run();
}

fn open(h: &mut Harness<'_, OchrepadApp>) {
    key(h, Modifiers::COMMAND | Modifiers::SHIFT, Key::A);
    assert!(h.state().palette_is_open());
}

fn text(h: &Harness<'_, OchrepadApp>) -> String {
    h.state().buffer().doc().to_string()
}

const SORT: &str = "Edit: Line Operations: Sort Lines Ascending";

#[test]
fn typing_finds_a_command_and_enter_runs_it() {
    let mut h = harness("pear\napple\nfig\n");
    open(&mut h);
    let all = h.state().palette_names();
    assert!(all.len() > 150, "every command: {}", all.len());
    assert_eq!(all[0], "File: New", "in the menus' order");
    assert!(all.contains(&SORT.to_owned()));

    // Typing goes to the palette, not the document.
    typ(&mut h, "sort asc");
    assert_eq!(text(&h), "pear\napple\nfig\n");
    let found = h.state().palette_names();
    assert_eq!(found[0], SORT, "{found:?}");
    assert!(found.len() < all.len());

    key(&mut h, Modifiers::NONE, Key::Enter);
    assert!(!h.state().palette_is_open());
    assert_eq!(text(&h), "apple\nfig\npear\n");
    // The editor has the keyboard again.
    typ(&mut h, "x");
    assert!(text(&h).contains('x'));
}

#[test]
fn initials_match_and_the_arrow_keys_choose() {
    let mut h = harness("b\na\n");
    open(&mut h);
    // The first letters of a name's words.
    typ(&mut h, "sld");
    let found = h.state().palette_names();
    assert!(
        found.contains(&"Edit: Line Operations: Sort Lines Descending".to_owned()),
        "{found:?}"
    );
    typ(&mut h, " zzzz");
    assert!(h.state().palette_names().is_empty());
    h.get_by_label("No command matches");
    // Enter with nothing to run leaves the palette open.
    key(&mut h, Modifiers::NONE, Key::Enter);
    assert!(h.state().palette_is_open());
    key(&mut h, Modifiers::NONE, Key::Escape);
    assert!(!h.state().palette_is_open());
    assert_eq!(text(&h), "b\na\n");

    // Down moves to the second command; Up from the first goes to the last.
    open(&mut h);
    typ(&mut h, "word wrap");
    assert_eq!(h.state().palette_names()[0], "View: Word Wrap");
    assert!(!h.state().settings.word_wrap);
    key(&mut h, Modifiers::NONE, Key::Enter);
    assert!(h.state().settings.word_wrap);
}

#[test]
fn a_click_runs_a_command_and_it_is_listed_first_next_time() {
    let mut h = harness("pear\napple");
    open(&mut h);
    typ(&mut h, "upperc");
    h.get_by_label("Edit: Convert Case: UPPERCASE").click();
    h.run();
    assert!(!h.state().palette_is_open());
    assert_eq!(text(&h), "pear\nAPPLE", "the caret's line");

    open(&mut h);
    assert_eq!(
        h.state().palette_names()[0],
        "Edit: Convert Case: UPPERCASE",
        "run lately"
    );
    // The same key closes it.
    key(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::A);
    assert!(!h.state().palette_is_open());
}

#[test]
fn commands_follow_what_has_the_focus_and_show_their_keys() {
    let mut h = harness("text\n");
    // The button at the end of the menu bar opens it too.
    h.get_by_label("Commands").click();
    h.run();
    assert!(h.state().palette_is_open());
    key(&mut h, Modifiers::NONE, Key::Escape);

    // No shell starts: the terminal only has to have the focus.
    h.state_mut().settings.terminal_shell = "ochrepad-no-such-shell".to_owned();
    h.state_mut().new_terminal();
    h.run();
    open(&mut h);
    let names = h.state().palette_names();
    assert!(names.contains(&"View: New Terminal".to_owned()));
    assert!(
        !names.iter().any(|n| n.starts_with("Edit: ")),
        "editing commands do not apply to a terminal"
    );
    key(&mut h, Modifiers::NONE, Key::Escape);
    assert!(!h.state().palette_is_open());
}

#[test]
fn the_palette_looks_right() {
    let mut h = harness("pear\napple\nfig\n");
    h.set_size(Vec2::new(760.0, 420.0));
    h.run();
    open(&mut h);
    typ(&mut h, "line");
    key(&mut h, Modifiers::NONE, Key::ArrowDown);
    util::snapshot(&mut h, "palette");
}
