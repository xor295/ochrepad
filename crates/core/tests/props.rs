//! Property tests over random documents and random action sequences.

mod util;

use ochrepad_core::position::{byte_to_visual_col, next_grapheme_boundary, visual_col_to_byte};
use ochrepad_core::{Action, ByteOffset, Document, Motion, SelectionSet};
use proptest::prelude::*;

const PIECES: &[&str] = &[
    "ß",
    "İ",
    "ǅ",
    "1",
    "-2.5",
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

/// Short replacement text: Replace All over every character with a long
/// replacement grows the document exponentially across a sequence.
fn short_text() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(PIECES), 0..3).prop_map(|v| v.concat())
}

fn search_query() -> impl Strategy<Value = ochrepad_core::SearchQuery> {
    let pattern = prop_oneof![
        prop::sample::select(PIECES).prop_map(str::to_owned),
        prop::sample::select(vec!["^", "$", r"\w+", r"(a)(b)?", "a*", r"\s", "."])
            .prop_map(str::to_owned),
    ];
    (pattern, any::<[bool; 5]>()).prop_map(|(pattern, b)| {
        ochrepad_core::SearchQuery::new(
            pattern,
            ochrepad_core::SearchOptions {
                regex: b[0],
                match_case: b[1],
                whole_word: b[2],
                wrap: b[3],
                in_selection: b[4],
            },
        )
    })
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
        1 => text().prop_map(Action::PasteBlock),
        2 => prop::sample::select(vec![
            ochrepad_core::CaseMode::Upper,
            ochrepad_core::CaseMode::Lower,
            ochrepad_core::CaseMode::Title,
            ochrepad_core::CaseMode::Invert,
        ])
        .prop_map(Action::ConvertCase),
        1 => any::<[bool; 4]>().prop_map(|b| Action::SortLines(ochrepad_core::SortOptions {
            descending: b[0],
            case_insensitive: b[1],
            numeric: b[2],
            remove_duplicates: b[3],
        })),
        1 => prop::sample::select(vec![
            Action::TrimTrailingWhitespace,
            Action::DuplicateLine,
            Action::MoveLinesUp,
            Action::MoveLinesDown,
            Action::JoinLines,
        ]),
        2 => (search_query(), short_text()).prop_map(|(query, replacement)| Action::ReplaceAll { query, replacement }),
        1 => (search_query(), short_text()).prop_map(|(query, replacement)| Action::Replace { query, replacement }),
        1 => (search_query(), any::<bool>()).prop_map(|(query, backward)| Action::Find { query, backward }),
        1 => search_query().prop_map(Action::SelectAllMatches),
        2 => (0usize..12, 0usize..20, 0usize..12, 0usize..20).prop_map(|(al, ac, hl, hc)| {
            Action::ColumnSelect {
                anchor_line: al,
                anchor_col: ochrepad_core::VisualCol(ac),
                head_line: hl,
                head_col: ochrepad_core::VisualCol(hc),
            }
        }),
        2 => prop::sample::select(vec![
            ochrepad_core::Direction::Up,
            ochrepad_core::Direction::Down,
            ochrepad_core::Direction::Left,
            ochrepad_core::Direction::Right,
        ])
        .prop_map(Action::ColumnExtend),
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
            // Search actions may fail (no match); that is fine here.
            let _ = t.ed.apply(t.view, a);
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
            // Search actions may fail (no match); that is fine here.
            let _ = t.ed.apply(t.view, a);
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
            // Search actions may fail (no match); that is fine here.
            let _ = t.ed.apply(t.view, a);
            let edits = t.ed.buffer_mut(id).unwrap().take_line_edits();
            // Very many edits at once collapse into a conservative
            // "everything from line N" entry, which carries no line counts.
            if edits.iter().any(|e| e.is_everything()) {
                continue;
            }
            let mut lines = before;
            for e in &edits {
                prop_assert!(e.start <= e.old_end && (e.old_end as isize) < lines);
                lines += e.new_end as isize - e.old_end as isize;
            }
            prop_assert_eq!(lines, t.buffer().doc().line_count() as isize);
        }
    }

    /// Every selection of a column selection lies inside the rectangle's
    /// columns, on its own line of the rectangle.
    #[test]
    fn column_selections_stay_inside_the_rectangle(
        s in text(),
        (al, ac, hl, hc) in (0usize..12, 0usize..20, 0usize..12, 0usize..20),
    ) {
        let mut t = util::setup(&s);
        t.apply(Action::ColumnSelect {
            anchor_line: al,
            anchor_col: ochrepad_core::VisualCol(ac),
            head_line: hl,
            head_col: ochrepad_core::VisualCol(hc),
        });
        check(&t)?;
        let sels = t.sels();
        let rect = sels.rect().cloned().expect("column selection keeps its rectangle");
        let doc = t.buffer().doc();
        prop_assert_eq!(sels.len(), rect.bottom() - rect.top() + 1);
        for (i, sel) in sels.ranges().iter().enumerate() {
            let line = rect.top() + i;
            prop_assert_eq!(doc.line_of(sel.start()), line);
            prop_assert_eq!(doc.line_of(sel.end()), line);
            let end_col = byte_to_visual_col(doc, sel.end(), 4);
            prop_assert!(end_col <= rect.right());
            if !sel.is_empty() {
                prop_assert!(byte_to_visual_col(doc, sel.start(), 4) >= rect.left());
            }
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

proptest! {
    #![proptest_config(ProptestConfig { cases: 2000, ..ProptestConfig::default() })]

    /// The aligned rows list every line of each side exactly once, in
    /// order; equal rows have equal text; hunks cover exactly the non-equal
    /// rows; word ranges lie on char boundaries.
    #[test]
    fn diff_rows_cover_both_sides(a in text(), b in text()) {
        use ochrepad_core::diff::{RowKind, diff_lines, diff_words};
        let (left, right) = (Document::new(&a), Document::new(&b));
        let d = diff_lines(&left, &right);
        let ls: Vec<usize> = d.rows.iter().filter_map(|r| r.left).collect();
        let rs: Vec<usize> = d.rows.iter().filter_map(|r| r.right).collect();
        prop_assert_eq!(ls, (0..left.line_count()).collect::<Vec<_>>());
        prop_assert_eq!(rs, (0..right.line_count()).collect::<Vec<_>>());
        let mut in_hunk = vec![false; d.rows.len()];
        for h in &d.hunks {
            for i in h.clone() {
                in_hunk[i] = true;
            }
        }
        for (i, r) in d.rows.iter().enumerate() {
            prop_assert_eq!(in_hunk[i], r.kind != RowKind::Equal);
            if let (Some(l), Some(rr)) = (r.left, r.right) {
                let (lt, rt) = (left.line_text(l), right.line_text(rr));
                if r.kind == RowKind::Equal {
                    prop_assert_eq!(&lt, &rt);
                } else {
                    let (wa, wb) = diff_words(&lt, &rt);
                    for w in &wa {
                        prop_assert!(lt.is_char_boundary(w.start) && lt.is_char_boundary(w.end));
                    }
                    for w in &wb {
                        prop_assert!(rt.is_char_boundary(w.start) && rt.is_char_boundary(w.end));
                    }
                }
            }
        }
    }
}
