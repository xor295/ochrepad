//! Syntax highlighting in the app: detection, the language picker, the size
//! limit, theme following light/dark mode, and re-detection on Save As.

use egui::{Event, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;
use ochrepad::editor_view::LineStyler;

fn harness() -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(900.0, 600.0))
        .with_step_dt(1.0 / 60.0)
        // Background highlighting runs on a wall-clock budget; let `run`
        // wait for it even on a loaded machine.
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.run();
    h
}

fn open(h: &mut Harness<'_, OchrepadApp>, name: &str, text: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::write(&path, text).unwrap();
    h.state_mut().open_path(&path);
    h.run();
    dir
}

/// Distinct colors on line `line`, from the app's own highlighter.
fn colors(h: &mut Harness<'_, OchrepadApp>, line: usize) -> usize {
    let app = h.state_mut();
    let doc = app.buffer().doc().clone();
    let id = app.buffer_id();
    let Some(mut styler) = app.highlighting.styler(id) else {
        return 0;
    };
    let spans = styler.line_spans(&doc, line..line + 1);
    let mut colors: Vec<[u8; 4]> = spans[0]
        .iter()
        .flat_map(|s| s.iter().map(|s| s.color))
        .collect();
    colors.sort();
    colors.dedup();
    colors.len()
}

#[test]
fn opening_a_rust_file_highlights_it() {
    let mut h = harness();
    let _dir = open(
        &mut h,
        "main.rs",
        "fn main() {\n    let s = \"hi\"; // greet\n}\n",
    );
    h.get_by_label("Rust");
    assert!(colors(&mut h, 1) >= 3, "keyword, string and comment colors");
}

#[test]
fn shebang_is_detected_without_an_extension() {
    let mut h = harness();
    let _dir = open(&mut h, "script", "#!/usr/bin/env python3\nprint('x')\n");
    h.get_by_label("Python");
}

#[test]
fn language_picker_changes_the_language() {
    let mut h = harness();
    h.event(Event::Paste("fn main() {}".into()));
    h.run();
    assert_eq!(colors(&mut h, 0), 1, "plain text has one color");
    h.get_by_label("Plain Text").click();
    h.run();
    // The list is sorted; pick an entry near the top so it is on screen.
    let pick = h.state().highlighting.languages()[1].clone();
    h.get_by_label(&pick).click();
    h.run();
    assert_eq!(h.state().highlighting.language(h.state().buffer_id()), pick);
    assert_eq!(h.state().buffer().language.as_deref(), Some(pick.as_str()));
}

#[test]
fn large_files_are_not_highlighted() {
    let mut h = harness();
    h.state_mut().highlighting.size_limit = 16;
    let _dir = open(&mut h, "big.rs", "fn main() { let x = 1; }\n");
    h.get_by_label("Highlighting off: file larger than 16 bytes");
    assert_eq!(colors(&mut h, 0), 0);
}

#[test]
fn syntax_theme_follows_light_and_dark_mode() {
    let mut h = harness();
    let _dir = open(&mut h, "a.rs", "fn x() {}\n");
    h.ctx.set_theme(egui::Theme::Dark);
    h.run();
    assert_eq!(
        h.state().highlighting.active_theme(h.state().buffer_id()),
        Some("OneHalfDark")
    );
    h.ctx.set_theme(egui::Theme::Light);
    h.run();
    assert_eq!(
        h.state().highlighting.active_theme(h.state().buffer_id()),
        Some("OneHalfLight")
    );
    h.state_mut().highlighting.theme_override = Some("Dracula".into());
    h.run();
    assert_eq!(
        h.state().highlighting.active_theme(h.state().buffer_id()),
        Some("Dracula")
    );
}

#[test]
fn save_as_redetects_the_language() {
    let mut h = harness();
    h.event(Event::Paste("print('hi')\n".into()));
    h.run();
    h.get_by_label("Plain Text");
    let dir = tempfile::tempdir().unwrap();
    assert!(h.state_mut().save_to(&dir.path().join("hello.py")));
    h.run();
    h.get_by_label("Python");
}

#[test]
fn typing_keeps_highlighting_in_sync() {
    let mut h = harness();
    let _dir = open(&mut h, "a.rs", "let a = 1;\nlet b = 2;\n");
    let before = colors(&mut h, 1);
    // Opening a block comment on line 1 turns line 2 into a comment.
    h.event(Event::Text("/*".into()));
    h.run();
    assert!(before > 1);
    assert_eq!(colors(&mut h, 1), 1, "line 2 is now all comment");
}
