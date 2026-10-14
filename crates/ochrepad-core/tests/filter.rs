//! 0.3.0 P6: the filter view hides lines through the display map.

mod util;

use ochrepad_core::{Action, DisplayMap, Motion};
use util::setup;

fn shown_lines(t: &util::T) -> Vec<usize> {
    let v = t.ed.view(t.view).unwrap();
    let rows = DisplayMap::new(t.buffer(), v);
    let mut lines: Vec<usize> = rows
        .rows_from(rows.first_row(), usize::MAX)
        .map(|r| r.line)
        .collect();
    lines.dedup();
    lines
}

fn mv(motion: Motion) -> Action {
    Action::MoveCursor {
        motion,
        extend: false,
    }
}

const LOG: &str = "INFO a\nERROR b\nINFO c\nINFO d\nERROR e\nINFO f";

#[test]
fn a_filter_shows_only_its_lines_and_moves_carets_onto_them() {
    let mut t = setup(&format!("|{LOG}"));
    t.ed.set_filter(t.view, Some(&[1, 4]));
    assert_eq!(shown_lines(&t), vec![1, 4]);
    // The caret was on hidden line 0: it moved to the nearest shown line.
    assert_eq!(
        t.show(),
        "INFO a\n|ERROR b\nINFO c\nINFO d\nERROR e\nINFO f"
    );
    // Up and Down move between shown lines.
    t.apply(mv(Motion::Down));
    assert_eq!(t.ed.view(t.view).unwrap().selections.primary().head.0, 29);
    t.apply(mv(Motion::Up));
    assert_eq!(t.ed.view(t.view).unwrap().selections.primary().head.0, 7);
    t.ed.set_filter(t.view, None);
    assert_eq!(shown_lines(&t), (0..6).collect::<Vec<_>>());
}

#[test]
fn edits_move_the_filter_and_show_the_lines_they_touch() {
    let mut t = setup("INFO a\n|ERROR b\nINFO c\nINFO d\nERROR e\nINFO f");
    t.ed.set_filter(t.view, Some(&[1, 4]));
    // Two new lines above the shown ones: they show (they were edited), and
    // the hidden lines below move down with the text.
    t.apply(Action::InsertText("new 1\nnew 2\n".into()));
    assert_eq!(shown_lines(&t), vec![1, 2, 3, 6]);
    t.apply(Action::Undo);
    assert_eq!(shown_lines(&t), vec![1, 4]);
    t.apply(Action::Redo);
    assert_eq!(shown_lines(&t), vec![1, 2, 3, 6]);
}

#[test]
fn going_to_a_hidden_line_shows_it() {
    let mut t = setup(&format!("|{LOG}"));
    t.ed.set_filter(t.view, Some(&[1, 4]));
    t.apply(Action::GoToLine { line: 3, column: 1 });
    assert_eq!(shown_lines(&t), vec![1, 2, 4]);
}

#[test]
fn hiding_the_first_lines_and_folding_work_together() {
    let mut t = setup("a\n|b\n  c\n  d\ne\nf");
    // Shows lines 1..=4: line 0 is hidden, so the first row is line 1.
    t.ed.set_filter(t.view, Some(&[1, 2, 3, 4]));
    assert_eq!(shown_lines(&t), vec![1, 2, 3, 4]);
    // Folding "b" hides its block as well.
    assert!(t.ed.fold_at(t.view, 1));
    assert_eq!(shown_lines(&t), vec![1, 4]);
    t.apply(mv(Motion::DocStart));
    assert_eq!(
        shown_lines(&t),
        vec![0, 1, 4],
        "the caret's line comes back"
    );
}

#[test]
fn a_filter_matching_nothing_keeps_the_caret_line() {
    let mut t = setup("INFO a\nERROR b\n|INFO c");
    t.ed.set_filter(t.view, Some(&[]));
    assert_eq!(shown_lines(&t), vec![2]);
}

#[test]
fn appended_text_is_not_an_undo_step_and_keeps_the_buffer_clean() {
    let mut t = setup("one|");
    t.apply(Action::InsertText(" two".into()));
    let b = t.ed.view(t.view).unwrap().buffer();
    t.ed.buffer_mut(b).unwrap().mark_saved();
    // The file grows on disk while the caret is at the end: it follows.
    t.ed.append_unrecorded(b, " three").unwrap();
    assert_eq!(t.show(), "one two three|");
    assert!(!t.buffer().is_dirty());
    // Undo takes back the typing, not the appended text.
    t.apply(Action::Undo);
    assert_eq!(t.text(), "one three");
    assert!(t.buffer().is_dirty());
    t.apply(Action::Redo);
    assert_eq!(t.text(), "one two three");
    assert!(!t.buffer().is_dirty(), "back at the text on disk");
    // A caret elsewhere stays put.
    t.apply(Action::SetSelections(ochrepad_core::SelectionSet::caret(
        ochrepad_core::ByteOffset(0),
    )));
    t.ed.append_unrecorded(b, "\nfour").unwrap();
    assert_eq!(t.show(), "|one two three\nfour");
    assert!(!t.buffer().is_dirty());
}
