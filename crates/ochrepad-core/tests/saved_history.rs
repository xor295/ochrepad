//! 0.3.0 P5: undo history kept between sessions.

use ochrepad_core::{Action, Editor, FileMeta, SavedHistory, sha256_hex};

/// An editor with one buffer holding `text`, and its view.
fn editor(text: &str) -> (Editor, ochrepad_core::BufferId, ochrepad_core::ViewId) {
    let mut ed = Editor::new();
    let b = ed.add_buffer(text, FileMeta::default());
    let v = ed.add_view(b).unwrap();
    (ed, b, v)
}

fn text(ed: &Editor, b: ochrepad_core::BufferId) -> String {
    ed.buffer(b).unwrap().doc().to_string()
}

/// Inserts `s` and a line break as one undo step.
fn step(ed: &mut Editor, v: ochrepad_core::ViewId, s: &str) {
    ed.apply(v, Action::InsertText(format!("{s}\n"))).unwrap();
}

#[test]
fn a_history_survives_json_and_undoes_in_a_new_session() {
    let (mut ed, b, v) = editor("start\n");
    ed.apply(
        v,
        Action::MoveCursor {
            motion: ochrepad_core::Motion::DocEnd,
            extend: false,
        },
    )
    .unwrap();
    for s in ["one", "two", "three"] {
        step(&mut ed, v, s);
    }
    // Undo the last one, so there is a redo step too.
    ed.apply(v, Action::Undo).unwrap();
    let ended = text(&ed, b);
    assert_eq!(ended, "start\none\ntwo\n");
    ed.buffer_mut(b).unwrap().mark_saved();
    let saved = ed.buffer(b).unwrap().saved_history(1 << 20);
    let json = serde_json::to_string(&saved).unwrap();

    // A new session: the file is loaded again, with the same text.
    let (mut ed, b, v) = editor(&ended);
    let restored: SavedHistory = serde_json::from_str(&json).unwrap();
    ed.buffer_mut(b)
        .unwrap()
        .restore_history(restored, true)
        .unwrap();
    assert!(!ed.buffer(b).unwrap().is_dirty());
    ed.apply(v, Action::Undo).unwrap();
    assert_eq!(text(&ed, b), "start\none\n");
    assert!(ed.buffer(b).unwrap().is_dirty());
    ed.apply(v, Action::Undo).unwrap();
    ed.apply(v, Action::Undo).unwrap();
    assert_eq!(text(&ed, b), "start\n");
    // Redo all the way, past where the history was saved.
    for _ in 0..3 {
        ed.apply(v, Action::Redo).unwrap();
    }
    assert_eq!(text(&ed, b), "start\none\ntwo\nthree\n");
    ed.apply(v, Action::Undo).unwrap();
    assert!(!ed.buffer(b).unwrap().is_dirty(), "back at the saved text");
    // New edits start a new step and get new revisions.
    ed.apply(v, Action::InsertText("x".into())).unwrap();
    ed.apply(v, Action::Undo).unwrap();
    assert_eq!(text(&ed, b), "start\none\ntwo\n");
    assert!(!ed.buffer(b).unwrap().is_dirty());
}

#[test]
fn a_history_that_does_not_fit_the_text_is_refused() {
    let (mut ed, b, v) = editor("abc");
    ed.apply(v, Action::InsertText("é".into())).unwrap();
    let saved = ed.buffer(b).unwrap().saved_history(1 << 20);
    let current = text(&ed, b);
    assert_eq!(current, "éabc");

    // Another text of the same length: the undo would cut a character.
    let (mut other, ob, _) = editor("a\u{e9}de");
    assert!(
        other
            .buffer_mut(ob)
            .unwrap()
            .restore_history(saved.clone(), true)
            .is_err()
    );
    // Damaged on disk: an edit past the end, or steps that do not connect.
    let mut json: serde_json::Value = serde_json::to_value(&saved).unwrap();
    json["undo"][0]["txns"][0]["inverse"]["edits"][0]["end"] = 99.into();
    let broken: SavedHistory = serde_json::from_value(json).unwrap();
    let (mut ed2, b2, _) = editor(&current);
    assert!(
        ed2.buffer_mut(b2)
            .unwrap()
            .restore_history(broken, true)
            .is_err()
    );
    let mut unlinked = saved.clone();
    unlinked.revision += 1;
    assert!(
        ed2.buffer_mut(b2)
            .unwrap()
            .restore_history(unlinked, true)
            .is_err()
    );
    // A revision at the end of the range would overflow on the next edit.
    let mut overflowing = saved.clone();
    overflowing.next_revision = u64::MAX;
    assert!(
        ed2.buffer_mut(b2)
            .unwrap()
            .restore_history(overflowing, true)
            .is_err()
    );
    // A rectangle on lines the text doesn't have (Delete Line would make a
    // list of them all).
    let mut json: serde_json::Value = serde_json::to_value(&saved).unwrap();
    json["undo"][0]["txns"][0]["before"]["rect"] = serde_json::json!({
        "start_line": 0,
        "end_line": 1_000_000_000u64,
        "start_col": 0,
        "end_col": 1,
    });
    let far: SavedHistory = serde_json::from_value(json).unwrap();
    assert!(
        ed2.buffer_mut(b2)
            .unwrap()
            .restore_history(far, true)
            .is_err()
    );
    // The intact one still restores.
    assert!(
        ed2.buffer_mut(b2)
            .unwrap()
            .restore_history(saved, true)
            .is_ok()
    );
}

#[test]
fn the_size_cap_keeps_the_newest_steps() {
    let (mut ed, b, v) = editor("");
    for i in 0..50 {
        step(&mut ed, v, &format!("line {i} {}", "x".repeat(100)));
    }
    let all = ed.buffer(b).unwrap().saved_history(usize::MAX);
    assert_eq!(all.undo.len(), 50);
    let some = ed.buffer(b).unwrap().saved_history(5_000);
    assert!(!some.undo.is_empty() && some.undo.len() < 50);
    // The kept steps are the newest, ending at the current text.
    assert_eq!(
        some.undo.last().unwrap().rev_after,
        all.undo.last().unwrap().rev_after
    );
    let size: usize = some.undo.iter().map(|s| s.approx_size()).sum();
    assert!(size <= 5_000);
    // Restoring it undoes only as far as it goes.
    let (mut ed2, b2, v2) = editor(&text(&ed, b));
    let kept = some.undo.len();
    ed2.buffer_mut(b2)
        .unwrap()
        .restore_history(some, true)
        .unwrap();
    for _ in 0..kept + 5 {
        ed2.apply(v2, Action::Undo).unwrap();
    }
    let lines = text(&ed2, b2).lines().count();
    assert_eq!(lines, 50 - kept);
}

#[test]
fn the_hash_is_of_the_text() {
    let (ed, b, _) = editor("abc");
    assert_eq!(
        sha256_hex(ed.buffer(b).unwrap().doc()),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
