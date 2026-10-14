//! Property tests over random documents and random action sequences.

mod util;

use ochrepad_core::position::{byte_to_visual_col, next_grapheme_boundary, visual_col_to_byte};
use ochrepad_core::text_tools::{format_json, minify_json, validate_json};
use ochrepad_core::{Action, ByteOffset, Codec, Document, Indent, Motion, SelectionSet};
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
        Just(Motion::RowHome),
        Just(Motion::RowEnd),
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
            ochrepad_core::CaseMode::TitleBlend,
            ochrepad_core::CaseMode::Sentence,
            ochrepad_core::CaseMode::SentenceBlend,
            ochrepad_core::CaseMode::Invert,
            ochrepad_core::CaseMode::Random { seed: 3 },
        ])
        .prop_map(Action::ConvertCase),
        1 => any::<[bool; 5]>().prop_map(|b| Action::SortLines(ochrepad_core::SortOptions {
            descending: b[0],
            case_insensitive: b[1],
            numeric: b[2],
            remove_duplicates: b[3],
            by_length: b[4],
        })),
        1 => any::<u64>().prop_map(|seed| Action::ShuffleLines { seed }),
        1 => prop::sample::select(vec![
            Action::TrimTrailingWhitespace,
            Action::DuplicateLine,
            Action::MoveLinesUp,
            Action::MoveLinesDown,
            Action::JoinLines,
            Action::RemoveEmptyLines { include_whitespace: false },
            Action::RemoveEmptyLines { include_whitespace: true },
            Action::RemoveDuplicateLines { consecutive_only: false },
            Action::RemoveDuplicateLines { consecutive_only: true },
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
        2 => prop_oneof![
            Just(Action::Fold),
            Just(Action::Unfold),
            Just(Action::ToggleFold),
            Just(Action::FoldAll),
            Just(Action::UnfoldAll),
            (1usize..4).prop_map(Action::FoldLevel),
            (1usize..4).prop_map(Action::UnfoldLevel),
        ],
        2 => prop_oneof![
            (0usize..14, 0usize..12).prop_map(|(line, column)| Action::GoToLine { line, column }),
            Just(Action::Indent),
            Just(Action::Unindent),
            any::<bool>().prop_map(|to_spaces| Action::ConvertIndentation { to_spaces }),
            Just(Action::ToggleLineComment),
            (1usize..30).prop_map(|width| Action::SplitLines { width }),
            Just(Action::CommentLines),
            Just(Action::UncommentLines),
            Just(Action::ToggleBlockComment),
            Just(Action::GoToMatchingBracket),
            Just(Action::SelectToMatchingBracket),
        ],
        // Notepad++'s line and blank operations, and the Column Editor.
        1 => prop::sample::select(vec![
            Action::DeleteLine,
            Action::TransposeLines,
            Action::InsertLineAbove,
            Action::InsertLineBelow,
            Action::ReverseLines,
            Action::TrimLeadingWhitespace,
            Action::TrimLeadingAndTrailingWhitespace,
            Action::EolToSpace,
            Action::TabsToSpaces,
            Action::SpacesToTabs,
        ]),
        1 => prop_oneof![
            short_text().prop_map(|t| Action::ColumnInsert(ochrepad_core::ColumnInsert::Text(t))),
            (-5i64..20, -3i64..4, 0usize..3, 0usize..3, 0usize..4).prop_map(
                |(initial, increase, repeat, leading, format)| {
                    use ochrepad_core::{Leading, NumberFormat};
                    Action::ColumnInsert(ochrepad_core::ColumnInsert::Numbers(
                        ochrepad_core::NumberSequence {
                            initial,
                            increase,
                            repeat,
                            leading: [Leading::None, Leading::Zeros, Leading::Spaces][leading],
                            format: [
                                NumberFormat::Dec,
                                NumberFormat::Hex,
                                NumberFormat::Oct,
                                NumberFormat::Bin,
                            ][format],
                        },
                    ))
                }
            ),
        ],
        // Text tools, and JSON for them to work on.
        1 => prop::sample::select(vec!["[1,2]", "{\"a\":[]}", " ", "\"x\"", ",", "{", "]"])
            .prop_map(|s| Action::InsertText(s.to_owned())),
        1 => prop_oneof![
            prop::option::of((any::<bool>(), 1usize..5)).prop_map(|i| Action::FormatJson {
                indent: i.map(|(use_spaces, width)| ochrepad_core::Indent { use_spaces, width }),
            }),
            Just(Action::MinifyJson),
            prop::sample::select(vec![Codec::Base64, Codec::Url, Codec::Hex]).prop_map(Action::Encode),
            prop::sample::select(vec![Codec::Base64, Codec::Url, Codec::Hex]).prop_map(Action::Decode),
        ],
        // Bookmarks, and the commands on the lines they mark.
        2 => prop_oneof![
            Just(Action::ToggleBookmark),
            Just(Action::NextBookmark),
            Just(Action::PrevBookmark),
            Just(Action::ClearBookmarks),
            Just(Action::InverseBookmarks),
            Just(Action::RemoveBookmarkedLines),
            Just(Action::RemoveUnbookmarkedLines),
            short_text().prop_map(Action::ReplaceBookmarkedLines),
            prop::sample::select(vec!["a", "b", "\\n"]).prop_map(|p| {
                Action::BookmarkMatches(ochrepad_core::SearchQuery::new(
                    p,
                    ochrepad_core::SearchOptions {
                        regex: true,
                        ..Default::default()
                    },
                ))
            }),
        ],
    ]
}

/// Random JSON values, with blank space between tokens.
fn json_value() -> impl Strategy<Value = String> {
    let leaf = prop_oneof![
        prop::sample::select(vec![
            "null",
            "true",
            "false",
            "-0.5e3",
            "0",
            "12.25",
            "\"\"",
            "\"a b\"",
            "\"\\\"q\\u00e9\\n\"",
            "\"漢👍\"",
        ])
        .prop_map(str::to_owned),
        (0u32..1000).prop_map(|n| n.to_string()),
    ];
    leaf.prop_recursive(4, 32, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4)
                .prop_map(|v| format!("[ {} ]", v.join(" ,\n"))),
            prop::collection::vec(inner, 0..4).prop_map(|v| {
                let members: Vec<String> = v
                    .iter()
                    .enumerate()
                    .map(|(i, x)| format!("\"k{i}\" :\t{x}"))
                    .collect();
                format!("{{{}}}", members.join(","))
            }),
        ]
    })
}

/// Strings of JSON tokens and near-tokens, mostly invalid.
fn json_ish() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(vec![
            "{", "}", "[", "]", ":", ",", "\"a\"", "\"", "\\", "1", "-", "0", ".", " ", "\n",
            "true", "nul", "x", "\t",
        ]),
        0..20,
    )
    .prop_map(|v| v.concat())
}

fn check(t: &util::T) -> Result<(), TestCaseError> {
    let sels = t.sels();
    let doc = t.buffer().doc();
    sels.validate(doc)
        .map_err(|e| TestCaseError::fail(format!("{e}\nstate: {}", t.show())))?;
    // Bookmarks: sorted, apart, on lines of the text.
    let marks = t.buffer().bookmarks().lines();
    prop_assert!(marks.windows(2).all(|w| w[0] < w[1]), "{marks:?}");
    prop_assert!(
        marks.iter().all(|&l| l < doc.line_count()),
        "{marks:?} in {} lines",
        doc.line_count()
    );
    // Folds: no caret is hidden, and the display rows are the lines that are
    // not hidden, in order.
    let view = t.ed.view(t.view).unwrap();
    let folds = view.folds();
    for s in sels.ranges() {
        prop_assert!(
            !folds.is_hidden(doc.line_of(s.head)),
            "hidden caret: {}",
            t.show()
        );
    }
    let rows = ochrepad_core::DisplayMap::new(t.buffer(), view);
    // Folds do not apply to a padded (compare) view.
    let padded = view.padding.is_some();
    let visible: Vec<usize> = (0..doc.line_count())
        .filter(|&l| padded || !folds.is_hidden(l))
        .collect();
    prop_assert_eq!(rows.visible_line_count(), visible.len());
    let all: Vec<_> = rows.rows_from(rows.first_row(), usize::MAX).collect();
    let mut shown: Vec<usize> = all.iter().map(|r| r.line).collect();
    shown.dedup();
    prop_assert_eq!(shown, visible);
    // A line's rows cover it exactly, and each row's start maps back to it.
    for (i, &r) in all.iter().enumerate() {
        let range = rows.row_range(r);
        let blank = rows.is_filler(r);
        if blank {
            prop_assert!(range.is_empty());
        } else {
            prop_assert_eq!(rows.row_of(range.start), r);
        }
        if r.sub == 0 {
            prop_assert_eq!(range.start, doc.line_start(r.line));
        } else {
            prop_assert_eq!(rows.row_range(all[i - 1]).end, range.start);
        }
        if !blank && rows.is_last_in_line(r) {
            prop_assert_eq!(range.end, doc.line_end(r.line));
        }
        prop_assert!(rows.row_end(r) >= range.start && rows.row_end(r) <= range.end);
    }
    Ok(())
}

/// How a random view lays out its rows: a wrap width, padding (blank rows
/// before lines, as in a compare view), and a filter showing every `step`th
/// line from `offset` (the filter view).
type Layout = (
    Option<usize>,
    Option<(Vec<usize>, usize)>,
    Option<(usize, usize)>,
);

fn layout() -> impl Strategy<Value = Layout> {
    (
        prop::option::of(10usize..16),
        prop::option::weighted(0.3, (prop::collection::vec(0usize..3, 0..8), 0usize..3)),
        prop::option::weighted(0.3, (1usize..4, 0usize..4)),
    )
}

/// A document whose view has the random `layout`.
fn setup_wrapped(initial: &str, (wrap, padding, filter): Layout) -> util::T {
    let mut t = util::setup(initial);
    let b = t.ed.view(t.view).unwrap().buffer();
    t.ed.buffer_mut(b).unwrap().comments = Some(ochrepad_core::CommentSyntax {
        line: Some("//".into()),
        block: Some(("/*".into(), "*/".into())),
    });
    let v = t.ed.view_mut(t.view).unwrap();
    v.wrap_cols = wrap;
    v.padding = padding.map(|(before, after_last)| {
        std::sync::Arc::new(ochrepad_core::Padding { before, after_last })
    });
    if let Some((step, offset)) = filter {
        let lines = t.buffer().doc().line_count();
        let shown: Vec<usize> = (0..lines).filter(|l| (l + offset) % step == 0).collect();
        t.ed.set_filter(t.view, Some(&shown));
    }
    t
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 3000, ..ProptestConfig::default() })]

    /// Selections stay sorted, non-overlapping, on char boundaries and in
    /// range; undoing everything restores the original text, and the
    /// selections from just before the first edit still in the history.
    #[test]
    fn undo_all_restores_original(
        initial in text(),
        actions in prop::collection::vec(action(), 0..40),
        layout in layout(),
    ) {
        let mut t = setup_wrapped(&initial, layout);
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
    fn undo_then_redo_is_identity(
        initial in text(),
        actions in prop::collection::vec(action(), 1..40),
        layout in layout(),
    ) {
        let mut t = setup_wrapped(&initial, layout);
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
    fn line_edits_track_line_count(
        initial in text(),
        actions in prop::collection::vec(action(), 0..40),
        layout in layout(),
    ) {
        let mut t = setup_wrapped(&initial, layout);
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

/// Whether `b` can undo, and its text.
fn undo_state(ed: &ochrepad_core::Editor, b: ochrepad_core::BufferId) -> (bool, bool, String) {
    let buffer = ed.buffer(b).unwrap();
    (
        buffer.history().can_undo(),
        buffer.history().can_redo(),
        buffer.doc().to_string(),
    )
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2000, ..ProptestConfig::default() })]

    /// A history saved as JSON and restored onto the same text in another
    /// editor (a later session) undoes and redoes through the same texts as
    /// the original.
    #[test]
    fn a_saved_history_replays_like_the_original(
        initial in text(),
        actions in prop::collection::vec(action(), 0..30),
    ) {
        let mut ed = ochrepad_core::Editor::new();
        let b = ed.add_buffer(&initial, Default::default());
        let v = ed.add_view(b).unwrap();
        for a in actions {
            let _ = ed.apply(v, a);
        }
        let saved = ed.buffer(b).unwrap().saved_history(usize::MAX);
        let json = serde_json::to_string(&saved).unwrap();
        let mut later = ochrepad_core::Editor::new();
        let lb = later.add_buffer(&ed.buffer(b).unwrap().doc().to_string(), Default::default());
        let lv = later.add_view(lb).unwrap();
        let read: ochrepad_core::SavedHistory = serde_json::from_str(&json).unwrap();
        prop_assert!(later.buffer_mut(lb).unwrap().restore_history(read, true).is_ok());
        for action in [Action::Undo, Action::Redo] {
            loop {
                let (a, b2) = (undo_state(&ed, b), undo_state(&later, lb));
                prop_assert_eq!(&a, &b2);
                let more = if action == Action::Undo { a.0 } else { a.1 };
                if !more {
                    break;
                }
                ed.apply(v, action.clone()).unwrap();
                later.apply(lv, action.clone()).unwrap();
            }
            if action == Action::Undo {
                prop_assert_eq!(undo_state(&later, lb).2, initial.clone());
            }
        }
    }

    /// Formatting and minifying change only the blank space between tokens:
    /// each undoes the other, and the value read back is the same.
    #[test]
    fn json_format_and_minify_keep_the_value(
        json in json_value(),
        width in 1usize..5,
        tabs in any::<bool>(),
    ) {
        let indent = Indent { use_spaces: !tabs, width };
        let pretty = format_json(&json, indent).unwrap();
        let min = minify_json(&json).unwrap();
        prop_assert_eq!(minify_json(&pretty).unwrap(), min.clone());
        prop_assert_eq!(format_json(&min, indent).unwrap(), pretty.clone());
        let value = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap();
        prop_assert_eq!(value(&pretty), value(&json));
    }

    /// Our validator accepts exactly what serde_json accepts.
    #[test]
    fn json_validation_agrees_with_serde_json(s in json_ish()) {
        let ours = validate_json(&s).is_ok();
        let theirs = serde_json::from_str::<serde::de::IgnoredAny>(&s).is_ok();
        prop_assert_eq!(ours, theirs, "{:?}", s);
    }

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

proptest! {
    #![proptest_config(ProptestConfig { cases: 500, ..ProptestConfig::default() })]

    /// With auto-closing on, brackets, quotes and Backspace typed among
    /// random actions keep every invariant, and undoing still gets back to
    /// the start.
    #[test]
    fn auto_closing_keeps_undo_working(
        initial in text(),
        actions in prop::collection::vec(
            prop_oneof![
                3 => prop::sample::select(vec!["(", ")", "[", "]", "{", "}", "\"", "'"])
                    .prop_map(|s| Action::InsertText(s.to_owned())),
                1 => Just(Action::DeleteBackward),
                2 => Just(Action::InsertNewline),
                4 => action(),
            ],
            0..30,
        ),
        layout in layout(),
        smart in any::<bool>(),
    ) {
        let mut t = setup_wrapped(&initial, layout);
        t.ed.settings.auto_close = true;
        // Smart indentation works on the same keys.
        t.ed.settings.smart_indent = smart;
        for a in actions {
            let _ = t.ed.apply(t.view, a);
            check(&t)?;
        }
        while t.buffer().history().can_undo() {
            t.apply(Action::Undo);
            check(&t)?;
        }
        prop_assert_eq!(t.text(), initial);
    }

    /// Text appended outside the history (follow mode) between random
    /// actions keeps every invariant, and undoing everything leaves the
    /// original text with the appended text after it.
    #[test]
    fn appends_between_actions_keep_undo_working(
        initial in text(),
        steps in prop::collection::vec(
            (action(), prop::option::weighted(0.25, short_text())),
            0..30,
        ),
        layout in layout(),
    ) {
        let mut t = setup_wrapped(&initial, layout);
        let b = t.ed.view(t.view).unwrap().buffer();
        let mut appended = String::new();
        for (a, append) in steps {
            let _ = t.ed.apply(t.view, a);
            check(&t)?;
            if let Some(s) = append {
                t.ed.append_unrecorded(b, &s).unwrap();
                appended.push_str(&s);
                check(&t)?;
            }
        }
        while t.buffer().history().can_undo() {
            t.apply(Action::Undo);
            check(&t)?;
        }
        prop_assert_eq!(t.text(), format!("{initial}{appended}"));
    }
}
