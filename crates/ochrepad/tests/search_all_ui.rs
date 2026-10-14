//! Find All and Replace All across open documents, and the Search Results
//! panel.

mod util;

use egui::{Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;
use ochrepad_core::{Action, ByteOffset};

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

fn paste(h: &mut Harness<'_, OchrepadApp>, text: &str) {
    h.event(Event::Paste(text.to_owned()));
    h.run();
}

/// Three documents, "new 1" active again at the end.
fn three_documents(h: &mut Harness<'_, OchrepadApp>) {
    paste(h, "let total = 1;\nprint(total);\n");
    key(h, Modifiers::COMMAND, Key::N);
    paste(h, "nothing here\n");
    key(h, Modifiers::COMMAND, Key::N);
    paste(h, "x\ny = total + total\n");
    let first = h.state().tabs()[0];
    h.state_mut().activate(first);
    h.run();
}

/// Opens the find panel with `pattern` and picks `item` from Find All.
fn find(h: &mut Harness<'_, OchrepadApp>, pattern: &str, item: &str) {
    key(h, Modifiers::COMMAND, Key::F);
    h.event(Event::Text(pattern.to_owned()));
    h.run();
    h.get_by_label("Find All").click();
    h.run();
    h.get_by_label(item).click();
    h.run();
}

fn active_text(h: &Harness<'_, OchrepadApp>) -> String {
    h.state().buffer().doc().to_string()
}

fn selection(h: &Harness<'_, OchrepadApp>) -> (usize, usize) {
    let app = h.state();
    let s = app
        .editor
        .view(app.active_view())
        .unwrap()
        .selections
        .primary()
        .clone();
    (s.start().0, s.end().0)
}

#[test]
fn find_all_in_open_documents_lists_and_goes_to_matches() {
    let mut h = harness(Vec2::new(900.0, 600.0));
    three_documents(&mut h);
    find(&mut h, "total", "In All Open Documents");
    {
        let s = &h.state().results.searches[0];
        assert!(s.done);
        assert_eq!(s.matches, 4);
        let names: Vec<&str> = s.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["new 1", "new 3"]);
    }
    // The match on the third document's second line.
    h.get_by_label_contains("y = total + total").click();
    h.run();
    assert_eq!(h.state().file_name(), "new 3");
    assert_eq!(selection(&h), (6, 11));
}

#[test]
fn a_result_in_an_edited_document_is_found_again_on_its_line() {
    let mut h = harness(Vec2::new(900.0, 600.0));
    paste(&mut h, "a\nlet total = 1;\n");
    find(&mut h, "total", "In Current Document");
    // Edit the line: the match moves right by three bytes, to 9..14.
    let view = h.state().active_view();
    h.state_mut()
        .editor
        .apply(
            view,
            Action::SetSelections(ochrepad_core::SelectionSet::caret(ByteOffset(2))),
        )
        .unwrap();
    h.state_mut()
        .editor
        .apply(view, Action::InsertText("// ".into()))
        .unwrap();
    h.run();
    h.get_by_label_contains("let total = 1;").click();
    h.run();
    assert_eq!(selection(&h), (9, 14));
}

#[test]
fn replace_all_in_open_documents_asks_then_replaces_each_as_one_step() {
    let mut h = harness(Vec2::new(900.0, 600.0));
    three_documents(&mut h);
    key(&mut h, Modifiers::COMMAND, Key::H);
    h.event(Event::Text("total".into()));
    h.run();
    h.state_mut().find.replacement = "sum".into();
    h.get_by_label("Replace All in All Open Documents").click();
    h.run();
    h.get_by_label_contains("Replace 4 matches in 2 open documents");
    h.get_by_label("Replace All Matches").click();
    h.run();
    let texts: Vec<String> = {
        let app = h.state();
        app.tabs()
            .into_iter()
            .map(|v| {
                let b = app.editor.view(v).unwrap().buffer();
                app.editor.buffer(b).unwrap().doc().to_string()
            })
            .collect()
    };
    assert_eq!(
        texts,
        vec![
            "let sum = 1;\nprint(sum);\n",
            "nothing here\n",
            "x\ny = sum + sum\n"
        ]
    );
    assert_eq!(
        h.state().find.message(),
        Some("Replaced 4 occurrences in 2 documents")
    );
    // One undo restores the active document.
    key(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(active_text(&h), "let total = 1;\nprint(total);\n");
}

#[test]
fn the_results_panel_looks_right() {
    let mut h = harness(Vec2::new(760.0, 420.0));
    three_documents(&mut h);
    find(&mut h, "total", "In All Open Documents");
    key(&mut h, Modifiers::NONE, Key::Escape);
    util::snapshot(&mut h, "search_results");
}
