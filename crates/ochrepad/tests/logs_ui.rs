//! 0.3.0 P6: log tools in the editor: the filter bar, highlight rules and
//! follow mode.

mod util;

use std::time::Duration;

use egui::{Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;

const LOG: &str = "2026-10-13 INFO  starting\n2026-10-13 ERROR disk full\n2026-10-13 DEBUG retry 1\n2026-10-13 WARN  slow write\n2026-10-13 ERROR gave up\n2026-10-13 INFO  done\n";

fn harness(size: Vec2) -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(size)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .wgpu()
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.run();
    h
}

fn key(h: &mut Harness<'_, OchrepadApp>, m: Modifiers, k: Key) {
    h.key_press_modifiers(m, k);
    h.run();
}

/// Steps in real time until `done`, for work on other threads.
fn wait_for(h: &mut Harness<'_, OchrepadApp>, done: impl Fn(&OchrepadApp) -> bool) {
    for _ in 0..500 {
        h.step();
        if done(h.state()) {
            h.run();
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out");
}

fn hidden(h: &Harness<'_, OchrepadApp>) -> usize {
    let app = h.state();
    app.editor
        .view(app.active_view())
        .unwrap()
        .folds()
        .filter_hidden_count()
}

#[test]
fn the_filter_bar_shows_only_matching_lines() {
    let mut h = harness(Vec2::new(900.0, 400.0));
    h.event(Event::Paste(LOG.into()));
    h.run();
    key(&mut h, Modifiers::ALT | Modifiers::SHIFT, Key::L);
    h.event(Event::Text("ERROR".into()));
    wait_for(&mut h, |a| {
        a.editor
            .view(a.active_view())
            .is_some_and(|v| v.folds().filter_hidden_count() > 0)
            && !a.filter.is_busy(a.active_view())
    });
    // Two error lines; the empty last line is hidden too.
    assert_eq!(hidden(&h), 5);
    h.get_by_label("2 of 7 lines");
    // Hide matching: the other lines.
    h.get_by_label("Hide matching").click();
    wait_for(&mut h, |a| {
        a.editor
            .view(a.active_view())
            .is_some_and(|v| v.folds().filter_hidden_count() == 2)
            && !a.filter.is_busy(a.active_view())
    });
    h.get_by_label("5 of 7 lines");
    // Escape in the field removes the filter.
    h.get_by_label("×").click();
    h.run();
    assert_eq!(hidden(&h), 0);
    assert!(!h.state().filter.is_open(h.state().active_view()));
}

#[test]
fn edits_filter_again_after_a_pause() {
    let mut h = harness(Vec2::new(900.0, 400.0));
    h.event(Event::Paste(LOG.into()));
    h.run();
    key(&mut h, Modifiers::ALT | Modifiers::SHIFT, Key::L);
    h.event(Event::Text("ERROR".into()));
    wait_for(&mut h, |a| {
        a.editor
            .view(a.active_view())
            .is_some_and(|v| v.folds().filter_hidden_count() == 5)
    });
    // Type a new error line at the start of the document.
    h.get_by_label("Editor: new 1").click();
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::Home);
    h.event(Event::Paste("ERROR new\n".into()));
    h.run();
    // The new error line shows; so does the line the caret is on now (the
    // INFO line the paste pushed down), which the filter leaves alone.
    wait_for(&mut h, |a| {
        !a.filter.is_busy(a.active_view())
            && a.editor
                .view(a.active_view())
                .is_some_and(|v| v.folds().filter_hidden_count() == 4)
    });
    h.get_by_label("4 of 8 lines");
}

#[test]
fn the_line_being_written_stays_when_the_filter_is_applied_again() {
    let mut h = harness(Vec2::new(900.0, 400.0));
    h.event(Event::Paste(LOG.into()));
    h.run();
    key(&mut h, Modifiers::ALT | Modifiers::SHIFT, Key::L);
    h.event(Event::Text("ERROR".into()));
    wait_for(&mut h, |a| {
        !a.filter.is_busy(a.active_view())
            && a.editor
                .view(a.active_view())
                .is_some_and(|v| v.folds().filter_hidden_count() == 5)
    });
    // A note under the first error: End, Enter, and some typing.
    h.get_by_label("Editor: new 1").click();
    h.run();
    let view = h.state().active_view();
    h.state_mut()
        .editor
        .apply(view, ochrepad_core::Action::GoToLine { line: 2, column: 1 })
        .unwrap();
    key(&mut h, Modifiers::NONE, Key::End);
    key(&mut h, Modifiers::NONE, Key::Enter);
    h.event(Event::Text("note: ret".into()));
    h.run();
    // The filter runs again after the pause; the note doesn't match, but
    // the caret stays on it and the typing goes on there.
    let revision = h.state().buffer().revision();
    wait_for(&mut h, |a| {
        !a.filter.is_busy(a.active_view()) && a.filter_applied_at(a.active_view()) == Some(revision)
    });
    h.event(Event::Text("ried later".into()));
    h.run();
    assert_eq!(
        text(&h).lines().nth(2),
        Some("note: retried later"),
        "{}",
        text(&h)
    );
}

fn append(path: &std::path::Path, bytes: &[u8]) {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(bytes).unwrap();
}

fn text(h: &Harness<'_, OchrepadApp>) -> String {
    h.state().buffer().doc().to_string()
}

#[test]
fn follow_mode_appends_what_the_file_gains() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("app.log");
    std::fs::write(&file, "line 1\n").unwrap();
    let mut h = harness(Vec2::new(800.0, 300.0));
    h.state_mut().open_path(&file);
    h.run();
    let b = h.state().buffer_id();
    h.state_mut().set_following(b, true);
    h.run();
    h.get_by_label("Following");

    append(&file, b"line 2\n");
    wait_for(&mut h, |a| {
        a.buffer().doc().to_string() == "line 1\nline 2\n"
    });
    assert!(!h.state().buffer().is_dirty());
    assert!(!h.state().buffer().history().can_undo(), "not an undo step");
    // The caret was at the end, so it follows.
    let end = h.state().buffer().doc().len();
    let head = h
        .state()
        .editor
        .view(h.state().active_view())
        .unwrap()
        .selections
        .primary()
        .head;
    assert_eq!(head, end);

    // A line written in pieces, ending in CR LF: the CR waits for its LF.
    append(&file, b"line 3 part");
    wait_for(&mut h, |a| a.buffer().doc().to_string().ends_with("part"));
    append(&file, b" end\r");
    append(&file, b"\n");
    wait_for(&mut h, |a| {
        a.buffer().doc().to_string().ends_with("part end\n")
    });
    assert_eq!(text(&h), "line 1\nline 2\nline 3 part end\n");

    // Rotated (shorter): reloaded, without asking, and not as an undo step
    // (a log rewritten all the time would pile them up).
    std::fs::write(&file, "fresh\n").unwrap();
    wait_for(&mut h, |a| a.buffer().doc().to_string() == "fresh\n");
    assert!(h.state().pending_disk_changes().is_empty());
    assert!(!h.state().buffer().is_dirty());
    assert!(!h.state().buffer().history().can_undo());
}

#[test]
fn follow_reads_what_a_utf16_file_gains_without_reloading_it() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("ps.log");
    let utf16 = |s: &str| -> Vec<u8> { s.encode_utf16().flat_map(u16::to_le_bytes).collect() };
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(utf16("top\r\n  a\r\n  b\r\n"));
    std::fs::write(&file, &bytes).unwrap();
    let mut h = harness(Vec2::new(800.0, 300.0));
    h.state_mut().open_path(&file);
    h.run();
    assert_eq!(h.state().buffer().meta.encoding, "UTF-16LE");
    let b = h.state().buffer_id();
    // A fold over the top: an append leaves it, a reload would drop it.
    let view = h.state().active_view();
    assert!(h.state_mut().editor.fold_at(view, 0));
    h.state_mut().set_following(b, true);
    h.run();

    append(&file, &utf16("next \u{263A}\r"));
    append(&file, &utf16("\n"));
    wait_for(&mut h, |a| {
        a.buffer().doc().to_string().ends_with("next \u{263A}\n")
    });
    assert_eq!(text(&h), "top\n  a\n  b\nnext \u{263A}\n");
    let folded = h.state().editor.view(view).unwrap().folds().is_folded(0);
    assert!(folded, "appended, not reloaded");
    assert!(!h.state().buffer().is_dirty());
}

#[test]
fn a_followed_log_is_filtered_as_it_grows() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("app.log");
    std::fs::write(&file, LOG).unwrap();
    let mut h = harness(Vec2::new(900.0, 400.0));
    h.state_mut().open_path(&file);
    h.run();
    let b = h.state().buffer_id();
    h.state_mut().set_following(b, true);
    h.run();
    key(&mut h, Modifiers::ALT | Modifiers::SHIFT, Key::L);
    h.event(Event::Text("ERROR".into()));
    // The two error lines, and the end the caret follows (empty line 6).
    wait_for(&mut h, |a| {
        !a.filter.is_busy(a.active_view())
            && a.editor
                .view(a.active_view())
                .is_some_and(|v| v.folds().filter_hidden_count() == 4)
    });
    // Lines arrive faster than the filter's pause, so it never pauses;
    // what they bring is filtered as it comes.
    for i in 0..12 {
        let level = if i % 4 == 3 { "ERROR" } else { "INFO " };
        append(&file, format!("2026-10-14 {level} event {i}\n").as_bytes());
        for _ in 0..6 {
            h.step();
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    wait_for(&mut h, |a| a.buffer().doc().line_count() == 19);
    let view = h.state().active_view();
    let v = h.state().editor.view(view).unwrap();
    let shown: Vec<usize> = (0..19).filter(|&l| !v.folds().filter_hides(l)).collect();
    // The old errors, the three new ones, and the end.
    assert_eq!(shown, vec![1, 4, 9, 13, 17, 18]);
    let end = h.state().buffer().doc().len();
    assert_eq!(v.selections.primary().head, end, "still following the end");
}

#[test]
fn a_log_looks_right() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("server.log");
    std::fs::write(&file, LOG).unwrap();
    let mut h = harness(Vec2::new(640.0, 260.0));
    h.state_mut().open_path(&file);
    h.run();
    // Errors and warnings tinted, debug output dimmed; the filter bar hides
    // the INFO lines.
    key(&mut h, Modifiers::ALT | Modifiers::SHIFT, Key::L);
    h.event(Event::Text("INFO".into()));
    h.run();
    h.get_by_label("Hide matching").click();
    wait_for(&mut h, |a| {
        !a.filter.is_busy(a.active_view())
            && a.editor
                .view(a.active_view())
                .is_some_and(|v| v.folds().filter_hidden_count() == 2)
    });
    util::snapshot(&mut h, "log_view");
}
