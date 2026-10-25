//! The client against the scripted server (and, for broken output and a
//! missing program, real processes).

use std::sync::Arc;
use std::time::{Duration, Instant};

use ochrepad_core::position::ColumnUnits;
use ochrepad_core::{ByteOffset, Document};
use serde_json::{Value, json};

use crate::client::{Client, Event, State};
use crate::config::ServerConfig;
use crate::message::codes;
use crate::testing::FakeServer;
use crate::transport::{Launch, SharedLog};

fn config() -> ServerConfig {
    ServerConfig {
        name: "fake".into(),
        command: vec!["fake".into()],
        languages: vec!["Rust".into()],
        settings: json!({"fake": {"answer": 42}}),
        initialization_options: Value::Null,
    }
}

fn root() -> std::path::PathBuf {
    std::env::temp_dir()
}

fn start(launch: &Launch) -> Client {
    Client::start(
        &config(),
        launch,
        &root(),
        SharedLog::default(),
        Arc::new(|| {}),
    )
}

/// Polls until `done` holds for an event, collecting them all.
fn until(c: &mut Client, done: impl Fn(&Event) -> bool) -> Vec<Event> {
    let start = Instant::now();
    let mut all = Vec::new();
    loop {
        let events = c.poll();
        let hit = events.iter().any(&done);
        all.extend(events);
        if hit {
            return all;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out: {all:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Waits until the server has been sent `n` messages for `method`.
fn sent(server: &FakeServer, method: &str, n: usize) -> Vec<Value> {
    let start = Instant::now();
    loop {
        let p = server.params(method);
        if p.len() >= n {
            return p;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{method} not sent: {:?}",
            server.methods()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

const URI: &str = "file:///tmp/a.rs";

#[test]
fn documents_are_opened_changed_saved_and_closed() {
    let server = FakeServer::new();
    let mut c = start(&server.launch());
    // Opened before the server is ready: sent once it is.
    let mut doc = Document::new("fn main() {}\n");
    c.open(URI, "rust", &doc);
    until(&mut c, |e| matches!(e, Event::Started));
    assert_eq!(c.units, ColumnUnits::Utf16);
    let open = &sent(&server, "textDocument/didOpen", 1)[0];
    assert_eq!(open["textDocument"]["text"], "fn main() {}\n");
    assert_eq!(open["textDocument"]["version"], 1);
    // An edit: only what changed, in the old text's positions.
    doc.replace(ByteOffset(11)..ByteOffset(11), "😀 x");
    c.change(URI, &doc);
    let change = &sent(&server, "textDocument/didChange", 1)[0];
    assert_eq!(change["textDocument"]["version"], 2);
    assert_eq!(
        change["contentChanges"][0],
        json!({"range": {"start": {"line": 0, "character": 11}, "end": {"line": 0, "character": 11}}, "text": "😀 x"})
    );
    // UTF-16: the emoji is two units.
    assert_eq!(
        c.position(&doc, ByteOffset(15)),
        json!({"line": 0, "character": 13})
    );
    c.change(URI, &doc);
    c.save(URI);
    c.close(URI);
    sent(&server, "textDocument/didClose", 1);
    assert_eq!(
        server.params("textDocument/didChange").len(),
        1,
        "nothing changed"
    );
    assert_eq!(server.params("textDocument/didSave").len(), 1);
    c.shutdown();
    let events = until(&mut c, |e| matches!(e, Event::Stopped { .. }));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Stopped { asked: true, .. }))
    );
    let methods = server.methods();
    assert_eq!(&methods[methods.len() - 2..], ["shutdown", "exit"]);
}

#[test]
fn utf8_positions_and_full_sync_when_the_server_wants_them() {
    let server = FakeServer::new().capabilities(json!({
        "positionEncoding": "utf-8",
        "textDocumentSync": 1,
    }));
    let mut c = start(&server.launch());
    until(&mut c, |e| matches!(e, Event::Started));
    assert_eq!(c.units, ColumnUnits::Utf8);
    let mut doc = Document::new("é\n");
    c.open(URI, "rust", &doc);
    doc.replace(ByteOffset(2)..ByteOffset(2), "ü");
    c.change(URI, &doc);
    let change = &sent(&server, "textDocument/didChange", 1)[0];
    assert_eq!(change["contentChanges"][0], json!({"text": "éü\n"}));
    assert_eq!(
        c.position(&doc, ByteOffset(2)),
        json!({"line": 0, "character": 2})
    );
}

#[test]
fn requests_are_answered_refused_or_given_up_on() {
    let server = FakeServer::new()
        .answer(
            "textDocument/hover",
            |p| json!({"contents": p["position"]["line"]}),
        )
        .answer("slow/thing", |_| {
            std::thread::sleep(Duration::from_millis(300));
            Value::Null
        });
    let mut c = start(&server.launch());
    assert!(
        c.request("textDocument/hover", json!({}), 1, Duration::from_secs(5))
            .is_none(),
        "not before it is running"
    );
    until(&mut c, |e| matches!(e, Event::Started));
    c.request(
        "textDocument/hover",
        json!({"position": {"line": 7}}),
        1,
        Duration::from_secs(5),
    );
    c.request("unknown/method", json!({}), 2, Duration::from_secs(5));
    c.request("slow/thing", json!({}), 3, Duration::from_millis(50));
    let mut answers = Vec::new();
    while answers.len() < 3 {
        for e in until(&mut c, |e| matches!(e, Event::Answer { .. })) {
            if let Event::Answer { tag, result } = e {
                answers.push((tag, result));
            }
        }
    }
    answers.sort_by_key(|(t, _)| *t);
    assert_eq!(answers[0].1.as_ref().unwrap()["contents"], 7);
    assert_eq!(
        answers[1].1.as_ref().unwrap_err().code,
        codes::METHOD_NOT_FOUND
    );
    assert_eq!(answers[2].1.as_ref().unwrap_err().code, codes::TIMED_OUT);
    // A cancelled request's answer is not passed on.
    let id = c
        .request("textDocument/hover", json!({}), 4, Duration::from_secs(5))
        .unwrap();
    c.cancel(id);
    sent(&server, "$/cancelRequest", 2);
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        !c.poll()
            .iter()
            .any(|e| matches!(e, Event::Answer { tag: 4, .. }))
    );
}

#[test]
fn what_the_server_asks_and_tells() {
    let server = FakeServer::new()
        .request_after_start(
            "workspace/configuration",
            json!({"items": [{"section": "fake.answer"}, {"section": "other"}]}),
        )
        .request_after_start("workspace/applyEdit", json!({"edit": {"changes": {}}}))
        .request_after_start("window/showDocument", json!({"uri": "file://evil/share/x"}))
        .request_after_start("made/up", json!({}))
        .react("textDocument/didOpen", |p| {
            vec![(
                "textDocument/publishDiagnostics".into(),
                json!({"uri": p["textDocument"]["uri"], "version": 1,
                       "diagnostics": [{"range": {"start": {"line": 0, "character": 0},
                                                  "end": {"line": 0, "character": 2}},
                                        "message": "bad"}]}),
            )]
        });
    let mut c = start(&server.launch());
    let events = until(&mut c, |e| matches!(e, Event::ApplyEdit { .. }));
    let Some(Event::ApplyEdit { id, edit }) = events
        .into_iter()
        .find(|e| matches!(e, Event::ApplyEdit { .. }))
    else {
        unreachable!()
    };
    assert_eq!(edit, json!({"changes": {}}));
    c.answer_apply_edit(id, false, Some("not now"));
    c.open(URI, "rust", &Document::new("x\n"));
    let events = until(&mut c, |e| matches!(e, Event::Diagnostics { .. }));
    let diags = events.iter().find_map(|e| match e {
        Event::Diagnostics {
            uri, diagnostics, ..
        } => Some((uri.clone(), diagnostics.len())),
        _ => None,
    });
    assert_eq!(diags, Some((URI.to_owned(), 1)));
    // Its requests were answered: the settings by section, the edit
    // refused, a network document not shown, an unknown method an error.
    let start = Instant::now();
    while server.responses().len() < 4 {
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(5));
    }
    let r = server.responses();
    assert_eq!(r[0].as_ref().unwrap(), &json!([42, null]));
    assert!(
        r.iter()
            .any(|a| a.as_ref().is_ok_and(|v| v == &json!({"success": false})))
    );
    assert!(
        r.iter()
            .any(|a| a.as_ref().is_ok_and(|v| v["applied"] == false))
    );
    assert!(
        r.iter()
            .any(|a| a.as_ref().is_err_and(|e| e.code == codes::METHOD_NOT_FOUND))
    );
}

#[test]
fn a_server_that_quits_or_never_starts() {
    let quits = Launch::InProcess(Arc::new(|_, _| {}));
    let mut c = start(&quits);
    let events = until(&mut c, |e| matches!(e, Event::Stopped { .. }));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Stopped { asked: false, .. }))
    );
    assert!(matches!(c.state, State::Stopped { asked: false, .. }));

    let missing = Launch::Program {
        program: root().join("no-such-server-here.exe"),
        args: Vec::new(),
        cwd: root(),
    };
    let c = start(&missing);
    assert!(matches!(c.state, State::Stopped { .. }));
}

#[test]
fn a_real_process_with_broken_output_ends_its_session() {
    let (shell, args) = if cfg!(windows) {
        ("cmd", vec!["/c".to_owned(), "echo not a header".to_owned()])
    } else {
        ("sh", vec!["-c".to_owned(), "echo not a header".to_owned()])
    };
    let program = crate::config::resolve_program(shell, &[]).expect("a shell");
    let mut c = start(&Launch::Program {
        program,
        args,
        cwd: root(),
    });
    let events = until(&mut c, |e| matches!(e, Event::Stopped { .. }));
    let reason = events.iter().find_map(|e| match e {
        Event::Stopped { reason, .. } => Some(reason.clone()),
        _ => None,
    });
    assert!(reason.unwrap().contains("broke off"));
}
