//! The completion popup: it opens while a word is typed, follows it, and
//! accepts with Enter, Tab or a click; Ctrl+Space opens it any time.

use egui::{Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;
use ochrepad_core::{Action, ByteOffset, SelectionSet};

fn harness(text: &str) -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(900.0, 600.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.run();
    let view = h.state().active_view();
    let app = h.state_mut();
    app.editor
        .apply(view, Action::InsertText(text.to_owned()))
        .unwrap();
    h.run();
    h
}

fn typ(h: &mut Harness<'_, OchrepadApp>, s: &str) {
    for c in s.chars() {
        h.event(Event::Text(c.to_string()));
        h.run();
    }
}

fn key(h: &mut Harness<'_, OchrepadApp>, m: Modifiers, k: Key) {
    h.key_press_modifiers(m, k);
    h.run();
}

fn labels(h: &Harness<'_, OchrepadApp>) -> Vec<String> {
    h.state()
        .view_state()
        .map(|s| s.completion.labels())
        .unwrap_or_default()
}

fn text(h: &Harness<'_, OchrepadApp>) -> String {
    h.state().buffer().doc().to_string()
}

const WORDS: &str = "println printf print_all parse_args\n";

#[test]
fn typing_two_characters_opens_the_popup_and_enter_accepts() {
    let mut h = harness(WORDS);
    typ(&mut h, "p");
    assert!(labels(&h).is_empty(), "not after one character");
    typ(&mut h, "r");
    let shown = labels(&h);
    assert!(shown.contains(&"println".to_owned()), "{shown:?}");
    assert!(
        !shown.contains(&"parse_args".to_owned()),
        "pr: not parse_args"
    );
    h.get_by_label("Completions");
    typ(&mut h, "intl");
    assert_eq!(labels(&h)[0], "println");
    key(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(text(&h), format!("{WORDS}println"));
    assert!(labels(&h).is_empty());
    // One undo step back to what was typed.
    key(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(text(&h), format!("{WORDS}printl"));
}

#[test]
fn arrows_move_and_escape_closes() {
    let mut h = harness(WORDS);
    typ(&mut h, "pri");
    let shown = labels(&h);
    assert!(shown.len() >= 3, "{shown:?}");
    key(&mut h, Modifiers::NONE, Key::ArrowDown);
    key(&mut h, Modifiers::NONE, Key::Tab);
    assert_eq!(text(&h), format!("{WORDS}{}", shown[1]));
    typ(&mut h, " pa");
    assert!(!labels(&h).is_empty());
    key(&mut h, Modifiers::NONE, Key::Escape);
    assert!(labels(&h).is_empty());
    // Escape went to the popup, not the editor; Enter now breaks the line.
    key(&mut h, Modifiers::NONE, Key::Enter);
    assert!(text(&h).ends_with("pa\n"));
}

#[test]
fn tab_replaces_the_rest_of_the_word() {
    let mut h = harness(WORDS);
    typ(&mut h, "printxyz");
    // Back to after "pri", then complete over "ntxyz".
    let view = h.state().active_view();
    let at = WORDS.len() + 3;
    h.state_mut()
        .editor
        .apply(
            view,
            Action::SetSelections(SelectionSet::caret(ByteOffset(at))),
        )
        .unwrap();
    key(&mut h, Modifiers::COMMAND, Key::Space);
    assert!(labels(&h).contains(&"printf".to_owned()));
    let i = labels(&h).iter().position(|l| l == "printf").unwrap();
    for _ in 0..i {
        key(&mut h, Modifiers::NONE, Key::ArrowDown);
    }
    key(&mut h, Modifiers::NONE, Key::Tab);
    assert_eq!(text(&h), format!("{WORDS}printf"));
}

#[test]
fn a_non_word_character_closes_and_ctrl_space_opens_anywhere() {
    let mut h = harness(WORDS);
    typ(&mut h, "pri");
    assert!(!labels(&h).is_empty());
    typ(&mut h, "(");
    assert!(labels(&h).is_empty());
    // Nothing typed yet: Ctrl+Space shows everything.
    key(&mut h, Modifiers::COMMAND, Key::Space);
    let all = labels(&h);
    for w in ["println", "printf", "print_all", "parse_args"] {
        assert!(all.contains(&w.to_owned()), "{all:?}");
    }
}

#[test]
fn clicking_a_suggestion_accepts_it() {
    let mut h = harness(WORDS);
    typ(&mut h, "pa");
    // A real press and release on the row (the editor under it must not
    // take them).
    let at = h.get_by_label("parse_args").rect().center();
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.event(Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.run();
    assert_eq!(text(&h), format!("{WORDS}parse_args"));
}

#[test]
fn read_only_documents_and_the_setting_turn_it_off() {
    let mut h = harness(WORDS);
    h.state_mut().settings.completion = false;
    h.run();
    typ(&mut h, "pri");
    assert!(labels(&h).is_empty());
    h.state_mut().settings.completion = true;
    let view = h.state().active_view();
    let buffer = h.state().editor.view(view).unwrap().buffer();
    h.state_mut().editor.buffer_mut(buffer).unwrap().read_only = true;
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::Space);
    assert!(labels(&h).is_empty());
}

#[test]
fn words_from_other_open_documents_are_offered() {
    let mut h = harness("");
    let first = h.state().active_view();
    h.state_mut()
        .editor
        .apply(first, Action::InsertText("elsewhere_defined\n".into()))
        .unwrap();
    h.state_mut().new_file();
    h.run();
    // The other document's words are counted on a thread.
    for _ in 0..200 {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    h.run();
    typ(&mut h, "else");
    assert_eq!(labels(&h), ["elsewhere_defined"]);
}

fn rust(h: &mut Harness<'_, OchrepadApp>) {
    h.state_mut().set_language("Rust");
    h.run();
}

#[test]
fn keywords_of_the_language_are_offered() {
    let mut h = harness("");
    rust(&mut h);
    typ(&mut h, "mat");
    let shown = labels(&h);
    assert!(shown.contains(&"match".to_owned()), "{shown:?}");
    let kind = h
        .state()
        .view_state()
        .unwrap()
        .completion
        .shown()
        .iter()
        .find(|c| c.label == "match")
        .map(|c| c.kind);
    assert_eq!(kind, Some(ochrepad_core::complete::Kind::Keyword));
}

#[test]
fn not_by_itself_in_comments_and_strings() {
    let mut h = harness("print_all println\n// ");
    rust(&mut h);
    typ(&mut h, "pri");
    assert!(labels(&h).is_empty(), "in a comment");
    key(&mut h, Modifiers::COMMAND, Key::Space);
    assert!(!labels(&h).is_empty(), "Ctrl+Space still works");
    key(&mut h, Modifiers::NONE, Key::Escape);
    key(&mut h, Modifiers::NONE, Key::Enter);
    typ(&mut h, "let s = \"pri");
    assert!(labels(&h).is_empty(), "in a string");
    typ(&mut h, "\";");
    key(&mut h, Modifiers::NONE, Key::Enter);
    typ(&mut h, "pri");
    assert!(!labels(&h).is_empty(), "in code again");
}

#[test]
fn definitions_from_open_documents_are_offered_as_such() {
    let mut h = harness("fn compute_total(x: u32) -> u32 { x }\n");
    rust(&mut h);
    h.state_mut().new_file();
    rust(&mut h);
    // Definitions are found on a thread.
    for _ in 0..300 {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    h.run();
    typ(&mut h, "comp");
    let state = h.state().view_state().unwrap();
    let found = state
        .completion
        .shown()
        .into_iter()
        .find(|c| c.label == "compute_total")
        .cloned();
    let c = found.expect("offered");
    assert_eq!(c.kind, ochrepad_core::complete::Kind::Function);
    assert_eq!(c.source, ochrepad_core::complete::Source::Definition);
}

/// Steps in real time until `done`, for at most 20 seconds.
fn wait_until(
    h: &mut Harness<'_, OchrepadApp>,
    what: &str,
    done: impl Fn(&Harness<'_, OchrepadApp>) -> bool,
) {
    let start = std::time::Instant::now();
    while !done(h) {
        assert!(
            start.elapsed() < std::time::Duration::from_secs(20),
            "timed out waiting for {what}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.step();
    }
}

#[test]
fn words_and_definitions_from_the_open_folder() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("lib.rs"),
        "pub fn zebrafish_count() -> u32 { 0 }\n// a zebrafinch\n",
    )
    .unwrap();
    let mut h = harness("");
    h.state_mut().open_folder(dir.path());
    h.run();
    wait_until(&mut h, "the folder's index", |h| {
        h.state()
            .completion
            .folder()
            .is_some_and(|f| f.progress == ochrepad::folder_index::Progress::Ready)
    });
    rust(&mut h);
    // Only the folder has these words: the popup waits for its thread.
    typ(&mut h, "zeb");
    wait_until(&mut h, "the folder's answer", |h| labels(h).len() == 2);
    let shown = h.state().view_state().unwrap().completion.shown();
    let fish = shown.iter().find(|c| c.label == "zebrafish_count");
    let fish = fish.expect("offered");
    assert_eq!(fish.kind, ochrepad_core::complete::Kind::Function);
    assert_eq!(fish.detail.as_deref(), Some("lib.rs"));
    assert!(shown.iter().any(|c| c.label == "zebrafinch"));
    // Typing on narrows the list at once, then the folder answers again.
    typ(&mut h, "rafinc");
    assert_eq!(labels(&h), ["zebrafinch"], "from the last answer");
    key(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(text(&h), "zebrafinch");
}
