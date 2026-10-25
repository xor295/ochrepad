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
    // The status bar is drawn before the frame takes in diagnostics.
    h.run();
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

fn typ(h: &mut Harness<'_, OchrepadApp>, s: &str) {
    for c in s.chars() {
        h.event(Event::Text(c.to_string()));
        h.run();
    }
}

fn labels(h: &Harness<'_, OchrepadApp>) -> Vec<String> {
    h.state()
        .view_state()
        .map(|s| s.completion.labels())
        .unwrap_or_default()
}

fn text(h: &Harness<'_, OchrepadApp>) -> String {
    h.state().buffer().doc().to_string()
}

#[test]
fn the_servers_completions_come_first_with_their_documentation_and_imports() {
    let server = FakeServer::new()
        .capabilities(json!({
            "textDocumentSync": { "openClose": true, "change": 2 },
            "completionProvider": { "triggerCharacters": ["."], "resolveProvider": true },
        }))
        .answer("textDocument/completion", |p| {
            let pos = p["position"].clone();
            let line = pos["line"].clone();
            json!({
                "isIncomplete": false,
                "items": [
                    {"label": "push_item(…)", "filterText": "push_item", "insertText": "push_item",
                     "kind": 3, "sortText": "1", "labelDetails": {"description": "fn(&mut self)"}},
                    {"label": "HashMap", "kind": 22, "sortText": "2",
                     "textEdit": {"range": {"start": {"line": line, "character": 0}, "end": pos},
                                  "newText": "HashMap"},
                     "additionalTextEdits": [{
                         "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
                         "newText": "use std::collections::HashMap;\n"}]},
                ],
            })
        })
        .answer("completionItem/resolve", |p| {
            let mut item = p.clone();
            item["documentation"] = json!({"kind": "markdown", "value": "Pushes **one** item."});
            item
        });
    let (dir, mut h) = open_rust_file(&server);
    h.state_mut()
        .settings
        .lsp_trusted
        .push(dir.path().to_path_buf());
    wait(&mut h, "didOpen", |_| {
        !server.params("textDocument/didOpen").is_empty()
    });
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::End);
    h.run();
    typ(&mut h, "pu");
    wait(&mut h, "the server's items", |h| {
        labels(h).first().map(String::as_str) == Some("push_item(…)")
    });
    // The selected item's documentation, asked for when it is selected.
    wait(&mut h, "the documentation", |h| {
        h.query_by_label_contains("Pushes").is_some()
    });
    h.key_press(egui::Key::Tab);
    h.run();
    assert_eq!(text(&h), "fn main() {}\npush_item");

    // An item with an import: one undo step for both.
    h.key_press(egui::Key::Enter);
    typ(&mut h, "Ha");
    wait(&mut h, "HashMap", |h| {
        labels(h).contains(&"HashMap".to_owned())
    });
    let i = labels(&h).iter().position(|l| l == "HashMap").unwrap();
    for _ in 0..i {
        h.key_press(egui::Key::ArrowDown);
        h.run();
    }
    h.key_press(egui::Key::Enter);
    h.run();
    assert_eq!(
        text(&h),
        "use std::collections::HashMap;\nfn main() {}\npush_item\nHashMap"
    );
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run();
    assert_eq!(text(&h), "fn main() {}\npush_item\nHa");
}

#[test]
fn f12_opens_the_definition_the_server_names() {
    let server = FakeServer::new()
        .capabilities(json!({
            "textDocumentSync": { "openClose": true, "change": 2 },
            "definitionProvider": true,
        }))
        .answer("textDocument/definition", |p| {
            let uri = p["textDocument"]["uri"]
                .as_str()
                .unwrap()
                .replace("main.rs", "other.rs");
            json!([{
                "uri": uri,
                "range": {"start": {"line": 1, "character": 3}, "end": {"line": 1, "character": 8}},
            }])
        });
    let (dir, mut h) = open_rust_file(&server);
    std::fs::write(dir.path().join("other.rs"), "// other\nfn thing() {}\n").unwrap();
    h.state_mut()
        .settings
        .lsp_trusted
        .push(dir.path().to_path_buf());
    wait(&mut h, "didOpen", |_| {
        !server.params("textDocument/didOpen").is_empty()
    });
    h.key_press(egui::Key::F12);
    wait(&mut h, "the other file", |h| {
        h.state()
            .buffer()
            .meta
            .path
            .as_deref()
            .is_some_and(|p| p.ends_with("other.rs"))
    });
    h.run();
    h.get_by_label("Ln 2, Col 4");
}

#[test]
fn typing_an_opening_bracket_shows_the_signature_until_escape() {
    let server = FakeServer::new()
        .capabilities(json!({
            "textDocumentSync": { "openClose": true, "change": 2 },
            "signatureHelpProvider": { "triggerCharacters": ["(", ","] },
        }))
        .answer("textDocument/signatureHelp", |p| {
            let active = if p["context"]["triggerCharacter"] == "," {
                1
            } else {
                0
            };
            json!({
                "signatures": [{
                    "label": "fn add(left: i32, right: i32) -> i32",
                    "parameters": [{"label": [7, 16]}, {"label": "right: i32"}],
                    "documentation": "Adds them.",
                }],
                "activeParameter": active,
            })
        });
    let (dir, mut h) = open_rust_file(&server);
    h.state_mut()
        .settings
        .lsp_trusted
        .push(dir.path().to_path_buf());
    wait(&mut h, "didOpen", |_| {
        !server.params("textDocument/didOpen").is_empty()
    });
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::End);
    h.run();
    typ(&mut h, "add(");
    wait(&mut h, "the signature", |h| {
        h.query_by_label_contains("fn add(left").is_some()
    });
    let asked = server.params("textDocument/signatureHelp");
    assert_eq!(asked[0]["context"]["triggerCharacter"], "(");
    h.get_by_label_contains("Adds them.");
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(h.query_by_label_contains("fn add(left").is_none());
}

#[test]
fn shift_f12_lists_the_references_in_search_results() {
    let server = FakeServer::new()
        .capabilities(json!({
            "textDocumentSync": { "openClose": true, "change": 2 },
            "referencesProvider": true,
            "definitionProvider": true,
        }))
        .answer("textDocument/references", |p| {
            assert_eq!(p["context"]["includeDeclaration"], true);
            let uri = p["textDocument"]["uri"].as_str().unwrap().to_owned();
            let other = uri.replace("main.rs", "other.rs");
            json!([
                {"uri": uri, "range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 7}}},
                {"uri": other, "range": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 8}}},
            ])
        });
    let (dir, mut h) = open_rust_file(&server);
    std::fs::write(dir.path().join("other.rs"), "// other\nfn x(main: u8) {}\n").unwrap();
    h.state_mut()
        .settings
        .lsp_trusted
        .push(dir.path().to_path_buf());
    wait(&mut h, "didOpen", |_| {
        !server.params("textDocument/didOpen").is_empty()
    });
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::G);
    h.run();
    h.event(Event::Text("1:4".into()));
    h.key_press(egui::Key::Enter);
    h.run();
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::F12);
    wait(&mut h, "the references", |h| {
        h.state()
            .results
            .searches
            .front()
            .is_some_and(|s| s.done && s.files.len() == 2)
    });
    let s = h.state().results.searches.front().unwrap();
    assert_eq!(s.query.pattern, "main");
    assert_eq!(s.matches, 2);
    assert_eq!(s.files[1].hits[0].text, "fn x(main: u8) {}");
    // Shift+F12 is not also F12.
    assert!(server.params("textDocument/definition").is_empty());
    h.run();
    h.get_by_label_contains("references (fake-rust)");
}

/// The open document saved as `name`, and its text.
fn doc_named(h: &Harness<'_, OchrepadApp>, name: &str) -> Option<(String, bool)> {
    h.state()
        .editor
        .buffers()
        .find(|b| b.meta.path.as_deref().is_some_and(|p| p.ends_with(name)))
        .map(|b| (b.doc().to_string(), b.is_dirty()))
}

#[test]
fn rename_changes_every_file_the_server_names_and_opens_the_others_unsaved() {
    let server = FakeServer::new()
        .capabilities(json!({
            "textDocumentSync": { "openClose": true, "change": 2 },
            "renameProvider": { "prepareProvider": true },
        }))
        .answer("textDocument/prepareRename", |_| {
            json!({
                "range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 7}},
                "placeholder": "main",
            })
        })
        .answer("textDocument/rename", |p| {
            let uri = p["textDocument"]["uri"].as_str().unwrap().to_owned();
            let other = uri.replace("main.rs", "other.rs");
            let name = p["newName"].clone();
            json!({"documentChanges": [
                {"textDocument": {"uri": uri, "version": null}, "edits": [
                    {"range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 7}}, "newText": name},
                ]},
                {"textDocument": {"uri": other, "version": null}, "edits": [
                    {"range": {"start": {"line": 1, "character": 0}, "end": {"line": 1, "character": 4}}, "newText": name},
                ]},
            ]})
        });
    let (dir, mut h) = open_rust_file(&server);
    std::fs::write(dir.path().join("other.rs"), "// other\nmain();\n").unwrap();
    h.state_mut()
        .settings
        .lsp_trusted
        .push(dir.path().to_path_buf());
    wait(&mut h, "didOpen", |_| {
        !server.params("textDocument/didOpen").is_empty()
    });
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::G);
    h.run();
    h.event(Event::Text("1:4".into()));
    h.key_press(egui::Key::Enter);
    h.run();
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::F6);
    wait(&mut h, "the rename dialog", |h| {
        h.query_by_label("Rename main to:").is_some()
    });
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(Event::Text("start".into()));
    h.key_press(egui::Key::Enter);
    wait(&mut h, "the renamed files", |h| {
        doc_named(h, "other.rs").is_some()
    });
    h.run();
    assert_eq!(
        doc_named(&h, "main.rs"),
        Some(("fn start() {}\n".to_owned(), true))
    );
    assert_eq!(
        doc_named(&h, "other.rs"),
        Some(("// other\nstart();\n".to_owned(), true)),
        "opened, edited, not saved"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("other.rs")).unwrap(),
        "// other\nmain();\n"
    );
    // The tab that was active stays so.
    assert!(
        h.state()
            .buffer()
            .meta
            .path
            .as_deref()
            .is_some_and(|p| p.ends_with("main.rs"))
    );
}

#[test]
fn format_document_and_format_on_save() {
    let server = FakeServer::new()
        .capabilities(json!({
            "textDocumentSync": { "openClose": true, "change": 2 },
            "documentFormattingProvider": true,
        }))
        .answer("textDocument/formatting", |p| {
            assert!(p["options"]["tabSize"].is_u64());
            json!([{"range": {"start": {"line": 0, "character": 11}, "end": {"line": 0, "character": 11}}, "newText": "\n"}])
        });
    let (dir, mut h) = open_rust_file(&server);
    h.state_mut()
        .settings
        .lsp_trusted
        .push(dir.path().to_path_buf());
    wait(&mut h, "didOpen", |_| {
        !server.params("textDocument/didOpen").is_empty()
    });
    h.key_press_modifiers(egui::Modifiers::ALT | egui::Modifiers::SHIFT, egui::Key::F);
    wait(&mut h, "the formatting", |h| text(h) == "fn main() {\n}\n");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run();
    assert_eq!(text(&h), "fn main() {}\n", "one undo step");

    // Format on save: formatted, then saved.
    h.state_mut().settings.format_on_save = true;
    h.event(Event::Text("x".into()));
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
    let file = dir.path().join("main.rs");
    wait(&mut h, "the formatted file on disk", |_| {
        std::fs::read_to_string(&file).is_ok_and(|t| t == "xfn main() \n{}\n")
    });
}

#[test]
fn code_actions_apply_edits_and_let_a_command_edit_only_while_it_runs() {
    let edit_to = |uri: &Value, text: &str| {
        json!({"changes": {uri.as_str().unwrap(): [
            {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "newText": text},
        ]}})
    };
    let server = diagnostics_on_open()
        .capabilities(json!({
            "textDocumentSync": { "openClose": true, "change": 2 },
            "codeActionProvider": { "resolveProvider": true },
            "executeCommandProvider": { "commands": ["fake.edit"] },
        }))
        // Refused: no code action is running.
        .request_after_start("workspace/applyEdit", json!({"edit": {"changes": {}}}))
        .answer("textDocument/codeAction", move |p| {
            let uri = &p["textDocument"]["uri"];
            json!([
                {"title": "Not here", "disabled": {"reason": "Select a block"}},
                {"title": "Add a comment", "kind": "quickfix", "isPreferred": true,
                 "edit": edit_to(uri, "// a\n")},
                {"title": "Resolve me", "data": {"uri": uri}},
                {"title": "Run a command", "command": "fake.edit", "arguments": [uri]},
            ])
        })
        .answer("codeAction/resolve", move |p| {
            let mut a = p.clone();
            a["edit"] = edit_to(&p["data"]["uri"], "// b\n");
            a
        })
        .request_on("workspace/executeCommand", move |p| {
            vec![(
                "workspace/applyEdit".into(),
                json!({"edit": edit_to(&p["arguments"][0], "// c\n")}),
            )]
        })
        .answer("workspace/executeCommand", |_| Value::Null);
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
    wait(&mut h, "the refused edit", |_| {
        !server.responses().is_empty()
    });
    assert_eq!(server.responses()[0].as_ref().unwrap()["applied"], false);

    // On the problem.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::G);
    h.run();
    h.event(Event::Text("1:5".into()));
    h.key_press(egui::Key::Enter);
    h.run();
    let actions = |h: &mut Harness<'_, OchrepadApp>| {
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Period);
        wait(h, "the menu", |h| {
            h.query_by_label("Add a comment").is_some()
        });
    };
    // The preferred one is selected.
    actions(&mut h);
    assert_eq!(
        server.params("textDocument/codeAction")[0]["context"]["diagnostics"][0]["message"],
        "this is wrong",
        "the problem there, as the server sent it"
    );
    h.key_press(egui::Key::Enter);
    h.run();
    assert_eq!(text(&h), "// a\nfn main() {}\n");
    // One that needs resolving.
    actions(&mut h);
    h.get_by_label("Resolve me").click();
    wait(&mut h, "the resolved edit", |h| {
        text(h).starts_with("// b\n")
    });
    // A disabled one says why.
    actions(&mut h);
    h.get_by_label("Not here").click();
    h.run();
    assert!(text(&h).starts_with("// b\n// a\n"), "{}", text(&h));
    // A command, whose edit the server asks for while it runs.
    actions(&mut h);
    h.get_by_label("Run a command").click();
    wait(&mut h, "the command's edit", |h| {
        text(h).starts_with("// c\n")
    });
    let applied = server.responses();
    assert_eq!(applied.last().unwrap().as_ref().unwrap()["applied"], true);
}

#[test]
fn the_function_list_uses_the_servers_symbols() {
    let server = FakeServer::new()
        .capabilities(json!({
            "textDocumentSync": { "openClose": true, "change": 2 },
            "documentSymbolProvider": true,
        }))
        .answer("textDocument/documentSymbol", |_| {
            let at =
                json!({"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 7}});
            json!([{"name": "from_server", "kind": 12, "range": at, "selectionRange": at}])
        });
    let (dir, mut h) = open_rust_file(&server);
    h.state_mut()
        .settings
        .lsp_trusted
        .push(dir.path().to_path_buf());
    wait(&mut h, "didOpen", |_| {
        !server.params("textDocument/didOpen").is_empty()
    });
    // An edit, so the list is read again now that the server runs.
    h.event(Event::Text(" ".into()));
    h.state_mut().settings.show_function_list = true;
    wait(&mut h, "the server's symbols", |h| {
        h.state()
            .function_list
            .entries(h.state().buffer_id())
            .is_some_and(|e| e.iter().any(|e| e.symbol.name == "from_server"))
    });
}

#[test]
fn go_to_symbol_searches_the_server_and_opens_the_one_chosen() {
    let server = FakeServer::new().capabilities(json!({
        "textDocumentSync": { "openClose": true, "change": 2 },
        "workspaceSymbolProvider": true,
    }));
    // The answer names other.rs beside the file the server was given.
    let received = server.received.clone();
    let server = server.answer("workspace/symbol", move |p| {
        assert_eq!(p["query"], "Thi");
        let opened = received
            .lock()
            .unwrap()
            .iter()
            .find_map(|m| match m {
                ochrepad_lsp::message::Message::Notification { method, params }
                    if method == "textDocument/didOpen" =>
                {
                    params["textDocument"]["uri"].as_str().map(str::to_owned)
                }
                _ => None,
            })
            .unwrap();
        json!([{"name": "Thing", "kind": 23, "containerName": "other", "location": {
            "uri": opened.replace("main.rs", "other.rs"),
            "range": {"start": {"line": 1, "character": 3}, "end": {"line": 1, "character": 8}},
        }}])
    });
    let (dir, mut h) = open_rust_file(&server);
    std::fs::write(dir.path().join("other.rs"), "// other\nfn Thing() {}\n").unwrap();
    h.state_mut()
        .settings
        .lsp_trusted
        .push(dir.path().to_path_buf());
    wait(&mut h, "didOpen", |_| {
        !server.params("textDocument/didOpen").is_empty()
    });
    h.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::ALT,
        egui::Key::T,
    );
    h.run();
    h.event(Event::Text("Thi".into()));
    wait(&mut h, "the symbol", |h| {
        h.query_by_label_contains("Thing  (other)").is_some()
    });
    h.key_press(egui::Key::Enter);
    wait(&mut h, "other.rs", |h| {
        h.state()
            .buffer()
            .meta
            .path
            .as_deref()
            .is_some_and(|p| p.ends_with("other.rs"))
    });
    h.run();
    h.get_by_label("Ln 2, Col 4");
}
