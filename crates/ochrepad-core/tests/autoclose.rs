//! Auto-closing brackets and quotes (Notepad++'s Auto-Insert), when on.

mod util;

use ochrepad_core::Action;
use util::{T, setup};

fn on(marked: &str) -> T {
    let mut t = setup(marked);
    t.ed.settings.auto_close = true;
    t
}

fn typ(t: &mut T, s: &str) {
    t.apply(Action::InsertText(s.into()));
}

#[test]
fn brackets_close_and_are_stepped_over() {
    let mut t = on("|");
    typ(&mut t, "(");
    assert_eq!(t.show(), "(|)");
    typ(&mut t, "x");
    assert_eq!(t.show(), "(x|)");
    typ(&mut t, ")");
    assert_eq!(t.show(), "(x)|");
    // Nested, and before a closing bracket.
    let mut t = on("f(|)");
    typ(&mut t, "[");
    assert_eq!(t.show(), "f([|])");
    typ(&mut t, "{");
    assert_eq!(t.show(), "f([{|}])");
}

#[test]
fn no_partner_where_it_would_get_in_the_way() {
    // Before a word.
    let mut t = on("|abc");
    typ(&mut t, "(");
    assert_eq!(t.show(), "(|abc");
    // An apostrophe after a letter.
    let mut t = on("don|");
    typ(&mut t, "'");
    assert_eq!(t.show(), "don'|");
    // A selection is replaced as usual.
    let mut t = on("abc");
    t.set_sels(vec![(0, 3)]);
    typ(&mut t, "(");
    assert_eq!(t.show(), "(|");
}

#[test]
fn quotes_pair_and_backspace_deletes_an_empty_pair() {
    let mut t = on("x = |");
    typ(&mut t, "\"");
    assert_eq!(t.show(), "x = \"|\"");
    t.apply(Action::DeleteBackward);
    assert_eq!(t.show(), "x = |");
    typ(&mut t, "\"");
    typ(&mut t, "s");
    typ(&mut t, "\"");
    assert_eq!(t.show(), "x = \"s\"|", "the closing quote is stepped over");
}

#[test]
fn every_caret_gets_its_pair() {
    let mut t = on("a|\nb|");
    typ(&mut t, "[");
    assert_eq!(t.show(), "a[|]\nb[|]");
    t.apply(Action::DeleteBackward);
    assert_eq!(t.show(), "a|\nb|");
}

#[test]
fn a_pair_and_what_is_typed_in_it_undo_together() {
    let mut t = on("|");
    typ(&mut t, "(");
    typ(&mut t, "a");
    typ(&mut t, "b");
    t.apply(Action::Undo);
    assert_eq!(t.show(), "|");
}

#[test]
fn it_is_off_unless_asked_for() {
    let mut t = setup("|");
    typ(&mut t, "(");
    assert_eq!(t.show(), "(|");
}
