//! 0.3.0 P4: Edit > Text Tools (JSON, encodings, hashes).

use egui::{Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;

fn with_text(text: &str) -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(900.0, 600.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.run();
    h.event(Event::Paste(text.to_owned()));
    h.run();
    h
}

fn text(h: &Harness<'_, OchrepadApp>) -> String {
    h.state().buffer().doc().to_string()
}

fn key(h: &mut Harness<'_, OchrepadApp>, m: Modifiers, k: Key) {
    h.key_press_modifiers(m, k);
    h.run();
}

/// Edit > Text Tools > `item`.
fn tool(h: &mut Harness<'_, OchrepadApp>, item: &str) {
    h.get_by_label("Edit").click();
    h.run();
    h.get_by_label_contains("Text Tools").hover();
    h.run();
    h.get_by_label(item).click();
    h.run();
}

fn copied(h: &Harness<'_, OchrepadApp>) -> Vec<String> {
    h.output()
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn json_is_formatted_with_the_documents_indentation() {
    let mut h = with_text(r#"{"b":[1,{"c":null}],"a":"x"}"#);
    tool(&mut h, "Format JSON");
    assert_eq!(
        text(&h),
        "{\n\t\"b\": [\n\t\t1,\n\t\t{\n\t\t\t\"c\": null\n\t\t}\n\t],\n\t\"a\": \"x\"\n}",
        "a new document indents with tabs; key order is kept"
    );
    tool(&mut h, "Minify JSON");
    assert_eq!(text(&h), r#"{"b":[1,{"c":null}],"a":"x"}"#);

    // Two spaces for this document, from the status bar.
    h.get_by_label("Tabs: 4").click();
    h.run();
    h.get_by_label("Indent Using Spaces").click();
    h.run();
    h.get_by_label("Spaces: 4").click();
    h.run();
    h.get_by_label("Width 2").click();
    h.run();
    tool(&mut h, "Format JSON");
    assert_eq!(
        text(&h),
        "{\n  \"b\": [\n    1,\n    {\n      \"c\": null\n    }\n  ],\n  \"a\": \"x\"\n}"
    );
    // One undo step each.
    key(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(text(&h), r#"{"b":[1,{"c":null}],"a":"x"}"#);
}

#[test]
fn invalid_json_moves_the_caret_and_says_why() {
    let source = "{\n  \"a\": 1,\n  \"b\" 2\n}";
    let mut h = with_text(source);
    tool(&mut h, "Validate JSON");
    h.get_by_label_contains("Invalid JSON on line 3, column 7:");
    h.get_by_label_contains("expected ':' after the key but found '2'");
    h.get_by_label("Ln 3, Col 7");
    h.get_by_label("OK").click();
    h.run();

    // Format refuses it the same way and changes nothing.
    key(&mut h, Modifiers::COMMAND, Key::Home);
    tool(&mut h, "Format JSON");
    h.get_by_label_contains("Invalid JSON on line 3, column 7:");
    h.get_by_label("Ln 3, Col 7");
    assert_eq!(text(&h), source);
    h.get_by_label("OK").click();
    h.run();

    // Fixed, it is valid.
    h.event(Event::Text(":".to_owned()));
    h.run();
    tool(&mut h, "Validate JSON");
    h.get_by_label("The document is valid JSON");
}

#[test]
fn base64_and_url_encoding_work_on_the_selection() {
    let mut h = with_text("a b&c");
    key(&mut h, Modifiers::COMMAND, Key::A);
    tool(&mut h, "URL Encode");
    assert_eq!(text(&h), "a%20b%26c");
    tool(&mut h, "URL Decode");
    tool(&mut h, "Base64 Encode");
    assert_eq!(text(&h), "YSBiJmM=");
    tool(&mut h, "Base64 Decode");
    assert_eq!(text(&h), "a b&c");
    // Text that is not Base64 is refused, with a note.
    tool(&mut h, "Base64 Decode");
    assert_eq!(text(&h), "a b&c");
    h.get_by_label_contains("is not valid Base64");
}

#[test]
fn the_hash_dialog_shows_and_copies_hashes() {
    let mut h = with_text("abc");
    tool(&mut h, "Hashes…");
    h.get_by_label("Of the document: 3 bytes of UTF-8.");
    h.get_by_label("900150983cd24fb0d6963f7d28e17f72");
    h.get_by_label("a9993e364706816aba3e25717850c26c9cd0d89d");
    let sha256 = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    h.get_by_label(sha256);
    // The third Copy button is SHA-256's.
    h.get_all_by_label("Copy").nth(2).unwrap().click();
    h.step();
    assert_eq!(copied(&h), vec![sha256.to_owned()]);
    h.run();
    h.get_by_label("Close").click();
    h.run();
    assert!(h.query_by_label(sha256).is_none());

    // A selection is hashed on its own.
    key(&mut h, Modifiers::SHIFT, Key::ArrowLeft);
    tool(&mut h, "Hashes…");
    h.get_by_label("Of the selection: 1 byte of UTF-8.");
    h.get_by_label("4a8a08f09d37b73795649038408b5f33");
}
