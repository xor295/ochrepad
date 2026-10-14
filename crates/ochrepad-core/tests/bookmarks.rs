//! Bookmarks (Search > Bookmark): marked lines that follow edits, and the
//! commands on them.

mod util;

use ochrepad_core::{Action, ActionError, Motion, SearchOptions, SearchQuery};
use util::setup;

fn marks(t: &util::T) -> Vec<usize> {
    t.buffer().bookmarks().lines().to_vec()
}

fn mark(t: &mut util::T, lines: &[usize]) {
    let b = t.ed.view(t.view).unwrap().buffer();
    t.ed.buffer_mut(b)
        .unwrap()
        .set_bookmarks(lines.iter().copied());
}

#[test]
fn toggle_marks_each_carets_line_or_unmarks_them_all() {
    let mut t = setup("a|\nb\nc|\nd");
    t.apply(Action::ToggleBookmark);
    assert_eq!(marks(&t), vec![0, 2]);
    t.apply(Action::ToggleBookmark);
    assert!(marks(&t).is_empty());
    // One of them marked already: both get marked.
    mark(&mut t, &[2]);
    t.apply(Action::ToggleBookmark);
    assert_eq!(marks(&t), vec![0, 2]);
    // Not an undo step: undo has nothing to do.
    assert!(!t.buffer().history().can_undo());
}

#[test]
fn next_and_previous_go_round_the_marks() {
    let mut t = setup("|zero\none\ntwo\nthree\nfour");
    assert_eq!(
        t.ed.apply(t.view, Action::NextBookmark),
        Err(ActionError::NoBookmarks)
    );
    mark(&mut t, &[1, 3]);
    t.apply(Action::NextBookmark);
    assert_eq!(t.show(), "zero\n|one\ntwo\nthree\nfour");
    t.apply(Action::NextBookmark);
    assert_eq!(t.show(), "zero\none\ntwo\n|three\nfour");
    t.apply(Action::NextBookmark);
    assert_eq!(t.show(), "zero\n|one\ntwo\nthree\nfour", "wraps around");
    t.apply(Action::PrevBookmark);
    assert_eq!(t.show(), "zero\none\ntwo\n|three\nfour", "wraps back");
    t.apply(Action::PrevBookmark);
    assert_eq!(t.show(), "zero\n|one\ntwo\nthree\nfour");
}

#[test]
fn marks_follow_the_text_they_mark() {
    let mut t = setup("a\nb\n|c\nd");
    mark(&mut t, &[2, 3]);
    // Lines added above push the marks down.
    t.mv(Motion::DocStart);
    t.apply(Action::InsertText("new\n".into()));
    assert_eq!(marks(&t), vec![3, 4]);
    // Typing inside a marked line keeps its mark.
    t.apply(Action::GoToLine { line: 4, column: 2 });
    t.apply(Action::InsertText("xyz".into()));
    assert_eq!(marks(&t), vec![3, 4]);
    // A line break typed at the start of a marked line pushes the mark down
    // with the line's text (Scintilla's markers do this).
    t.apply(Action::GoToLine { line: 4, column: 1 });
    t.apply(Action::InsertNewline);
    assert_eq!(marks(&t), vec![4, 5]);
    assert_eq!(t.text(), "new\na\nb\n\ncxyz\nd");
    // A line break typed at its end leaves the mark where it is.
    t.apply(Action::GoToLine { line: 5, column: 5 });
    t.apply(Action::InsertNewline);
    assert_eq!(marks(&t), vec![4, 6]);
    // Joining a marked line to the one before moves the mark up.
    t.apply(Action::GoToLine { line: 7, column: 1 });
    t.apply(Action::DeleteBackward);
    assert_eq!(t.text(), "new\na\nb\n\ncxyz\nd");
    assert_eq!(marks(&t), vec![4, 5]);
}

#[test]
fn sorting_lines_keeps_marks_on_the_same_lines() {
    let mut t = setup("pear\napple\nfig\nkiwi");
    mark(&mut t, &[1, 3]);
    t.apply(Action::SortLines(Default::default()));
    assert_eq!(t.text(), "apple\nfig\nkiwi\npear");
    assert_eq!(marks(&t), vec![1, 3]);
}

#[test]
fn deleted_lines_take_marks_to_the_line_after_and_undo_does_not_bring_them_back() {
    let mut t = setup("a\n|b\nc\nd");
    mark(&mut t, &[1, 2]);
    t.apply(Action::DeleteLine);
    assert_eq!(t.text(), "a\nc\nd");
    assert_eq!(marks(&t), vec![1], "the two marks merge on the line after");
    t.apply(Action::Undo);
    assert_eq!(t.text(), "a\nb\nc\nd");
    assert_eq!(marks(&t), vec![2], "the mark stays with c");
}

#[test]
fn removing_bookmarked_and_unbookmarked_lines() {
    let mut t = setup("a\nb\nc\nd\ne");
    mark(&mut t, &[1, 3]);
    t.apply(Action::RemoveUnbookmarkedLines);
    assert_eq!(t.text(), "b\nd");
    assert_eq!(marks(&t), vec![0, 1]);
    t.apply(Action::Undo);
    assert_eq!(t.text(), "a\nb\nc\nd\ne");
    t.ed.buffer_mut(t.ed.view(t.view).unwrap().buffer())
        .unwrap()
        .set_bookmarks([1, 4]);
    t.apply(Action::RemoveBookmarkedLines);
    assert_eq!(t.text(), "a\nc\nd");
    assert!(marks(&t).is_empty(), "their lines are gone");
    // One undo step each.
    t.apply(Action::Undo);
    assert_eq!(t.text(), "a\nb\nc\nd\ne");
    assert_eq!(
        t.ed.apply(t.view, Action::RemoveBookmarkedLines),
        Err(ActionError::NoBookmarks)
    );
}

#[test]
fn inverse_and_clear() {
    let mut t = setup("a\nb\nc");
    mark(&mut t, &[1]);
    t.apply(Action::InverseBookmarks);
    assert_eq!(marks(&t), vec![0, 2]);
    t.apply(Action::ClearBookmarks);
    assert!(marks(&t).is_empty());
}

#[test]
fn bookmark_matches_marks_the_lines_with_one() {
    let mut t = setup("INFO a\nERROR b\nINFO c\nERROR d ERROR");
    mark(&mut t, &[0]);
    let query = SearchQuery::new("ERROR", SearchOptions::default());
    t.apply(Action::BookmarkMatches(query));
    assert_eq!(marks(&t), vec![0, 1, 3]);
    let none = SearchQuery::new("FATAL", SearchOptions::default());
    assert_eq!(
        t.ed.apply(t.view, Action::BookmarkMatches(none)),
        Err(ActionError::NotFound)
    );
}

#[test]
fn paste_replaces_the_text_of_each_marked_line() {
    let mut t = setup("a\nb\nc\nd");
    mark(&mut t, &[1, 3]);
    t.apply(Action::ReplaceBookmarkedLines("x\ny".into()));
    assert_eq!(t.text(), "a\nx\ny\nc\nx\ny");
    assert_eq!(marks(&t), vec![1, 4], "the first line of each");
}

#[test]
fn copied_text_is_the_marked_lines() {
    let mut t = setup("a\nb\nc");
    mark(&mut t, &[0, 2]);
    let b = t.buffer();
    assert_eq!(
        ochrepad_core::bookmarks::marked_text(b.doc(), b.bookmarks()),
        "a\nc"
    );
}

#[test]
fn a_reload_keeps_marks_on_their_line_numbers() {
    let mut t = setup("a\nb\nc\nd");
    mark(&mut t, &[1, 3]);
    let b = t.ed.view(t.view).unwrap().buffer();
    t.ed.reload_buffer(b, "A\nB\nC", Default::default())
        .unwrap();
    assert_eq!(
        marks(&t),
        vec![1, 2],
        "line 3 is gone: its mark is on the last"
    );
}

#[test]
fn macros_record_and_replay_bookmark_commands() {
    let mut t = setup("|INFO a\nERROR b\nINFO c\nERROR d");
    let query = SearchQuery::new("ERROR", SearchOptions::default());
    t.ed.start_recording();
    t.apply(Action::Find {
        query: query.clone(),
        backward: false,
    });
    t.apply(Action::ToggleBookmark);
    let actions = t.ed.stop_recording();
    assert_eq!(actions.len(), 2);
    t.ed.play_macro(t.view, &actions, ochrepad_core::Repeat::Times(1))
        .unwrap();
    assert_eq!(marks(&t), vec![1, 3]);
}

#[test]
fn a_find_and_bookmark_macro_runs_to_the_end_and_stops() {
    // The find panel wraps round by default; running to the end must not.
    let mut t = setup("|INFO a\nERROR b\nINFO c\nERROR d\nINFO e");
    let query = SearchQuery::new(
        "ERROR",
        SearchOptions {
            wrap: true,
            ..SearchOptions::default()
        },
    );
    let actions = vec![
        Action::Find {
            query,
            backward: false,
        },
        Action::ToggleBookmark,
    ];
    let runs =
        t.ed.play_macro(t.view, &actions, ochrepad_core::Repeat::UntilEnd)
            .unwrap();
    assert_eq!(runs, 2);
    assert_eq!(marks(&t), vec![1, 3]);
}

#[test]
fn removing_unmarked_lines_keeps_the_final_line_break() {
    let mut t = setup("a\nb\nc\n");
    mark(&mut t, &[1]);
    t.apply(Action::RemoveUnbookmarkedLines);
    assert_eq!(t.text(), "b\n");
    let mut t = setup("a\nb\nc\n");
    mark(&mut t, &[0, 1, 2]);
    t.apply(Action::RemoveUnbookmarkedLines);
    assert_eq!(t.text(), "a\nb\nc\n");
}

#[test]
fn a_read_only_document_still_reloads_from_disk() {
    let mut t = setup("old\ntext");
    let b = t.ed.view(t.view).unwrap().buffer();
    t.ed.buffer_mut(b).unwrap().read_only = true;
    t.ed.reload_buffer(b, "new\ntext", Default::default())
        .unwrap();
    assert_eq!(t.text(), "new\ntext");
    assert!(t.buffer().read_only, "it stays read-only");
    assert_eq!(
        t.ed.apply(t.view, Action::InsertText("x".into())),
        Err(ActionError::ReadOnly)
    );
}

#[test]
fn a_bookmark_macro_run_to_the_end_stops_at_the_last_bookmark() {
    let mut t = setup("|a\nb\nc\nd\n");
    mark(&mut t, &[1, 3]);
    let actions = vec![Action::NextBookmark, Action::InsertText("// ".into())];
    let runs =
        t.ed.play_macro(t.view, &actions, ochrepad_core::Repeat::UntilEnd)
            .unwrap();
    assert_eq!(runs, 2);
    assert_eq!(t.text(), "a\n// b\nc\n// d\n");
}
