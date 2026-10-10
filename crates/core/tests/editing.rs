mod util;

use std::time::Duration;

use ochrepad_core::{
    Action, ActionError, ByteOffset, Editor, FileMeta, Motion, Selection, SelectionSet, ViewId,
};
use util::{render, setup};

const FAMILY: &str = "👨‍👩‍👧";

#[test]
fn insert_at_every_caret() {
    let mut t = setup("a|b\nc|d");
    assert_eq!(t.typ("X").show(), "aX|b\ncX|d");
}

#[test]
fn insert_replaces_selection() {
    let mut t = setup("abcd");
    t.set_sels(vec![(3, 1)]);
    assert_eq!(t.apply(Action::InsertText("XY".into())).show(), "aXY|d");
}

#[test]
fn newline_copies_indentation_up_to_the_caret() {
    let mut t = setup("\t  foo|bar");
    assert_eq!(t.typ("\n").show(), "\t  foo\n\t  |bar");
    let mut t = setup("  |  x");
    assert_eq!(t.typ("\n").show(), "  \n  |  x");
}

#[test]
fn delete_by_grapheme() {
    let mut t = setup(&format!("a{FAMILY}|b"));
    assert_eq!(t.apply(Action::DeleteBackward).show(), "a|b");
    let mut t = setup("xe\u{301}|");
    assert_eq!(t.apply(Action::DeleteBackward).show(), "x|");
    let mut t = setup("|漢x");
    assert_eq!(t.apply(Action::DeleteForward).show(), "|x");
}

#[test]
fn delete_with_selection_deletes_the_selection() {
    let mut t = setup("abcdef");
    t.set_sels(vec![(1, 3), (4, 5)]);
    assert_eq!(t.apply(Action::DeleteForward).show(), "a|d|f");
}

#[test]
fn delete_words() {
    let mut t = setup("foo bar|");
    assert_eq!(t.apply(Action::DeleteWordBackward).show(), "foo |");
    let mut t = setup("|foo bar");
    assert_eq!(t.apply(Action::DeleteWordForward).show(), "|bar");
}

#[test]
fn deletes_that_meet_merge_carets() {
    let mut t = setup("ab|c|");
    assert_eq!(t.apply(Action::DeleteBackward).show(), "a|");
    assert_eq!(t.sels().len(), 1);
}

#[test]
fn backspace_at_start_is_a_no_op() {
    let mut t = setup("|abc");
    t.apply(Action::DeleteBackward);
    assert_eq!(t.show(), "|abc");
    assert!(!t.buffer().is_dirty());
    assert!(!t.buffer().history().can_undo());
}

#[test]
fn typing_coalesces_into_one_undo_step() {
    let mut t = setup("|");
    t.typ("abc");
    assert_eq!(t.apply(Action::Undo).show(), "|");
    assert_eq!(t.apply(Action::Redo).show(), "abc|");
}

#[test]
fn a_pause_breaks_coalescing() {
    let mut t = setup("|");
    t.typ("ab");
    t.clock.advance(Duration::from_millis(1500));
    t.typ("cd");
    assert_eq!(t.apply(Action::Undo).show(), "ab|");
    assert_eq!(t.apply(Action::Undo).show(), "|");
}

#[test]
fn cursor_movement_breaks_coalescing() {
    let mut t = setup("|");
    t.typ("ab").mv(Motion::Left).mv(Motion::Right).typ("c");
    assert_eq!(t.apply(Action::Undo).show(), "ab|");
}

#[test]
fn newline_breaks_coalescing() {
    let mut t = setup("|");
    t.typ("ab\ncd");
    assert_eq!(t.apply(Action::Undo).show(), "ab\n|");
    assert_eq!(t.apply(Action::Undo).show(), "ab|");
    assert_eq!(t.apply(Action::Undo).show(), "|");
}

#[test]
fn backspaces_coalesce_but_not_with_typing() {
    let mut t = setup("abcd|");
    for _ in 0..3 {
        t.apply(Action::DeleteBackward);
    }
    assert_eq!(t.apply(Action::Undo).show(), "abcd|");

    let mut t = setup("|");
    t.typ("ab").apply(Action::DeleteBackward);
    assert_eq!(t.apply(Action::Undo).show(), "ab|");
}

#[test]
fn undo_restores_the_selection() {
    let mut t = setup("abcdef");
    t.set_sels(vec![(0, 3)]);
    t.apply(Action::InsertText("X".into()));
    assert_eq!(t.show(), "X|def");
    assert_eq!(t.apply(Action::Undo).show(), "^abc|def");
}

#[test]
fn dirty_tracks_the_saved_revision_through_undo() {
    let mut t = setup("|");
    t.typ("a");
    t.ed.buffer_mut(t.buffer().id()).unwrap().mark_saved();
    assert!(!t.buffer().is_dirty());
    t.typ("b");
    assert!(t.buffer().is_dirty());
    t.apply(Action::Undo);
    assert!(
        !t.buffer().is_dirty(),
        "undo back to the saved state is clean"
    );
    t.apply(Action::Undo);
    assert!(t.buffer().is_dirty());
    t.apply(Action::Redo);
    assert!(!t.buffer().is_dirty());
    t.apply(Action::Redo);
    assert!(t.buffer().is_dirty());
}

#[test]
fn a_new_edit_after_undo_is_a_new_revision() {
    let mut t = setup("|");
    t.typ("a");
    let rev = t.buffer().revision();
    t.ed.buffer_mut(t.buffer().id()).unwrap().mark_saved();
    t.apply(Action::Undo).typ("a");
    assert_eq!(t.text(), "a");
    assert_ne!(t.buffer().revision(), rev);
    assert!(t.buffer().is_dirty());
}

#[test]
fn undo_group_is_one_step() {
    let mut t = setup("x|");
    t.ed.begin_undo_group(t.view).unwrap();
    t.typ("ab\ncd").apply(Action::DeleteBackward);
    t.ed.end_undo_group(t.view).unwrap();
    t.typ("z");
    assert_eq!(t.apply(Action::Undo).show(), "xab\nc|");
    assert_eq!(t.apply(Action::Undo).show(), "x|");
}

#[test]
fn other_views_keep_their_carets_relative_to_the_text() {
    let mut ed = Editor::new();
    let buf = ed.add_buffer("hello world", FileMeta::default());
    let a = ed.add_view(buf).unwrap();
    let b = ed.add_view(buf).unwrap();
    ed.view_mut(b).unwrap().selections =
        SelectionSet::single(Selection::new(ByteOffset(6), ByteOffset(11)));

    let show = |ed: &Editor, v: ViewId| {
        render(
            &ed.buffer(buf).unwrap().doc().to_string(),
            &ed.view(v).unwrap().selections,
        )
    };
    ed.apply(a, Action::InsertText("XX".into())).unwrap();
    assert_eq!(show(&ed, a), "XX|hello world");
    assert_eq!(show(&ed, b), "XXhello ^world|");

    // Deleting text that contains B's selection collapses it.
    ed.apply(
        a,
        Action::MoveCursor {
            motion: Motion::DocEnd,
            extend: true,
        },
    )
    .unwrap();
    ed.apply(a, Action::DeleteBackward).unwrap();
    assert_eq!(show(&ed, b), "XX|");

    // Undo in A maps B's caret through the inverse edits but cannot restore a
    // selection that was deleted; undo only restores the acting view's.
    ed.apply(a, Action::Undo).unwrap();
    assert_eq!(show(&ed, b), "XX|hello world");
    ed.apply(a, Action::Undo).unwrap();
    assert_eq!(show(&ed, b), "|hello world");
}

#[test]
fn recorded_actions_replay_relative_to_the_caret() {
    let mut t = setup("|a\nb\nc");
    t.ed.start_recording();
    t.mv(Motion::LineEnd).typ(";").mv(Motion::Down);
    let actions = t.ed.stop_recording();
    assert_eq!(actions.len(), 3);
    for _ in 0..2 {
        for a in &actions {
            t.apply(a.clone());
        }
    }
    assert_eq!(t.text(), "a;\nb;\nc;");
}

#[test]
fn actions_round_trip_through_serde() {
    let actions = vec![
        Action::InsertText("x\u{301}".into()),
        Action::MoveCursor {
            motion: Motion::PageDown(30),
            extend: true,
        },
        Action::AddCursorBelow,
    ];
    let json = serde_json::to_string(&actions).unwrap();
    let back: Vec<Action> = serde_json::from_str(&json).unwrap();
    assert_eq!(back, actions);
}

#[test]
fn unknown_view_is_an_error() {
    let mut ed = Editor::new();
    assert_eq!(
        ed.apply(ViewId(99), Action::SelectAll),
        Err(ActionError::UnknownView(ViewId(99)))
    );
}

#[test]
fn dirty_lines_are_reported_for_the_highlighter() {
    let mut t = setup("a\nb\nc|");
    t.typ("x");
    let id = t.buffer().id();
    assert_eq!(t.ed.buffer_mut(id).unwrap().take_dirty_from_line(), Some(2));
    assert_eq!(t.ed.buffer_mut(id).unwrap().take_dirty_from_line(), None);
    t.set_sels(vec![(1, 1)]);
    t.typ("y");
    t.apply(Action::Undo);
    assert_eq!(t.ed.buffer_mut(id).unwrap().take_dirty_from_line(), Some(0));
}

#[test]
fn paste_distributes_lines_over_matching_selections() {
    let mut t = setup("a|\nb|\nc|");
    t.apply(Action::Paste("1\n2\n3\n".into()));
    assert_eq!(t.show(), "a1|\nb2|\nc3|");
    // Count mismatch: the whole text goes to every caret.
    let mut t = setup("a|\nb|");
    t.apply(Action::Paste("x\ny\nz".into()));
    assert_eq!(t.show(), "ax\ny\nz|\nbx\ny\nz|");
    // A single caret always gets the whole text.
    let mut t = setup("|");
    assert_eq!(t.apply(Action::Paste("p\nq".into())).show(), "p\nq|");
}

#[test]
fn set_selections_clamps_into_the_document() {
    let mut t = setup("é|x");
    let set = SelectionSet::new(
        vec![
            Selection::new(ByteOffset(1), ByteOffset(99)),
            Selection::caret(ByteOffset(0)),
        ],
        0,
    );
    t.apply(Action::SetSelections(set));
    // 1 is inside 'é' and snaps back to 0; 99 clamps to the end.
    assert_eq!(t.show(), "^éx|");
}
