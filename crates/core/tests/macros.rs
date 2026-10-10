//! Macro recording and playback.

mod util;

use ochrepad_core::{
    Action, ActionError, ByteOffset, Motion, NamedMacro, Repeat, SearchOptions, SearchQuery,
    SelectionSet,
};
use util::setup;

fn end_semicolon_down() -> Vec<Action> {
    vec![
        Action::MoveCursor {
            motion: Motion::LineEnd,
            extend: false,
        },
        Action::InsertText(";".into()),
        Action::MoveCursor {
            motion: Motion::Down,
            extend: false,
        },
    ]
}

#[test]
fn record_then_play_n_times_as_one_undo_step() {
    let mut t = setup("|a\nb\nc\nd");
    t.ed.start_recording();
    for a in end_semicolon_down() {
        t.apply(a);
    }
    let recorded = t.ed.stop_recording();
    assert_eq!(recorded, end_semicolon_down());
    assert_eq!(t.ed.play_macro(t.view, &recorded, Repeat::Times(2)), Ok(2));
    assert_eq!(t.text(), "a;\nb;\nc;\nd");
    t.apply(Action::Undo);
    assert_eq!(t.text(), "a;\nb\nc\nd", "the playback undoes as one step");
}

#[test]
fn mouse_selections_and_undo_are_not_recorded() {
    let mut t = setup("|abc");
    t.ed.start_recording();
    t.apply(Action::SetSelections(SelectionSet::caret(ByteOffset(1))));
    t.apply(Action::InsertText("x".into()));
    t.apply(Action::Undo);
    t.apply(Action::SelectAll);
    assert_eq!(
        t.ed.stop_recording(),
        vec![Action::InsertText("x".into()), Action::SelectAll]
    );
}

#[test]
fn playback_is_not_recorded_into_an_active_recording() {
    let mut t = setup("|a\nb");
    t.ed.start_recording();
    t.apply(Action::InsertText("1".into()));
    t.ed.play_macro(t.view, &end_semicolon_down(), Repeat::Times(1))
        .unwrap();
    assert_eq!(t.ed.recorded(), Some(&[Action::InsertText("1".into())][..]));
}

#[test]
fn a_failing_step_is_reported_with_its_run_and_step() {
    let mut t = setup("|x TODO");
    let find = Action::Find {
        query: SearchQuery::new("TODO", SearchOptions::default()),
        backward: false,
    };
    let actions = vec![find.clone(), Action::InsertText("DONE".into())];
    let err =
        t.ed.play_macro(t.view, &actions, Repeat::Times(3))
            .unwrap_err();
    assert_eq!((err.run, err.step), (2, 1));
    assert_eq!(err.action, find);
    assert_eq!(err.error, ActionError::NotFound);
    assert_eq!(t.text(), "x DONE", "the completed run stays applied");
    assert!(err.to_string().contains("step 1 of run 2"));
}

#[test]
fn until_end_stops_when_find_fails() {
    let mut t = setup("|a TODO b TODO c TODO d");
    let actions = vec![
        Action::Find {
            query: SearchQuery::new("TODO", SearchOptions::default()),
            backward: false,
        },
        Action::InsertText("DONE".into()),
    ];
    assert_eq!(t.ed.play_macro(t.view, &actions, Repeat::UntilEnd), Ok(3));
    assert_eq!(t.text(), "a DONE b DONE c DONE d");
}

#[test]
fn until_end_stops_at_the_end_of_the_document() {
    let mut t = setup("|a\nb\nc");
    assert_eq!(
        t.ed.play_macro(t.view, &end_semicolon_down(), Repeat::UntilEnd),
        Ok(3)
    );
    assert_eq!(t.text(), "a;\nb;\nc;");
}

#[test]
fn until_end_from_the_end_runs_once() {
    let mut t = setup("a\nb|");
    assert_eq!(
        t.ed.play_macro(t.view, &end_semicolon_down(), Repeat::UntilEnd),
        Ok(1)
    );
    assert_eq!(t.text(), "a\nb;");
}

#[test]
fn until_end_stops_when_a_run_changes_nothing() {
    let mut t = setup("a|bc");
    let up = vec![Action::MoveCursor {
        motion: Motion::Up,
        extend: false,
    }];
    // The first run sets the remembered column; the second changes nothing.
    assert_eq!(t.ed.play_macro(t.view, &up, Repeat::UntilEnd), Ok(2));
}

#[test]
fn named_macros_round_trip_through_json() {
    let m = NamedMacro {
        name: "semicolons".into(),
        shortcut: Some("Ctrl+Shift+1".into()),
        actions: end_semicolon_down(),
    };
    let json = serde_json::to_string(&m).unwrap();
    assert_eq!(serde_json::from_str::<NamedMacro>(&json).unwrap(), m);
    // The shortcut is optional in the file.
    let bare = r#"{"name":"x","actions":[{"InsertText":"y"}]}"#;
    assert_eq!(
        serde_json::from_str::<NamedMacro>(bare).unwrap().shortcut,
        None
    );
}
