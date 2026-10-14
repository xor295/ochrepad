//! Notepad++'s line and blank operations, and the Column Editor.

mod util;

use ochrepad_core::{Action, ColumnInsert, Leading, NumberFormat, NumberSequence, VisualCol};
use util::setup;

fn rect(t: &mut util::T, lines: (usize, usize), cols: (usize, usize)) {
    t.apply(Action::ColumnSelect {
        anchor_line: lines.0,
        anchor_col: VisualCol(cols.0),
        head_line: lines.1,
        head_col: VisualCol(cols.1),
    });
}

#[test]
fn delete_line_removes_the_carets_lines() {
    let mut t = setup("a\nb|\nc");
    t.apply(Action::DeleteLine);
    assert_eq!(t.show(), "a\n|c");
    t.apply(Action::Undo);
    assert_eq!(t.show(), "a\nb|\nc");

    // The last line takes the line break before it; so does the empty line
    // after a final line break.
    let mut t = setup("a\n|b");
    t.apply(Action::DeleteLine);
    assert_eq!(t.show(), "a|");
    let mut t = setup("a\n|");
    t.apply(Action::DeleteLine);
    assert_eq!(t.show(), "a|");

    // Several carets, and every line a selection touches.
    let mut t = setup("a|\nb\nc|");
    t.apply(Action::DeleteLine);
    assert_eq!(t.text(), "b");
    let mut t = setup("a\nb\nc\nd");
    t.set_sels(vec![(3, 5)]);
    t.apply(Action::DeleteLine);
    assert_eq!(t.text(), "a\nd");

    // An empty document has nothing to delete.
    let mut t = setup("|");
    t.apply(Action::DeleteLine);
    assert_eq!(t.text(), "");
    assert!(!t.buffer().is_dirty());
}

#[test]
fn delete_line_takes_a_rectangles_lines() {
    let mut t = setup("ab\ncd\nef\n");
    rect(&mut t, (0, 1), (1, 2));
    t.apply(Action::DeleteLine);
    assert_eq!(t.text(), "ef\n");
    assert!(t.sels().rect().is_none());
}

#[test]
fn transpose_swaps_with_the_line_above_and_keeps_the_caret_on_its_line() {
    let mut t = setup("a\nbb|\nc");
    t.apply(Action::TransposeLines);
    assert_eq!(t.show(), "bb\n|a\nc");
    // Again: back as it was, the caret at the start of line 2.
    t.apply(Action::TransposeLines);
    assert_eq!(t.show(), "a\n|bb\nc");

    // The first line has nothing above it.
    let mut t = setup("a|\nb");
    t.apply(Action::TransposeLines);
    assert_eq!(t.text(), "a\nb");
    assert!(!t.buffer().is_dirty());

    // The last line without a line break.
    let mut t = setup("a\nb|");
    t.apply(Action::TransposeLines);
    assert_eq!(t.show(), "b\n|a");

    // Carets on neighbouring lines: the second line's swap would overlap
    // the first's, so only the first happens.
    let mut t = setup("a\nb|\nc|");
    t.apply(Action::TransposeLines);
    assert_eq!(t.show(), "b\n|a\nc|");
    // Lines further apart both swap, as one undo step.
    let mut t = setup("a\nb|\nc\nd|");
    t.apply(Action::TransposeLines);
    assert_eq!(t.show(), "b\n|a\nd\n|c");
    t.apply(Action::Undo);
    assert_eq!(t.text(), "a\nb\nc\nd");

    // A selection on the line above a swapped one moves with its text (its
    // byte offsets would otherwise land inside other characters).
    let mut t = setup("ß\nab");
    t.set_sels(vec![(0, 2), (3, 4)]);
    t.apply(Action::TransposeLines);
    assert_eq!(t.show(), "ab\n^ß|");
}

#[test]
fn insert_line_above_and_below_keep_the_indentation() {
    let mut t = setup("  a|b\nc");
    t.apply(Action::InsertLineAbove);
    assert_eq!(t.show(), "  |\n  ab\nc");
    let mut t = setup("  a|b\nc");
    t.apply(Action::InsertLineBelow);
    assert_eq!(t.show(), "  ab\n  |\nc");

    // One line each for several carets; a selection uses its first or last
    // line.
    let mut t = setup("x|\ny|");
    t.apply(Action::InsertLineBelow);
    assert_eq!(t.show(), "x\n|\ny\n|");
    let mut t = setup("\ta\n\tb\nc");
    t.set_sels(vec![(1, 5)]);
    t.apply(Action::InsertLineAbove);
    assert_eq!(t.show(), "\t|\n\ta\n\tb\nc");
    let mut t = setup("\ta\n\tb\nc");
    t.set_sels(vec![(1, 5)]);
    t.apply(Action::InsertLineBelow);
    assert_eq!(t.show(), "\ta\n\tb\n\t|\nc");

    // An empty document.
    let mut t = setup("|");
    t.apply(Action::InsertLineAbove);
    assert_eq!(t.show(), "|\n");
    let mut t = setup("|");
    t.apply(Action::InsertLineBelow);
    assert_eq!(t.show(), "\n|");
}

#[test]
fn reverse_line_order() {
    let mut t = setup("|a\nb\nc\n");
    t.apply(Action::ReverseLines);
    assert_eq!(t.show(), "|c\nb\na\n");
    t.apply(Action::Undo);
    assert_eq!(t.text(), "a\nb\nc\n");

    // Only the selected lines.
    let mut t = setup("x\na\nb\nc\ny");
    t.set_sels(vec![(2, 7)]);
    t.apply(Action::ReverseLines);
    assert_eq!(t.show(), "x\n^c\nb\na|\ny");

    // A rectangle's lines, whole.
    let mut t = setup("ab\ncd\nef");
    rect(&mut t, (0, 1), (1, 1));
    t.apply(Action::ReverseLines);
    assert_eq!(t.text(), "cd\nab\nef");

    // Nothing to reverse in one line, or in lines that read the same.
    let mut t = setup("one|");
    t.apply(Action::ReverseLines);
    assert!(!t.buffer().is_dirty());
    let mut t = setup("|a\na");
    t.apply(Action::ReverseLines);
    assert!(!t.buffer().is_dirty());
}

#[test]
fn trim_leading_and_both_ends() {
    let text = "  a  \n\t b\t\n   \nc";
    let mut t = setup(&format!("|{text}"));
    t.apply(Action::TrimLeadingWhitespace);
    assert_eq!(t.text(), "a  \nb\t\n\nc");
    let mut t = setup(&format!("|{text}"));
    t.apply(Action::TrimLeadingAndTrailingWhitespace);
    assert_eq!(t.text(), "a\nb\n\nc");
    // One undo step.
    t.apply(Action::Undo);
    assert_eq!(t.text(), text);

    // Only the selected line; the caret stays on its text.
    let mut t = setup("  a\n  b\n  c");
    t.set_sels(vec![(4, 7)]);
    t.apply(Action::TrimLeadingWhitespace);
    assert_eq!(t.text(), "  a\nb\n  c");
}

#[test]
fn eol_to_space_joins_without_trimming() {
    let mut t = setup("|a \nb\n c\n");
    t.apply(Action::EolToSpace);
    assert_eq!(t.text(), "a  b  c\n", "the final line break stays");
    t.apply(Action::Undo);

    let mut t = setup("a\nb\nc\nd");
    t.set_sels(vec![(2, 5)]);
    t.apply(Action::EolToSpace);
    assert_eq!(t.text(), "a\nb c\nd");

    // One selected line has no line break to replace.
    let mut t = setup("a\nb");
    t.set_sels(vec![(0, 1)]);
    t.apply(Action::EolToSpace);
    assert!(!t.buffer().is_dirty());
}

#[test]
fn tabs_to_spaces_keeps_every_column() {
    let mut t = setup("|\ta\tb\n漢\tx");
    t.apply(Action::TabsToSpaces);
    assert_eq!(t.text(), "    a   b\n漢  x");
    t.apply(Action::Undo);
    assert_eq!(t.text(), "\ta\tb\n漢\tx");
    // A caret after a tab stays after its spaces.
    let mut t = setup("\t|a");
    t.apply(Action::TabsToSpaces);
    assert_eq!(t.show(), "    |a");
}

#[test]
fn spaces_to_tabs_wherever_columns_stay() {
    let mut t = setup("|        a  b    c\nabc d\n  \t x\n");
    t.apply(Action::SpacesToTabs);
    assert_eq!(t.text(), "\t\ta  b\tc\nabc d\n\t x\n");
    // Tabs to spaces gives the same columns back.
    t.apply(Action::TabsToSpaces);
    assert_eq!(t.text(), "        a  b    c\nabc d\n     x\n");
}

fn numbers(f: impl FnOnce(&mut NumberSequence)) -> Action {
    let mut s = NumberSequence::default();
    f(&mut s);
    Action::ColumnInsert(ColumnInsert::Numbers(s))
}

#[test]
fn column_editor_at_a_rectangle_pads_short_lines() {
    let mut t = setup("ab\nc\ndefg");
    rect(&mut t, (0, 2), (2, 2));
    t.apply(Action::ColumnInsert(ColumnInsert::Text("X".into())));
    assert_eq!(t.show(), "abX|\nc X|\ndeX|fg");
    t.apply(Action::Undo);
    assert_eq!(t.text(), "ab\nc\ndefg");

    let mut t = setup("ab\nc\ndefg");
    rect(&mut t, (0, 2), (2, 2));
    t.apply(numbers(|_| {}));
    assert_eq!(t.text(), "ab1\nc 2\nde3fg");

    // The text inside a rectangle is replaced.
    let mut t = setup("abcd\nefgh");
    rect(&mut t, (0, 1), (1, 3));
    t.apply(Action::ColumnInsert(ColumnInsert::Text("X".into())));
    assert_eq!(t.text(), "aXd\neXh");
}

#[test]
fn column_editor_from_one_caret_fills_down_to_the_end() {
    let mut t = setup("|a\nb\nc\n");
    t.apply(Action::ColumnInsert(ColumnInsert::Text("> ".into())));
    assert_eq!(t.show(), "> |a\n> b\n> c\n");

    let mut t = setup("ab\nc|d\nef");
    t.apply(numbers(|s| {
        s.initial = 9;
        s.leading = Leading::Zeros;
    }));
    assert_eq!(t.show(), "ab\nc09|d\ne10f");

    // Several carets: one number each, in order.
    let mut t = setup("a|b|c");
    t.apply(numbers(|s| {
        s.format = NumberFormat::Hex;
        s.initial = 10;
    }));
    assert_eq!(t.show(), "aA|bB|c");

    // One selection: each of its lines, at its start column.
    let mut t = setup("x1\nx2\nx3");
    t.set_sels(vec![(1, 4)]);
    t.apply(Action::ColumnInsert(ColumnInsert::Text("-".into())));
    assert_eq!(t.text(), "x-1\nx-2\nx3");

    // Empty text inserts nothing.
    let mut t = setup("|a");
    t.apply(Action::ColumnInsert(ColumnInsert::Text(String::new())));
    assert!(!t.buffer().is_dirty());
}

#[test]
fn new_actions_round_trip_through_json() {
    for (action, json) in [
        (Action::DeleteLine, r#""DeleteLine""#),
        (Action::TransposeLines, r#""TransposeLines""#),
        (Action::InsertLineAbove, r#""InsertLineAbove""#),
        (Action::EolToSpace, r#""EolToSpace""#),
        (
            Action::ColumnInsert(ColumnInsert::Text("x".into())),
            r#"{"ColumnInsert":{"Text":"x"}}"#,
        ),
    ] {
        assert_eq!(serde_json::to_string(&action).unwrap(), json);
        assert_eq!(serde_json::from_str::<Action>(json).unwrap(), action);
    }
    // Number options left out take their defaults.
    let a: Action = serde_json::from_str(r#"{"ColumnInsert":{"Numbers":{"initial":5}}}"#).unwrap();
    assert_eq!(
        a,
        numbers(|s| s.initial = 5),
        "increase 1, repeat 1, no padding, decimal"
    );
}

#[test]
fn column_editor_refuses_padding_out_to_an_absurd_column() {
    // A hand-written macro: a rectangle a trillion columns wide.
    let mut t = setup("|a\nb\nc");
    t.apply(Action::ColumnSelect {
        anchor_line: 0,
        anchor_col: VisualCol(1_000_000_000_000),
        head_line: 2,
        head_col: VisualCol(1_000_000_000_000),
    });
    t.apply(Action::ColumnInsert(ColumnInsert::Text("X".into())));
    assert_eq!(t.text(), "a\nb\nc");
}
