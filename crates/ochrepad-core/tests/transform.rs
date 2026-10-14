//! Text transforms: case, sort and line operations.

mod util;

use ochrepad_core::{Action, CaseMode, SortOptions, VisualCol};
use util::setup;

fn sort(f: impl FnOnce(&mut SortOptions)) -> Action {
    let mut o = SortOptions::default();
    f(&mut o);
    Action::SortLines(o)
}

#[test]
fn case_conversion_covers_text_that_changes_length() {
    let mut t = setup("a straße b");
    t.set_sels(vec![(2, 9)]);
    t.apply(Action::ConvertCase(CaseMode::Upper));
    assert_eq!(t.show(), "a ^STRASSE| b");
    t.apply(Action::Undo);
    assert_eq!(t.show(), "a ^straße| b");
}

#[test]
fn an_empty_selection_converts_its_line_and_keeps_the_caret() {
    let mut t = setup("hello\nwor|ld");
    t.apply(Action::ConvertCase(CaseMode::Upper));
    assert_eq!(t.show(), "hello\nWOR|LD");
    t.apply(Action::ConvertCase(CaseMode::Invert));
    assert_eq!(t.show(), "hello\nwor|ld");
}

#[test]
fn several_carets_on_one_line_convert_it_once() {
    let mut t = setup("a|b|c\nd|e");
    t.apply(Action::ConvertCase(CaseMode::Upper));
    assert_eq!(t.show(), "A|B|C\nD|E");
}

#[test]
fn title_case_and_column_selections() {
    let mut t = setup("|the QUICK brown fox's tail");
    t.apply(Action::ConvertCase(CaseMode::Title));
    assert_eq!(t.text(), "The Quick Brown Fox's Tail");

    let mut t = setup("abcd\nefgh");
    t.apply(Action::ColumnSelect {
        anchor_line: 0,
        anchor_col: VisualCol(1),
        head_line: 1,
        head_col: VisualCol(3),
    });
    t.apply(Action::ConvertCase(CaseMode::Upper));
    assert_eq!(t.show(), "a^BC|d\ne^FG|h");
    assert!(t.sels().rect().is_some(), "still a column selection");
}

#[test]
fn sort_the_whole_document_ignoring_the_final_newline() {
    let mut t = setup("|pear\napple\nfig\n");
    t.apply(sort(|_| {}));
    assert_eq!(t.text(), "apple\nfig\npear\n");
    t.apply(sort(|o| o.descending = true));
    assert_eq!(t.text(), "pear\nfig\napple\n");
    t.apply(Action::Undo);
    assert_eq!(t.text(), "apple\nfig\npear\n");
}

#[test]
fn sort_options() {
    let mut t = setup("|b\nA\na\nB");
    t.apply(sort(|_| {}));
    assert_eq!(t.text(), "A\nB\na\nb");
    let mut t = setup("|b\nA\na\nB");
    t.apply(sort(|o| o.case_insensitive = true));
    assert_eq!(t.text(), "A\na\nb\nB", "stable within equal keys");

    let mut t = setup("|10\n9\n-1\nx\n2.5\n 3 items");
    t.apply(sort(|o| o.numeric = true));
    assert_eq!(t.text(), "x\n-1\n2.5\n 3 items\n9\n10");

    let mut t = setup("|b\na\nb\nA\na");
    t.apply(sort(|o| o.remove_duplicates = true));
    assert_eq!(t.text(), "A\na\nb");
    let mut t = setup("|b\na\nb\nA\na");
    t.apply(sort(|o| {
        o.remove_duplicates = true;
        o.case_insensitive = true;
    }));
    assert_eq!(t.text(), "a\nb");
}

#[test]
fn sort_only_the_selected_lines() {
    // The selection ends at the start of "a": that line is not included.
    let mut t = setup("z\nc\nb\na");
    t.set_sels(vec![(2, 6)]);
    t.apply(sort(|_| {}));
    assert_eq!(t.text(), "z\nb\nc\na");
    assert_eq!(t.show(), "z\n^b\nc|\na");
}

#[test]
fn sort_by_the_column_rectangle() {
    let mut t = setup("x 3\ny 1\nz 2");
    t.apply(Action::ColumnSelect {
        anchor_line: 0,
        anchor_col: VisualCol(2),
        head_line: 2,
        head_col: VisualCol(3),
    });
    t.apply(sort(|_| {}));
    assert_eq!(t.text(), "y 1\nz 2\nx 3");
}

#[test]
fn trim_trailing_whitespace() {
    let mut t = setup("a  \nb\t\nc|");
    t.apply(Action::TrimTrailingWhitespace);
    assert_eq!(t.show(), "a\nb\nc|");

    let mut t = setup("a  \nb  \nc  ");
    t.set_sels(vec![(4, 5)]);
    t.apply(Action::TrimTrailingWhitespace);
    assert_eq!(t.text(), "a  \nb\nc  ", "only the selected line");
}

#[test]
fn duplicate_line_or_selection() {
    let mut t = setup("ab|c\nd");
    t.apply(Action::DuplicateLine);
    assert_eq!(t.show(), "ab|c\nabc\nd");

    let mut t = setup("abc");
    t.set_sels(vec![(0, 2)]);
    t.apply(Action::DuplicateLine);
    assert_eq!(t.show(), "^ab|abc");
}

#[test]
fn move_lines_up_and_down_with_the_selection() {
    let mut t = setup("one\ntw|o\nthree");
    t.apply(Action::MoveLinesUp);
    assert_eq!(t.show(), "tw|o\none\nthree");
    t.apply(Action::MoveLinesUp);
    assert_eq!(t.show(), "tw|o\none\nthree", "already at the top");
    t.apply(Action::MoveLinesDown).apply(Action::MoveLinesDown);
    assert_eq!(t.show(), "one\nthree\ntw|o");
    t.apply(Action::MoveLinesDown);
    assert_eq!(t.show(), "one\nthree\ntw|o", "already at the bottom");

    // A block of two selected lines, selection ending at a line start.
    let mut t = setup("a\nb\nc\nd");
    t.set_sels(vec![(2, 6)]);
    t.apply(Action::MoveLinesUp);
    assert_eq!(t.show(), "^b\nc\n|a\nd");
    t.apply(Action::Undo);
    assert_eq!(t.text(), "a\nb\nc\nd");
}

#[test]
fn join_lines() {
    let mut t = setup("a  \n   b\nc");
    t.apply(Action::SelectAll).apply(Action::JoinLines);
    assert_eq!(t.text(), "a b c");

    let mut t = setup("x|\n\ny");
    t.apply(Action::JoinLines);
    assert_eq!(
        t.show(),
        "x|\ny",
        "joining with an empty line adds no space"
    );
}

#[test]
fn moving_whole_lines_onto_the_last_line_keeps_the_selection_in_range() {
    // Select All on a file ending in a newline selects up to the start of
    // the empty last line; moving down swaps with that line.
    let mut t = setup("a\nb\n");
    t.apply(Action::SelectAll).apply(Action::MoveLinesDown);
    assert_eq!(t.text(), "\na\nb");
    assert_eq!(t.show(), "\n^a\nb|");
}

fn remove_empty(include_whitespace: bool) -> Action {
    Action::RemoveEmptyLines { include_whitespace }
}

fn remove_duplicates(consecutive_only: bool) -> Action {
    Action::RemoveDuplicateLines { consecutive_only }
}

#[test]
fn remove_empty_lines_keeps_the_final_newline() {
    let mut t = setup("|a\n\nb\n  \n\nc\n");
    t.apply(remove_empty(false));
    assert_eq!(t.text(), "a\nb\n  \nc\n");
    t.apply(Action::Undo);
    assert_eq!(t.text(), "a\n\nb\n  \n\nc\n", "one undo step");

    t.apply(remove_empty(true));
    assert_eq!(t.text(), "a\nb\nc\n", "blank lines go too");
}

#[test]
fn removing_lines_that_end_the_document_takes_the_break_before_them() {
    let mut t = setup("|x\n\n \t");
    t.apply(remove_empty(true));
    assert_eq!(t.text(), "x");

    let mut t = setup("|a\nb\na");
    t.apply(remove_duplicates(false));
    assert_eq!(
        t.text(),
        "a\nb",
        "the last line has no newline but is equal"
    );

    let mut t = setup("|\n\n");
    t.apply(remove_empty(false));
    assert_eq!(t.text(), "");
}

#[test]
fn nothing_to_remove_leaves_the_buffer_clean() {
    let mut t = setup("a\nb|\nc\n");
    t.apply(remove_empty(true)).apply(remove_duplicates(false));
    assert_eq!(t.show(), "a\nb|\nc\n");
    assert!(!t.buffer().is_dirty());
}

#[test]
fn remove_duplicate_lines_keeps_the_first_and_the_order() {
    let mut t = setup("|b\na\nb\nA\na\nb\n");
    t.apply(remove_duplicates(false));
    assert_eq!(t.text(), "b\na\nA\n", "case-sensitive, not sorted");

    let mut t = setup("|a\na\nb\na\na\n\n\n");
    t.apply(remove_duplicates(true));
    assert_eq!(t.text(), "a\nb\na\n\n", "only repeats next to each other");
}

#[test]
fn line_removal_works_on_the_selected_lines_only() {
    // Lines 1-3 selected (the selection ends at the start of line 4).
    let mut t = setup("a\na\n\na\n\na");
    t.set_sels(vec![(2, 7)]);
    t.apply(remove_duplicates(false));
    assert_eq!(t.text(), "a\na\n\n\na");
    t.apply(remove_empty(false));
    assert_eq!(t.text(), "a\na\n\na");
    assert_eq!(t.show(), "a\n^a\n|\na", "the selection keeps the rest");

    // A column selection counts as its lines, even where it is empty.
    let mut t = setup("x\n\ny\n\nz");
    t.apply(Action::ColumnSelect {
        anchor_line: 1,
        anchor_col: VisualCol(0),
        head_line: 2,
        head_col: VisualCol(0),
    });
    t.apply(remove_empty(false));
    assert_eq!(t.text(), "x\ny\n\nz");
}
