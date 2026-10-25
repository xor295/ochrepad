//! The real rust-analyzer on a small project: what the scripted server in
//! `lsp_ui.rs` cannot show (how a real server spells URIs, when it sends
//! what). Ignored, since CI has no rust-analyzer; run it after changing the
//! client:
//!
//! ```sh
//! cargo test -p ochrepad --test real_servers -- --ignored --nocapture
//! ```
//!
//! It passes without doing anything when rust-analyzer is not on `PATH`.

use std::time::{Duration, Instant};

use egui::{Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;
use ochrepad_lsp::uri::plain_path;

/// Steps in real time until `done`, for at most `secs` seconds.
fn wait(
    h: &mut Harness<'_, OchrepadApp>,
    what: &str,
    secs: u64,
    done: impl Fn(&Harness<'_, OchrepadApp>) -> bool,
) {
    let start = Instant::now();
    while !done(h) {
        if start.elapsed() > Duration::from_secs(secs) {
            let log = h
                .state()
                .language_servers
                .sessions
                .first()
                .map(|s| {
                    s.log
                        .lock()
                        .map(|l| l.lines().collect::<Vec<_>>().join("\n"))
                })
                .and_then(Result::ok)
                .unwrap_or_default();
            panic!("timed out waiting for {what}; the server's log:\n{log}");
        }
        std::thread::sleep(Duration::from_millis(10));
        h.step();
    }
    eprintln!("{what}: {:.1} s", start.elapsed().as_secs_f32());
}

fn labels(h: &Harness<'_, OchrepadApp>) -> Vec<String> {
    h.state()
        .view_state()
        .map(|s| s.completion.labels())
        .unwrap_or_default()
}

/// Screen position of (line, column) in the editor labelled `label`.
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
#[ignore = "needs rust-analyzer"]
fn rust_analyzer_diagnoses_completes_and_explains() {
    if ochrepad_lsp::config::resolve_program("rust-analyzer", &[]).is_none() {
        eprintln!("rust-analyzer is not on PATH; nothing to test");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // The spelling the trust question uses (canonicalize gives Windows'
    // verbatim one).
    let root = plain_path(&dir.path().canonicalize().unwrap());
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::create_dir(root.join("src")).unwrap();
    let file = root.join("src").join("main.rs");
    std::fs::write(
        &file,
        "fn main() {\n    let items: Vec<i32>   =   Vec::new();\n    use nothing_here::Thing;\n}\n",
    )
    .unwrap();
    let mut app = OchrepadApp::new();
    // Servers are looked for once the configuration is known.
    app.language_servers.load_config(root.join("lsp.json"));
    app.settings.lsp_trusted.push(root.clone());
    let mut h = Harness::builder()
        .with_size(Vec2::new(1000.0, 560.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), app);
    h.run();
    h.state_mut().open_path(&file);
    h.run();

    // Its diagnostics (sent for its own spelling of the file's URI).
    wait(&mut h, "the unresolved import", 120, |h| {
        h.state()
            .language_servers
            .diagnostics_of(h.state().buffer_id())
            .iter()
            .any(|d| d.message.contains("unresolved import"))
    });

    // Format Document (rustfmt, through the server).
    h.key_press_modifiers(Modifiers::ALT | Modifiers::SHIFT, Key::F);
    wait(&mut h, "the formatting", 30, |h| {
        h.state()
            .buffer()
            .doc()
            .to_string()
            .contains("let items: Vec<i32> = Vec::new();")
    });

    // The Function List from the server's symbols.
    h.state_mut().settings.show_function_list = true;
    wait(&mut h, "the function list", 30, |h| {
        h.state()
            .function_list
            .entries(h.state().buffer_id())
            .is_some_and(|e| e.iter().any(|e| e.symbol.name == "main"))
    });
    h.state_mut().settings.show_function_list = false;

    // Go to Symbol in Folder.
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::ALT, Key::T);
    h.run();
    h.event(Event::Text("main".into()));
    wait(&mut h, "main in the folder", 30, |h| {
        h.query_by_label_contains("ƒ  main").is_some()
    });
    h.key_press(Key::Escape);
    h.run();

    // Members after `items.`, with documentation.
    h.key_press_modifiers(Modifiers::COMMAND, Key::G);
    h.run();
    h.event(Event::Text("2:1".into()));
    h.key_press(Key::Enter);
    h.run();
    h.key_press(Key::End);
    h.key_press(Key::Enter);
    h.run();
    for c in "items.pu".chars() {
        h.event(Event::Text(c.to_string()));
        h.run();
    }
    wait(&mut h, "push", 60, |h| {
        labels(h).first().map(String::as_str) == Some("push")
    });
    wait(&mut h, "push's documentation", 30, |h| {
        h.query_by_label_contains("Appends an element").is_some()
    });
    h.key_press(Key::Tab);
    h.run();
    assert!(
        h.state()
            .buffer()
            .doc()
            .to_string()
            .contains("    items.push\n"),
        "{}",
        h.state().buffer().doc()
    );

    // Hover over `Vec` on line 2.
    h.event(Event::PointerMoved(cell_pos(
        &h,
        "Editor: main.rs",
        1,
        16.0,
    )));
    wait(&mut h, "Vec's hover", 30, |h| {
        h.query_by_label_contains("contiguous growable array")
            .is_some()
    });
    h.event(Event::PointerGone);

    // The signature after `push(`.
    h.event(Event::Text("(".into()));
    h.run();
    wait(&mut h, "push's signature", 30, |h| {
        h.query_by_label_contains("fn push(").is_some()
    });
    if let Ok(shot) = std::env::var("SHOT") {
        h.render().unwrap().save(shot).unwrap();
    }
    h.key_press(Key::Escape);
    h.run();

    // Shift+F12 on `items`: where it is declared and used.
    h.key_press_modifiers(Modifiers::COMMAND, Key::G);
    h.run();
    h.event(Event::Text("2:9".into()));
    h.key_press(Key::Enter);
    h.run();
    h.key_press_modifiers(Modifiers::SHIFT, Key::F12);
    wait(&mut h, "items' references", 30, |h| {
        h.state()
            .results
            .searches
            .front()
            .is_some_and(|s| s.done && s.matches == 2)
    });

    // Code actions on `items` (which ones depends on the server's version).
    h.key_press_modifiers(Modifiers::COMMAND, Key::Period);
    wait(&mut h, "code actions", 30, |h| {
        !h.state().code_action_titles().is_empty()
    });
    eprintln!("code actions: {:?}", h.state().code_action_titles());
    h.key_press(Key::Escape);
    h.run();
    assert!(h.state().code_action_titles().is_empty());

    // Rename `items` to `values`.
    h.key_press_modifiers(Modifiers::SHIFT, Key::F6);
    wait(&mut h, "the rename dialog", 30, |h| {
        h.query_by_label("Rename items to:").is_some()
    });
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Text("values".into()));
    h.key_press(Key::Enter);
    wait(&mut h, "the rename", 30, |h| {
        let text = h.state().buffer().doc().to_string();
        text.contains("let values: Vec") && text.contains("values.push(")
    });

    // F12 on `Vec`: the standard library's source.
    h.key_press_modifiers(Modifiers::COMMAND, Key::G);
    h.run();
    h.event(Event::Text("2:17".into()));
    h.key_press(Key::Enter);
    h.run();
    h.key_press(Key::F12);
    wait(&mut h, "Vec's definition", 30, |h| {
        h.state()
            .buffer()
            .meta
            .path
            .as_deref()
            .is_some_and(|p| p.ends_with("mod.rs"))
    });
    eprintln!("opened {:?}", h.state().buffer().meta.path);
    h.state_mut().language_servers.stop_all();
}
