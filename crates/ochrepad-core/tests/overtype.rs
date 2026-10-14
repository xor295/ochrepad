//! Overtype: typed characters replace the ones after the caret, up to the
//! line's end.

mod util;

use ochrepad_core::Action;
use util::setup;

fn over(text: &str) -> util::T {
    let mut t = setup(text);
    t.ed.settings.overtype = true;
    t
}

fn typed(t: &mut util::T, text: &str) {
    for c in text.chars() {
        t.apply(Action::InsertText(c.to_string()));
    }
}

#[test]
fn typing_replaces_the_characters_after_the_caret() {
    let mut t = over("a|bcd\nxy");
    typed(&mut t, "XY");
    assert_eq!(t.show(), "aXY|d\nxy");
    // At the line's end, it inserts; the next line is left alone.
    typed(&mut t, "ZW");
    assert_eq!(t.show(), "aXYZW|\nxy");
    // So does the end of the document.
    let mut t = over("ab|");
    typed(&mut t, "c");
    assert_eq!(t.show(), "abc|");
}

#[test]
fn a_character_replaces_a_whole_grapheme_and_a_tab() {
    // "e" with a combining accent is one character on screen.
    let mut t = over("|e\u{301}x\ty");
    typed(&mut t, "ab");
    assert_eq!(t.show(), "ab|\ty");
    typed(&mut t, "c");
    assert_eq!(t.show(), "abc|y");
    // Text with more than one character (from an input method) replaces
    // as many.
    let mut t = over("|12345");
    t.apply(Action::InsertText("\u{65e5}\u{672c}".into()));
    assert_eq!(t.show(), "\u{65e5}\u{672c}|345");
}

#[test]
fn every_caret_types_over_its_own_line() {
    let mut t = over("|abc\n|de\n|");
    typed(&mut t, "XYZ");
    assert_eq!(t.show(), "XYZ|\nXYZ|\nXYZ|");
}

#[test]
fn selections_enter_and_paste_insert_as_usual() {
    let mut t = over("abcdef");
    t.set_sels(vec![(1, 3)]);
    typed(&mut t, "X");
    assert_eq!(t.show(), "aX|def");
    t.apply(Action::Paste("PQ".into()));
    assert_eq!(t.show(), "aXPQ|def");
    t.apply(Action::InsertNewline);
    assert_eq!(t.show(), "aXPQ\n|def");
}

#[test]
fn typing_over_is_one_undo_step_that_brings_the_text_back() {
    let mut t = over("|hello world");
    typed(&mut t, "HELLO");
    assert_eq!(t.show(), "HELLO| world");
    t.apply(Action::Undo);
    assert_eq!(t.show(), "|hello world");
    t.apply(Action::Redo);
    assert_eq!(t.show(), "HELLO| world");
}

#[test]
fn off_it_inserts() {
    let mut t = setup("a|bc");
    typed(&mut t, "X");
    assert_eq!(t.show(), "aX|bc");
}

#[test]
fn auto_close_still_applies_where_nothing_is_typed_over() {
    let mut t = over("x|");
    t.ed.settings.auto_close = true;
    typed(&mut t, "(");
    assert_eq!(t.show(), "x(|)");
    // Inside the pair, the closer is typed over.
    typed(&mut t, ")");
    assert_eq!(t.show(), "x()|");
}
