//! Accepting completions (`Action::Complete`) and applying a language
//! server's edits (`Action::Edits`).

mod util;

use ochrepad_core::{Action, ActionError, ByteOffset, TextEdit};
use util::setup;

fn complete(before: usize, after: usize, text: &str) -> Action {
    Action::Complete {
        before,
        after,
        text: text.to_owned(),
    }
}

fn edit(start: usize, end: usize, text: &str) -> TextEdit {
    TextEdit {
        start: ByteOffset(start),
        end: ByteOffset(end),
        text: text.to_owned(),
    }
}

#[test]
fn completing_replaces_the_typed_part() {
    let mut t = setup("let x = val|;");
    assert_eq!(t.apply(complete(3, 0, "value")).show(), "let x = value|;");
    // The rest of a word after the caret too.
    let mut t = setup("call(ge|tTe)");
    assert_eq!(t.apply(complete(2, 3, "getText")).show(), "call(getText|)");
    // Never past the line.
    let mut t = setup("ab\n|cd");
    assert_eq!(t.apply(complete(5, 0, "x")).show(), "ab\nx|cd");
    let mut t = setup("ün|ï");
    assert_eq!(t.apply(complete(2, 1, "über")).show(), "über|");
}

#[test]
fn completing_at_every_caret_is_one_undo_step() {
    let mut t = setup("pri|\npri|\nx");
    t.apply(complete(3, 0, "println!"));
    assert_eq!(t.show(), "println!|\nprintln!|\nx");
    t.apply(Action::Undo);
    assert_eq!(t.text(), "pri\npri\nx");
}

#[test]
fn close_carets_do_not_overlap() {
    let mut t = setup("ab|cd|ef");
    t.apply(complete(2, 2, "X"));
    // The second caret's range starts where the first one's ended; the two
    // edits touch, so the carets end up together.
    assert_eq!(t.show(), "XX|");
    let mut t = setup("ab| cd|");
    t.apply(complete(2, 0, "X"));
    assert_eq!(t.show(), "X| X|");
}

#[test]
fn a_selection_is_replaced_like_typing() {
    let mut t = setup("hello world");
    t.set_sels(vec![(6, 11)]);
    assert_eq!(t.apply(complete(3, 0, "there")).show(), "hello there|");
}

#[test]
fn server_edits_apply_together_and_keep_carets_in_place() {
    let mut t = setup("fn main() {\n    let x=1;|\n}");
    // Formatting: "x=1" becomes "x = 1", and a use line is added at the top.
    t.apply(Action::Edits(vec![
        edit(21, 22, " = "),
        edit(0, 0, "use std::io;\n"),
    ]));
    assert_eq!(t.show(), "use std::io;\nfn main() {\n    let x = 1;|\n}");
    t.apply(Action::Undo);
    assert_eq!(t.text(), "fn main() {\n    let x=1;\n}");
}

#[test]
fn inserts_at_one_place_keep_their_order() {
    let mut t = setup("|x");
    t.apply(Action::Edits(vec![edit(0, 0, "a"), edit(0, 0, "b")]));
    assert_eq!(t.text(), "abx");
}

#[test]
fn edits_that_do_not_fit_are_refused_whole() {
    let bad = [
        vec![edit(0, 2, "a"), edit(1, 3, "b")],
        vec![edit(0, 99, "a")],
        vec![edit(3, 1, "a")],
        // Inside the two bytes of "é".
        vec![edit(2, 2, "a")],
    ];
    for edits in bad {
        let mut t = setup("hé world|");
        let before = t.text();
        let r = t.ed.apply(t.view, Action::Edits(edits.clone()));
        assert_eq!(r, Err(ActionError::InvalidEdits), "{edits:?}");
        assert_eq!(t.text(), before);
    }
}

#[test]
fn completions_are_recorded_in_macros_and_server_edits_are_not() {
    let mut t = setup("ab|");
    t.ed.start_recording();
    t.apply(complete(2, 0, "abc"));
    t.apply(Action::Edits(vec![edit(0, 0, "x")]));
    let recorded = t.ed.stop_recording();
    assert_eq!(recorded, vec![complete(2, 0, "abc")]);
}
