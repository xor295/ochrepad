//! Word wrap: motions through wrapped rows.

mod util;

use ochrepad_core::{Action, ByteOffset, Motion};
use util::{T, setup};

/// `marked` wrapped at `cols` cells.
fn wrapped(marked: &str, cols: usize) -> T {
    let mut t = setup(marked);
    t.ed.view_mut(t.view).unwrap().wrap_cols = Some(cols);
    t
}

fn head(t: &T) -> usize {
    t.sels().primary().head.0
}

// Line 0 wraps at 10 as "aaaa bbbb " / "cccc dddd " / "eeee".
const TEXT: &str = "aaaa bbbb cccc dddd eeee\nshort";

#[test]
fn up_and_down_move_by_rows_keeping_the_column() {
    let mut t = wrapped(TEXT, 10);
    t.set_sels(vec![(2, 2)]);
    t.mv(Motion::Down);
    assert_eq!(head(&t), 12, "the second row, column 2");
    t.mv(Motion::Down);
    assert_eq!(head(&t), 22);
    t.mv(Motion::Down);
    assert_eq!(head(&t), 27, "the next line");
    t.mv(Motion::Up).mv(Motion::Up);
    assert_eq!(head(&t), 12, "the remembered column");
}

#[test]
fn moving_to_a_shorter_row_stops_before_the_wrap_point() {
    // Rows "aaaaaaaa " / "b " / "cccccccccc". The second row is shorter
    // than column 5; its end is the wrap point, which shows on the row
    // after, so the caret stops one grapheme before it.
    let mut t = wrapped("aaaaaaaa b cccccccccc", 10);
    t.set_sels(vec![(5, 5)]);
    t.mv(Motion::Down);
    assert_eq!(head(&t), 10);
}

#[test]
fn home_and_end_go_to_the_row_then_the_line() {
    let mut t = wrapped(TEXT, 10);
    t.set_sels(vec![(12, 12)]);
    t.mv(Motion::RowEnd);
    assert_eq!(head(&t), 19, "end of the row, before its wrap point");
    t.mv(Motion::RowEnd);
    assert_eq!(head(&t), 24, "end of the line");
    t.set_sels(vec![(12, 12)]);
    t.mv(Motion::RowHome);
    assert_eq!(head(&t), 10, "start of the row");
    t.mv(Motion::RowHome);
    assert_eq!(head(&t), 0, "then smart home on the line");
}

#[test]
fn without_wrap_row_motions_are_line_motions() {
    let mut t = setup("  ab|cd\nx");
    t.mv(Motion::RowEnd);
    assert_eq!(head(&t), 6);
    t.mv(Motion::RowHome);
    assert_eq!(head(&t), 2, "smart home: first non-blank");
    t.mv(Motion::RowHome);
    assert_eq!(head(&t), 0);
}

#[test]
fn wrapped_rows_and_folds_together() {
    let mut t = wrapped("head\n  aaaa bbbb cccc dddd\ntail", 10);
    t.ed.fold_at(t.view, 0);
    t.set_sels(vec![(0, 0)]);
    t.mv(Motion::Down);
    assert_eq!(
        t.buffer().doc().line_of(ByteOffset(head(&t))),
        2,
        "over the fold"
    );
    t.apply(Action::UnfoldAll);
    t.mv(Motion::Up);
    // The indented line wraps as "  aaaa " / "bbbb " / "cccc " / "dddd".
    // Its last row is indented by 2, so column 0 is in the indent and lands
    // on the row's start.
    assert_eq!(head(&t), 5 + 17);
}

#[test]
fn add_cursor_below_uses_rows() {
    let mut t = wrapped(TEXT, 10);
    t.set_sels(vec![(1, 1)]);
    t.apply(Action::AddCursorBelow);
    let heads: Vec<usize> = t.sels().ranges().iter().map(|s| s.head.0).collect();
    assert_eq!(heads, vec![1, 11]);
}

#[test]
fn padding_rows_are_blank_and_carets_skip_them() {
    // Line 1 has two blank rows above it, and the last line one below.
    let mut t = setup("ab\ncd\nef");
    t.ed.view_mut(t.view).unwrap().padding = Some(std::sync::Arc::new(ochrepad_core::Padding {
        before: vec![0, 2, 0],
        after_last: 1,
    }));
    let view = t.ed.view(t.view).unwrap();
    let rows = ochrepad_core::DisplayMap::new(t.buffer(), view, 4);
    let all: Vec<_> = rows.rows_from(rows.first_row(), 99).collect();
    assert_eq!(all.len(), 6);
    let blank: Vec<bool> = all.iter().map(|&r| rows.is_filler(r)).collect();
    assert_eq!(blank, vec![false, true, true, false, false, true]);
    assert_eq!(rows.row_of(ByteOffset(4)), ochrepad_core::RowPos::new(1, 2));

    // Down from line 0 skips the blank rows; Down from the last line stays.
    t.set_sels(vec![(1, 1)]);
    t.mv(Motion::Down);
    assert_eq!(head(&t), 4);
    t.mv(Motion::Down).mv(Motion::Down);
    assert_eq!(head(&t), 7);
    t.mv(Motion::Up).mv(Motion::Up);
    assert_eq!(head(&t), 1);
}

#[test]
fn read_only_buffers_refuse_edits_but_not_selections() {
    let mut t = setup("abc|");
    let b = t.ed.view(t.view).unwrap().buffer();
    t.ed.buffer_mut(b).unwrap().read_only = true;
    let r = t.ed.apply(t.view, Action::InsertText("x".into()));
    assert_eq!(r, Err(ochrepad_core::ActionError::ReadOnly));
    assert_eq!(
        t.ed.apply(t.view, Action::Undo),
        Err(ochrepad_core::ActionError::ReadOnly)
    );
    t.mv(Motion::Left);
    assert_eq!(head(&t), 2);
    assert_eq!(t.text(), "abc");
}

#[test]
fn aux_views_follow_edits_but_are_not_listed() {
    let mut t = setup("abc|");
    let b = t.ed.view(t.view).unwrap().buffer();
    let aux = t.ed.add_aux_view(b).unwrap();
    assert_eq!(t.ed.views_of(b).count(), 1);
    // A new view's caret is at the start.
    t.ed.apply(aux, Action::InsertText("!".into())).unwrap();
    assert_eq!(t.text(), "!abc");
    assert_eq!(head(&t), 4, "the tab's caret moved with the text");
    t.ed.remove_buffer(b);
    assert!(t.ed.view(aux).is_none());
}
