//! File > Export > HTML…: the document as a web page in the syntax theme's
//! colours, made on a thread and written by the worker.

use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::{Event, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;

fn harness() -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(900.0, 600.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.run();
    h
}

fn open(h: &mut Harness<'_, OchrepadApp>, dir: &Path, name: &str, text: &str) -> PathBuf {
    let file = dir.join(name);
    std::fs::write(&file, text).unwrap();
    h.state_mut().open_path(&file);
    h.run();
    file
}

/// Exports the active document to `page`, as Export > HTML… does after its
/// dialog, and waits in real time for the page and then for the worker.
fn export(h: &mut Harness<'_, OchrepadApp>, page: &Path) {
    let buffer = h.state().buffer_id();
    h.state_mut().export_html_to(buffer, page);
    for _ in 0..1000 {
        h.step();
        if !h.state().exporting() {
            h.state_mut().flush_disk();
            h.run();
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out");
}

/// What the page's `<pre>` holds, without the colours' tags: the text,
/// escaped.
fn page_text(page: &str) -> String {
    let start = page.find("<pre>\n").expect("a <pre>") + "<pre>\n".len();
    let end = page.rfind("</pre>").expect("a </pre>");
    let mut rest = &page[start..end];
    let mut text = String::new();
    while let Some(tag) = rest.find('<') {
        text.push_str(&rest[..tag]);
        let close = rest[tag..].find('>').expect("a whole tag");
        rest = &rest[tag + close + 1..];
    }
    text.push_str(rest);
    text
}

#[test]
fn a_highlighted_document_exports_as_a_web_page() {
    let dir = tempfile::tempdir().unwrap();
    let saved = "fn main() {\n\tlet s = \"<b> & 'x'\";\n}\n";
    let mut h = harness();
    let file = open(&mut h, dir.path(), "main.rs", saved);
    h.get_by_label("Rust");
    // Changes not saved yet are exported too.
    h.event(Event::Paste("// héllo 漢字\n".into()));
    h.run();
    let page = dir.path().join("main.rs.html");
    export(&mut h, &page);

    let html = std::fs::read_to_string(&page).unwrap();
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains("<meta charset=\"utf-8\">"));
    assert!(html.contains("<title>main.rs</title>"));
    let width = h.state().buffer().indent.width;
    assert!(html.contains(&format!("tab-size: {width};")), "{html}");
    assert!(html.contains("<span style=\"color:#"), "{html}");
    assert_eq!(
        page_text(&html),
        "// héllo 漢字\nfn main() {\n\tlet s = &quot;&lt;b&gt; &amp; 'x'&quot;;\n}\n"
    );
    h.get_by_label_contains("Exported to main.rs.html");
    // The document stays as it was: on its own file, unsaved.
    assert!(h.state().buffer().is_dirty());
    assert_eq!(h.state().file_name(), "main.rs");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), saved);
}

#[test]
fn a_document_over_the_highlighting_limit_exports_without_colours() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness();
    h.state_mut().settings.highlight_limit_mb = 1;
    let big = "fn main() { let x = 1 < 2; }\n".repeat(40_000); // just over 1 MB
    open(&mut h, dir.path(), "big.rs", &big);
    let page = dir.path().join("big.rs.html");
    export(&mut h, &page);
    let html = std::fs::read_to_string(&page).unwrap();
    assert!(!html.contains("<span"));
    assert_eq!(page_text(&html), big.replace('<', "&lt;"));
    h.get_by_label_contains("Exported to big.rs.html without colours (over the 1 MB");
}

#[test]
fn the_document_s_own_file_is_never_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness();
    let file = open(&mut h, dir.path(), "page.html", "<p>hi</p>\n");
    h.event(Event::Paste("<h1>changed</h1>\n".into()));
    h.run();
    let buffer = h.state().buffer_id();
    h.state_mut().export_html_to(buffer, &file);
    h.run();
    assert!(!h.state().exporting());
    assert!(
        h.state()
            .message()
            .is_some_and(|m| m.contains("the document's own file")),
        "{:?}",
        h.state().message()
    );
    h.state_mut().flush_disk();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "<p>hi</p>\n");
}

#[test]
fn the_file_menu_has_export_html() {
    let mut h = harness();
    h.get_by_label("File").click();
    h.run();
    h.get_by_label_contains("Export ⏵").hover();
    h.run();
    h.get_by_label("HTML…");
}
