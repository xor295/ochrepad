//! Language servers in the app (PLAN_completion L1), with a scripted server
//! on a thread: the question before one runs in a folder, documents given
//! to it and kept in sync, diagnostics, and a server that keeps crashing.

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Event, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;
use ochrepad_lsp::config::ServerConfig;
use ochrepad_lsp::testing::FakeServer;
use ochrepad_lsp::transport::Launch;
use serde_json::{Value, json};

fn harness(app: OchrepadApp) -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(900.0, 600.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), app);
    h.run();
    h
}

fn config() -> ServerConfig {
    ServerConfig {
        name: "fake-rust".into(),
        command: vec!["fake".into()],
        languages: vec!["Rust".into()],
        settings: Value::Null,
        initialization_options: Value::Null,
    }
}

/// Steps in real time until `done`, for at most 10 seconds.
fn wait(
    h: &mut Harness<'_, OchrepadApp>,
    what: &str,
    done: impl Fn(&Harness<'_, OchrepadApp>) -> bool,
) {
    let start = Instant::now();
    while !done(h) {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(5));
        h.step();
    }
}

fn diagnostics_on_open() -> FakeServer {
    FakeServer::new().react("textDocument/didOpen", |p| {
        vec![(
            "textDocument/publishDiagnostics".into(),
            json!({
                "uri": p["textDocument"]["uri"],
                "diagnostics": [{
                    "range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 7}},
                    "severity": 1,
                    "message": "this is wrong",
                }],
            }),
        )]
    })
}

fn open_rust_file(server: &FakeServer) -> (tempfile::TempDir, Harness<'static, OchrepadApp>) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();
    let mut app = OchrepadApp::new();
    app.language_servers
        .set_test_server("Rust", config(), server.launch());
    let mut h = harness(app);
    h.state_mut().open_path(&file);
    h.run();
    (dir, h)
}

#[test]
fn a_server_runs_only_after_the_folder_is_trusted_and_follows_the_document() {
    let server = diagnostics_on_open();
    let (dir, mut h) = open_rust_file(&server);
    wait(&mut h, "the question", |h| {
        h.state().language_servers.question.is_some()
    });
    assert!(
        server.methods().is_empty(),
        "nothing runs before the answer"
    );
    h.get_by_label("Start in this folder").click();
    h.run();
    assert_eq!(h.state().settings.lsp_trusted, [dir.path().to_path_buf()]);
    wait(&mut h, "didOpen", |_| {
        !server.params("textDocument/didOpen").is_empty()
    });
    let open = &server.params("textDocument/didOpen")[0];
    assert_eq!(open["textDocument"]["text"], "fn main() {}\n");
    assert_eq!(open["textDocument"]["languageId"], "rust");
    // Its diagnostics, in the document.
    wait(&mut h, "diagnostics", |h| {
        !h.state()
            .language_servers
            .diagnostics_of(h.state().buffer_id())
            .is_empty()
    });
    let b = h.state().buffer_id();
    let d = &h.state().language_servers.diagnostics_of(b)[0];
    assert_eq!(d.message, "this is wrong");
    assert_eq!(
        d.range,
        Some(ochrepad_core::ByteOffset(3)..ochrepad_core::ByteOffset(7))
    );
    h.get_by_label_contains("fake-rust · 1 error, 0 warnings");
    // Typing is sent as a change, and the diagnostic moves with the text.
    h.event(Event::Text("// ".into()));
    h.run();
    wait(&mut h, "didChange", |_| {
        !server.params("textDocument/didChange").is_empty()
    });
    let change = &server.params("textDocument/didChange")[0];
    assert_eq!(change["contentChanges"][0]["text"], "// ");
    let d = &h.state().language_servers.diagnostics_of(b)[0];
    assert_eq!(
        d.range,
        Some(ochrepad_core::ByteOffset(6)..ochrepad_core::ByteOffset(10))
    );
    // Saving, then closing the document.
    assert!(h.state_mut().save());
    wait(&mut h, "didSave", |_| {
        !server.params("textDocument/didSave").is_empty()
    });
    let view = h.state().active_view();
    h.state_mut().close_view(view);
    h.run();
    wait(&mut h, "didClose", |_| {
        !server.params("textDocument/didClose").is_empty()
    });
}

#[test]
fn a_folder_can_be_declined_and_the_server_never_runs_there() {
    let server = FakeServer::new();
    let (dir, mut h) = open_rust_file(&server);
    wait(&mut h, "the question", |h| {
        h.state().language_servers.question.is_some()
    });
    h.get_by_label("Not in this folder").click();
    h.run();
    assert_eq!(h.state().settings.lsp_declined, [dir.path().to_path_buf()]);
    for _ in 0..30 {
        h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        h.state().language_servers.question.is_none(),
        "not asked again"
    );
    assert!(h.state().language_servers.sessions.is_empty());
    assert!(server.methods().is_empty());
}

#[test]
fn a_server_that_keeps_crashing_is_given_up_on() {
    let starts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = starts.clone();
    let crashes = Launch::InProcess(Arc::new(move |_, _| {
        counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }));
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();
    let mut app = OchrepadApp::new();
    app.settings.lsp_trusted.push(dir.path().to_path_buf());
    app.language_servers
        .set_test_server("Rust", config(), crashes);
    let mut h = harness(app);
    h.state_mut().open_path(&file);
    h.run();
    // Started, then restarted three times (a second or more apart).
    wait(&mut h, "four starts", |_| {
        starts.load(std::sync::atomic::Ordering::SeqCst) >= 4
    });
    wait(&mut h, "giving up", |h| {
        h.state()
            .language_servers
            .sessions
            .first()
            .is_some_and(|s| s.status().contains("stopped"))
    });
    for _ in 0..600 {
        h.step();
    }
    assert_eq!(starts.load(std::sync::atomic::Ordering::SeqCst), 4);
}

/// Screen position of (line, column) in the editor labelled `label`, at the
/// cell's center.
fn cell_pos(h: &Harness<'_, OchrepadApp>, label: &str, line: usize, col: f32) -> egui::Pos2 {
    let rect = h.get_by_label(label).rect();
    let font = egui::FontId::monospace(14.0);
    let (cell_w, line_h) = h
        .ctx
        .fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
    let x = rect.min.x + 5.0 * cell_w + 8.0 + (col + 0.5) * cell_w;
    egui::Pos2::new(x, rect.min.y + (line as f32 + 0.5) * line_h)
}

#[test]
fn resting_on_a_name_shows_its_problems_and_the_servers_hover() {
    let server = diagnostics_on_open().answer("textDocument/hover", |p| {
        assert_eq!(p["position"], json!({"line": 0, "character": 4}));
        json!({
            "contents": {"kind": "markdown", "value": "The **entry point**."},
            "range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 7}},
        })
    });
    let (dir, mut h) = open_rust_file(&server);
    h.state_mut()
        .settings
        .lsp_trusted
        .push(dir.path().to_path_buf());
    wait(&mut h, "diagnostics", |h| {
        !h.state()
            .language_servers
            .diagnostics_of(h.state().buffer_id())
            .is_empty()
    });
    let p = cell_pos(&h, "Editor: main.rs", 0, 4.0);
    h.event(Event::PointerMoved(p));
    wait(&mut h, "the hover", |h| {
        h.query_by_label_contains("entry point").is_some()
    });
    h.get_by_label("this is wrong");
    // Moving off the text closes it.
    h.event(Event::PointerMoved(cell_pos(&h, "Editor: main.rs", 5, 4.0)));
    h.run();
    assert!(h.query_by_label_contains("entry point").is_none());
}
