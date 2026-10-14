//! Multi-select Next and Multi-select All.

mod util;

use ochrepad_core::{Action, ActionError};
use util::setup;

fn next(match_case: bool, whole_word: bool) -> Action {
    Action::MultiSelectNext {
        match_case,
        whole_word,
    }
}

fn all(match_case: bool, whole_word: bool) -> Action {
    Action::MultiSelectAll {
        match_case,
        whole_word,
    }
}

#[test]
fn next_selects_the_word_then_adds_each_occurrence() {
    let mut t = setup("let x|y = xy + Xy + xyz;\nxy");
    t.apply(next(true, true));
    assert_eq!(t.show(), "let ^xy| = xy + Xy + xyz;\nxy");
    t.apply(next(true, true));
    assert_eq!(t.show(), "let ^xy| = ^xy| + Xy + xyz;\nxy");
    // "Xy" differs in case and "xyz" is not the whole word.
    t.apply(next(true, true));
    assert_eq!(t.show(), "let ^xy| = ^xy| + Xy + xyz;\n^xy|");
    // Round from the end: nothing left.
    assert_eq!(
        t.ed.apply(t.view, next(true, true)),
        Err(ActionError::NotFound)
    );
    // Typing edits them all.
    t.apply(Action::InsertText("z".into()));
    assert_eq!(t.text(), "let z = z + Xy + xyz;\nz");
}

#[test]
fn next_ignoring_case_and_parts_of_words() {
    let mut t = setup("ab Ab abc");
    t.set_sels(vec![(0, 2)]);
    t.apply(next(false, false));
    t.apply(next(false, false));
    assert_eq!(t.show(), "^ab| ^Ab| ^ab|c");
}

#[test]
fn next_goes_round_from_the_end() {
    let mut t = setup("cat dog cat dog");
    t.set_sels(vec![(12, 15)]);
    t.apply(next(true, true));
    assert_eq!(t.show(), "cat ^dog| cat ^dog|");
}

#[test]
fn all_selects_every_occurrence() {
    let mut t = setup("a.b x a.b y A.B");
    t.set_sels(vec![(12, 15)]);
    t.apply(all(false, false));
    assert_eq!(t.show(), "^a.b| x ^a.b| y ^A.B|");
    // The primary stays the one the caret was on.
    let sels = t.sels();
    assert_eq!(sels.primary().head.0, 15);
    let mut t = setup("|");
    assert_eq!(
        t.ed.apply(t.view, all(true, true)),
        Err(ActionError::NotFound)
    );
}

#[test]
fn whole_word_works_for_text_that_starts_or_ends_with_punctuation() {
    // A boundary is only asked for beside word characters.
    let mut t = setup("echo $var; x=$var");
    t.set_sels(vec![(5, 9)]);
    t.apply(all(true, true));
    assert_eq!(t.show(), "echo ^$var|; x=^$var|");
    let mut t = setup("foo(); foo();");
    t.set_sels(vec![(0, 4)]);
    t.apply(next(true, true));
    assert_eq!(t.show(), "^foo(|); ^foo(|);");
}
