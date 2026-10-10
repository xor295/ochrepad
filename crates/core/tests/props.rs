//! Property tests over random documents and random action sequences.

mod util;

use ochrepad_core::position::{byte_to_visual_col, next_grapheme_boundary, visual_col_to_byte};
use ochrepad_core::{Action, ByteOffset, Document, Motion, SelectionSet};
use proptest::prelude::*;

const PIECES: &[&str] = &[
    "a",
    "b",
    "Z",
    "_",
    ".",
    "(",
    " ",
    " ",
    "\t",
    "\n",
    "\n",
    "e\u{301}",
    "漢",
    "👨‍👩‍👧",
    "👍🏽",
    "9",
];

fn text() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(PIECES), 0..60).prop_map(|v| v.concat())
}

fn motion() -> impl Strategy<Value = Motion> {
    prop_oneof![
        Just(Motion::Left),
        Just(Motion::Right),
        Just(Motion::WordLeft),
        Just(Motion::WordRight),
        Just(Motion::Home),
        Just(Motion::LineStart),
        Just(Motion::LineEnd),
        Just(Motion::DocStart),
        Just(Motion::DocEnd),
        Just(Motion::Up),
        Just(Motion::Down),
        (0usize..5).prop_map(Motion::PageUp),
        (0usize..5).prop_map(Motion::PageDown),
    ]
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        4 => prop::sample::select(PIECES).prop_map(|s| Action::InsertText(s.to_owned())),
        1 => text().prop_map(Action::InsertText),
        1 => Just(Action::InsertNewline),
        2 => Just(Action::DeleteBackward),
        1 => Just(Action::DeleteForward),
        1 => Just(Action::DeleteWordBackward),
        1 => Just(Action::DeleteWordForward),
        5 => (motion(), any::<bool>()).prop_map(|(motion, extend)| Action::MoveCursor { motion, extend }),
        1 => Just(Action::SelectAll),
        1 => Just(Action::SelectWord),
        1 => Just(Action::SelectLine),
        1 => Just(Action::AddCursorAbove),
        2 => Just(Action::AddCursorBelow),
        1 => Just(Action::CollapseSelections),
        1 => text().prop_map(Action::Paste),
        1 => prop::collection::vec((0usize..300, 0usize..300), 1..4).prop_map(|v| {
            let ranges = v
                .into_iter()
                .map(|(a, h)| ochrepad_core::Selection::new(ByteOffset(a), ByteOffset(h)))
                .collect();
            Action::SetSelections(SelectionSet::new(ranges, 0))
        }),
        2 => Just(Action::Undo),
        1 => Just(Action::Redo),
    ]
}

fn check(t: &util::T) -> Result<(), TestCaseError> {
    let sels = t.sels();
    sels.validate(t.buffer().doc())
        .map_err(|e| TestCaseError::fail(format!("{e}\nstate: {}", t.show())))
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 3000, ..ProptestConfig::default() })]

    /// Selections stay sorted, non-overlapping, on char boundaries and in
    /// range; undoing everything restores the original text, and the
    /// selections from just before the first edit still in the history.
    #[test]
    fn undo_all_restores_original(initial in text(), actions in prop::collection::vec(action(), 0..40)) {
        let mut t = util::setup(&initial);
        let mut first_edit_sels: Option<SelectionSet> = None;
        for a in actions {
            let before = t.sels();
            let rev = t.buffer().revision();
            let had_undo = t.buffer().history().can_undo();
            let is_redo = a == Action::Redo;
            t.apply(a);
            check(&t)?;
            if !had_undo && !is_redo && t.buffer().revision() != rev {
                first_edit_sels = Some(before);
            }
        }
        let undid_something = t.buffer().history().can_undo();
        while t.buffer().history().can_undo() {
            t.apply(Action::Undo);
            check(&t)?;
        }
        prop_assert_eq!(t.text(), initial);
        prop_assert!(!t.buffer().is_dirty());
        if let Some(s) = first_edit_sels.filter(|_| undid_something) {
            prop_assert_eq!(t.sels(), s);
        }
    }

    /// Undo then redo returns to the same text and revision, and to the same
    /// selections when the last action was an edit.
    #[test]
    fn undo_then_redo_is_identity(initial in text(), actions in prop::collection::vec(action(), 1..40)) {
        let mut t = util::setup(&initial);
        let mut last_was_edit = false;
        for a in actions {
            let rev = t.buffer().revision();
            let is_undo = a == Action::Undo;
            t.apply(a);
            last_was_edit = !is_undo && t.buffer().revision() != rev;
        }
        if !t.buffer().history().can_undo() {
            return Ok(());
        }
        let (text, sels, rev) = (t.text(), t.sels(), t.buffer().revision());
        t.apply(Action::Undo).apply(Action::Redo);
        check(&t)?;
        prop_assert_eq!(t.text(), text);
        prop_assert_eq!(t.buffer().revision(), rev);
        if last_was_edit {
            prop_assert_eq!(t.sels(), sels);
        }
    }

    /// The line-edit log accounts for every change in line count, and every
    /// reported line is in range at the moment it is reported.
    #[test]
    fn line_edits_track_line_count(initial in text(), actions in prop::collection::vec(action(), 0..40)) {
        let mut t = util::setup(&initial);
        let id = t.buffer().id();
        for a in actions {
            let before = t.buffer().doc().line_count() as isize;
            t.apply(a);
            let edits = t.ed.buffer_mut(id).unwrap().take_line_edits();
            let mut lines = before;
            for e in &edits {
                prop_assert!(e.start <= e.old_end && (e.old_end as isize) < lines);
                lines += e.new_end as isize - e.old_end as isize;
            }
            prop_assert_eq!(lines, t.buffer().doc().line_count() as isize);
        }
    }

    /// ByteOffset -> VisualCol -> ByteOffset round-trips at every grapheme
    /// boundary, for any tab width.
    #[test]
    fn visual_col_round_trip(s in text(), tab_width in 1usize..9) {
        let doc = Document::new(&s);
        let mut p = ByteOffset(0);
        loop {
            let col = byte_to_visual_col(&doc, p, tab_width);
            let back = visual_col_to_byte(&doc, doc.line_of(p), col, tab_width);
            prop_assert_eq!(back, p, "col {:?} in {:?}", col, s);
            if p == doc.len() {
                break;
            }
            p = next_grapheme_boundary(&doc, p);
        }
    }
}
