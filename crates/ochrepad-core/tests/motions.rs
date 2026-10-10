mod util;

use ochrepad_core::{Action, ByteOffset, Motion};
use util::setup;

const FAMILY: &str = "👨‍👩‍👧";

fn heads_after_repeating(text: &str, start: usize, motion: Motion, n: usize) -> Vec<usize> {
    let mut t = setup(text);
    t.set_sels(vec![(start, start)]);
    (0..n)
        .map(|_| {
            t.mv(motion);
            t.sels().primary().head.0
        })
        .collect()
}

#[test]
fn right_and_left_step_by_grapheme() {
    let text = format!("e\u{301}{FAMILY}漢\tx");
    // é = 3 bytes, family = 18, 漢 = 3, tab = 1, x = 1
    assert_eq!(
        heads_after_repeating(&text, 0, Motion::Right, 6),
        vec![3, 21, 24, 25, 26, 26]
    );
    assert_eq!(
        heads_after_repeating(&text, 26, Motion::Left, 6),
        vec![25, 24, 21, 3, 0, 0]
    );
}

#[test]
fn right_and_left_cross_line_ends() {
    let mut t = setup("ab|\ncd");
    assert_eq!(t.mv(Motion::Right).show(), "ab\n|cd");
    assert_eq!(t.mv(Motion::Left).show(), "ab|\ncd");
}

#[test]
fn left_and_right_collapse_a_selection() {
    for (a, h) in [(1, 4), (4, 1)] {
        let mut t = setup("abcdef");
        t.set_sels(vec![(a, h)]);
        assert_eq!(t.mv(Motion::Left).show(), "a|bcdef");
        t.set_sels(vec![(a, h)]);
        assert_eq!(t.mv(Motion::Right).show(), "abcd|ef");
    }
}

#[test]
fn extend_grows_and_shrinks_selection() {
    let mut t = setup("a|bcd");
    assert_eq!(t.ext(Motion::Right).ext(Motion::Right).show(), "a^bc|d");
    assert_eq!(t.ext(Motion::Left).show(), "a^b|cd");
    assert_eq!(t.ext(Motion::Left).ext(Motion::Left).show(), "|a^bcd");
}

#[test]
fn word_right_and_left() {
    let text = "foo bar.baz  qux\n  next";
    assert_eq!(
        heads_after_repeating(text, 0, Motion::WordRight, 8),
        vec![4, 7, 8, 13, 16, 19, 23, 23]
    );
    assert_eq!(
        heads_after_repeating(text, 23, Motion::WordLeft, 8),
        vec![19, 16, 13, 8, 7, 4, 0, 0]
    );
}

#[test]
fn words_include_underscores_cjk_and_combining_marks() {
    let text = "snake_case 漢字 e\u{301}t";
    assert_eq!(
        heads_after_repeating(text, 0, Motion::WordRight, 3),
        vec![11, 18, 22]
    );
    assert_eq!(
        heads_after_repeating(text, 22, Motion::WordLeft, 3),
        vec![18, 11, 0]
    );
}

#[test]
fn smart_home_toggles() {
    let mut t = setup("x\n    abc|");
    assert_eq!(t.mv(Motion::Home).show(), "x\n    |abc");
    assert_eq!(t.mv(Motion::Home).show(), "x\n|    abc");
    assert_eq!(t.mv(Motion::Home).show(), "x\n    |abc");

    let mut t = setup("  \t  |");
    assert_eq!(t.mv(Motion::Home).show(), "|  \t  ");
    assert_eq!(t.mv(Motion::Home).show(), "  \t  |");
}

#[test]
fn line_and_document_ends() {
    let mut t = setup("ab\ncd|ef\ngh");
    assert_eq!(t.mv(Motion::LineStart).show(), "ab\n|cdef\ngh");
    assert_eq!(t.mv(Motion::LineEnd).show(), "ab\ncdef|\ngh");
    assert_eq!(t.mv(Motion::DocStart).show(), "|ab\ncdef\ngh");
    assert_eq!(t.mv(Motion::DocEnd).show(), "ab\ncdef\ngh|");
    assert_eq!(t.ext(Motion::LineStart).show(), "ab\ncdef\n|gh^");
}

#[test]
fn up_down_remember_the_visual_column() {
    let mut t = setup("abcde|f\nab\nabcdef");
    assert_eq!(t.mv(Motion::Down).show(), "abcdef\nab|\nabcdef");
    assert_eq!(t.mv(Motion::Down).show(), "abcdef\nab\nabcde|f");
    assert_eq!(
        t.mv(Motion::Up).mv(Motion::Up).show(),
        "abcde|f\nab\nabcdef"
    );

    // A horizontal move resets the goal.
    let mut t = setup("abcde|f\nab\nabcdef");
    t.mv(Motion::Down).mv(Motion::Left);
    assert_eq!(t.mv(Motion::Down).show(), "abcdef\nab\na|bcdef");
}

#[test]
fn up_down_through_wide_glyphs() {
    let mut t = setup("漢字|x\nabcd");
    assert_eq!(t.mv(Motion::Down).show(), "漢字x\nabcd|");
    assert_eq!(t.mv(Motion::Up).show(), "漢字|x\nabcd");

    // Column 3 is inside 字 (columns 2..4): snap to its start.
    let mut t = setup("漢字x\nabc|d");
    assert_eq!(t.mv(Motion::Up).show(), "漢|字x\nabcd");
}

#[test]
fn up_down_through_tabs() {
    let mut t = setup("\t|x\nabcdefgh");
    assert_eq!(t.mv(Motion::Down).show(), "\tx\nabcd|efgh");
    let mut t = setup("\tx\nab|cdefgh");
    assert_eq!(t.mv(Motion::Up).show(), "|\tx\nabcdefgh");
}

#[test]
fn up_down_through_combining_marks_and_zwj() {
    let e = "e\u{301}";
    let mut t = setup(&format!("{e}{e}|{e}\nabc"));
    assert_eq!(t.mv(Motion::Down).show(), format!("{e}{e}{e}\nab|c"));
    assert_eq!(t.mv(Motion::Up).show(), format!("{e}{e}|{e}\nabc"));

    let mut t = setup(&format!("{FAMILY}|x\nabc"));
    assert_eq!(t.mv(Motion::Down).show(), format!("{FAMILY}x\nab|c"));
    let mut t = setup(&format!("{FAMILY}x\na|bc"));
    assert_eq!(t.mv(Motion::Up).show(), format!("|{FAMILY}x\nabc"));
}

#[test]
fn up_on_first_line_and_down_on_last_line_stay() {
    let mut t = setup("a|bc\nd");
    assert_eq!(t.mv(Motion::Up).show(), "a|bc\nd");
    let mut t = setup("abc\nd|");
    assert_eq!(t.mv(Motion::Down).show(), "abc\nd|");
}

#[test]
fn page_up_and_down() {
    let text = "0\n1\n2\n3\n4\n5\n6\n7\n8\n9";
    let line = |t: &util::T| {
        t.ed.buffer(t.buffer().id())
            .unwrap()
            .doc()
            .line_of(t.sels().primary().head)
    };
    let mut t = setup(text);
    t.set_sels(vec![(2, 2)]);
    t.mv(Motion::PageDown(3));
    assert_eq!(line(&t), 4);
    t.mv(Motion::PageDown(100));
    assert_eq!(line(&t), 9);
    t.mv(Motion::PageUp(5));
    assert_eq!(line(&t), 4);
    t.ext(Motion::PageUp(100));
    assert_eq!(t.sels().primary().head, ByteOffset(0));
    assert_eq!(t.sels().primary().anchor, ByteOffset(8));
}

#[test]
fn select_all_word_and_line() {
    let mut t = setup("foo ba|r baz");
    assert_eq!(t.apply(Action::SelectWord).show(), "foo ^bar| baz");
    let mut t = setup("foo| bar");
    assert_eq!(t.apply(Action::SelectWord).show(), "^foo| bar");
    let mut t = setup("a  |  b");
    assert_eq!(t.apply(Action::SelectWord).show(), "a^    |b");
    assert_eq!(t.apply(Action::SelectAll).show(), "^a    b|");

    let mut t = setup("ab\nc|d\nef");
    assert_eq!(t.apply(Action::SelectLine).show(), "ab\n^cd\n|ef");
    let mut t = setup("ab\ncd\ne|f");
    assert_eq!(t.apply(Action::SelectLine).show(), "ab\ncd\n^ef|");
}

#[test]
fn add_cursor_below_and_above() {
    let mut t = setup("ab|cd\nx\nabcd");
    assert_eq!(t.apply(Action::AddCursorBelow).show(), "ab|cd\nx|\nabcd");
    // The new caret remembers the goal column through the short line.
    assert_eq!(t.apply(Action::AddCursorBelow).show(), "ab|cd\nx|\nab|cd");
    assert_eq!(t.sels().primary_index(), 2);
    assert_eq!(t.apply(Action::AddCursorBelow).show(), "ab|cd\nx|\nab|cd");
    assert_eq!(t.apply(Action::AddCursorAbove).show(), "ab|cd\nx|\nab|cd");
    assert_eq!(t.apply(Action::CollapseSelections).show(), "abcd\nx\nab|cd");

    let mut t = setup("abc\nab|c");
    assert_eq!(t.apply(Action::AddCursorAbove).show(), "ab|c\nab|c");
    assert_eq!(t.sels().primary_index(), 0);
}
