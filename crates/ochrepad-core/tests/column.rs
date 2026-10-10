//! Column (rectangular) selection and block paste.

mod util;

use ochrepad_core::{Action, Direction, Motion, VisualCol};
use util::setup;

fn column(anchor_line: usize, anchor_col: usize, head_line: usize, head_col: usize) -> Action {
    Action::ColumnSelect {
        anchor_line,
        anchor_col: VisualCol(anchor_col),
        head_line,
        head_col: VisualCol(head_col),
    }
}

fn ranges(t: &util::T) -> Vec<(usize, usize)> {
    t.sels()
        .ranges()
        .iter()
        .map(|s| (s.anchor.0, s.head.0))
        .collect()
}

#[test]
fn rectangle_spans_tabs_wide_glyphs_and_short_lines() {
    // line starts: 0, 7, 10, 13, 22
    let mut t = setup("abcdef\nab\n\tx\n漢字xy\nabcdef");
    t.apply(column(0, 1, 4, 4));
    assert_eq!(
        ranges(&t),
        vec![
            (1, 4),   // "bcd"
            (8, 9),   // "b", then the line ends
            (11, 11), // the tab straddles column 1: left out, nothing remains
            (16, 19), // 漢 straddles column 1: left out; "字"
            (23, 26),
        ]
    );
    assert_eq!(t.sels().primary_index(), 4, "primary is on the head line");
    let rect = t.sels().rect().cloned().unwrap();
    assert_eq!((rect.top(), rect.bottom()), (0, 4));
    assert_eq!((rect.left(), rect.right()), (VisualCol(1), VisualCol(4)));

    // Any non-column action drops the rectangle.
    t.mv(Motion::Right);
    assert!(t.sels().rect().is_none());
}

#[test]
fn rectangle_can_be_dragged_up_and_left() {
    let mut t = setup("abcdef\nabcdef\nabcdef");
    t.apply(column(2, 4, 0, 1));
    // Anchor at column 4, head at column 1, on every line.
    assert_eq!(ranges(&t), vec![(4, 1), (11, 8), (18, 15)]);
    assert_eq!(t.sels().primary_index(), 0);
}

#[test]
fn alt_shift_arrows_grow_a_rectangle_and_typing_hits_every_line() {
    let mut t = setup("ab|cd\nefgh\nij");
    t.apply(Action::ColumnExtend(Direction::Down));
    t.apply(Action::ColumnExtend(Direction::Down));
    assert_eq!(t.show(), "ab|cd\nef|gh\nij|");
    t.apply(Action::ColumnExtend(Direction::Right));
    assert_eq!(t.show(), "ab^c|d\nef^g|h\nij|");
    t.typ("X");
    assert_eq!(t.show(), "abX|d\nefX|h\nijX|");
    t.apply(Action::DeleteBackward);
    assert_eq!(t.show(), "ab|d\nef|h\nij|");
    // One undo step per keystroke across all lines.
    t.apply(Action::Undo);
    assert_eq!(t.text(), "abXd\nefXh\nijX");
}

#[test]
fn column_extend_left_steps_over_wide_glyphs() {
    let mut t = setup("x漢字|\nabcdef");
    t.apply(Action::ColumnExtend(Direction::Left));
    // From column 5 back over 字 (columns 3..5) to column 3.
    let rect = t.sels().rect().cloned().unwrap();
    assert_eq!(rect.end_col, VisualCol(3));
    t.apply(Action::ColumnExtend(Direction::Down));
    assert_eq!(t.show(), "x漢|字^\nabc|de^f");
}

#[test]
fn block_paste_lands_in_one_column_padding_short_lines() {
    let mut t = setup("a|b\nc\n");
    t.apply(Action::PasteBlock("12\n34\n56".into()));
    assert_eq!(t.text(), "a12b\nc34\n 56");
    assert_eq!(t.show(), "a12|b\nc34\n 56");
}

#[test]
fn block_paste_adds_lines_at_the_end_of_the_document() {
    let mut t = setup("x|");
    t.apply(Action::PasteBlock("1\n2\n3".into()));
    assert_eq!(t.text(), "x1\n 2\n 3");
    t.apply(Action::Undo);
    assert_eq!(t.text(), "x");
}

#[test]
fn block_paste_into_several_carets_distributes_like_paste() {
    let mut t = setup("a|\nb|");
    t.apply(Action::PasteBlock("1\n2".into()));
    assert_eq!(t.text(), "a1\nb2");
}
