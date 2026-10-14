//! View > Document List: the panel of the open documents.

use std::path::Path;

use egui::accesskit::{Role, Toggled};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::{Harness, Node};
use ochrepad::OchrepadApp;

fn harness_with(app: OchrepadApp) -> Harness<'static, OchrepadApp> {
    // Tall enough for the whole View menu.
    let mut h = Harness::builder()
        .with_size(Vec2::new(900.0, 700.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), app);
    h.run();
    h
}

fn harness() -> Harness<'static, OchrepadApp> {
    harness_with(OchrepadApp::new())
}

/// Whether `n` is one of the list's rows: tab buttons have the same labels,
/// but only rows can be selected.
fn is_row(n: &egui_kittest::kittest::AccessKitNode<'_>) -> bool {
    n.role() == Role::Button && n.toggled().is_some()
}

/// The rows' labels, top to bottom.
fn rows(h: &Harness<'_, OchrepadApp>) -> Vec<String> {
    h.query_all_by(is_row)
        .filter_map(|n| n.accesskit_node().label())
        .collect()
}

fn row<'a>(h: &'a Harness<'_, OchrepadApp>, title: &'static str) -> Node<'a> {
    h.get_by(move |n| is_row(n) && n.label().as_deref() == Some(title))
}

/// The rows marked as the active document's.
fn active_rows(h: &Harness<'_, OchrepadApp>) -> Vec<String> {
    h.query_all_by(|n| is_row(n) && n.toggled() == Some(Toggled::True))
        .filter_map(|n| n.accesskit_node().label())
        .collect()
}

fn file(dir: &Path, name: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, name).unwrap();
    path
}

fn press(h: &mut Harness<'_, OchrepadApp>, pos: Pos2, pressed: bool) {
    h.event(Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    });
    h.step();
}

#[test]
fn the_list_switches_documents_and_closes_them() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness();
    h.state_mut().open_path(&file(dir.path(), "a.txt"));
    h.state_mut().open_path(&file(dir.path(), "b.txt"));
    h.state_mut().new_file();
    h.run();
    h.get_by_label("View").click();
    h.run();
    h.get_by_label("Document List").click();
    h.run();
    assert!(h.state().settings.show_document_list);
    h.key_press(Key::Escape);
    h.run();
    // One row per tab, in tab order, with the active document marked.
    assert_eq!(rows(&h), ["a.txt", "b.txt", "new 2"]);
    assert_eq!(active_rows(&h), ["new 2"]);
    // A click switches to the document, and its editor takes the keyboard.
    row(&h, "a.txt").click();
    h.run();
    assert_eq!(h.state().file_name(), "a.txt");
    assert_eq!(active_rows(&h), ["a.txt"]);
    h.event(Event::Text("!".into()));
    h.run();
    assert_eq!(h.state().buffer().doc().to_string(), "!a.txt");
    // Rows show the tab's marks.
    assert_eq!(rows(&h), ["a.txt *", "b.txt", "new 2"]);
    // A middle click closes a tab, asking first about unsaved changes.
    let a = h.state().buffer_id();
    row(&h, "b.txt").click_button(PointerButton::Middle);
    h.run();
    assert_eq!(rows(&h), ["a.txt *", "new 2"]);
    row(&h, "a.txt *").click_button(PointerButton::Middle);
    h.run();
    assert_eq!(h.state().confirming(), Some(a));
    h.get_by_label("Cancel").click();
    h.run();
    assert_eq!(rows(&h), ["a.txt *", "new 2"]);
    row(&h, "a.txt *").click_button(PointerButton::Middle);
    h.run();
    h.get_by_label("Don't Save").click();
    h.run();
    assert_eq!(rows(&h), ["new 2"]);
    // × hides the panel.
    h.get_by_label("×").click();
    h.run();
    assert!(!h.state().settings.show_document_list);
    assert!(rows(&h).is_empty());
}

#[test]
fn viewers_are_listed_but_previews_and_comparisons_are_not() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness();
    h.state_mut().settings.show_document_list = true;
    h.state_mut().open_path(&file(dir.path(), "notes.md"));
    let notes = h.state().buffer_id();
    h.state_mut().open_path(&file(dir.path(), "other.md"));
    let other = h.state().buffer_id();
    h.state_mut().open_preview();
    h.state_mut().compare(notes, other);
    let log = h
        .state_mut()
        .open_viewer(&file(dir.path(), "big.log"))
        .expect("opens");
    h.run();
    assert_eq!(rows(&h), ["notes.md", "other.md", "big.log (read-only)"]);
    assert_eq!(active_rows(&h), ["big.log (read-only)"]);
    row(&h, "notes.md").click();
    h.run();
    assert_eq!(h.state_mut().focused_viewer(), None);
    assert_eq!(h.state().file_name(), "notes.md");
    row(&h, "big.log (read-only)").click();
    h.run();
    assert_eq!(h.state_mut().focused_viewer(), Some(log));
    row(&h, "big.log (read-only)").click_button(PointerButton::Middle);
    h.run();
    assert!(h.state().viewers.get(log).is_none());
    assert_eq!(rows(&h), ["notes.md", "other.md"]);
}

#[test]
fn the_active_row_is_scrolled_into_view() {
    let mut h = harness();
    h.state_mut().settings.show_document_list = true;
    for _ in 0..60 {
        h.state_mut().new_file();
    }
    h.run();
    // Only the rows on screen are drawn: the newest, active one is, and
    // the first is scrolled away.
    let shown = rows(&h);
    assert!(shown.contains(&"new 61".to_owned()), "{shown:?}");
    assert!(!shown.contains(&"new 1".to_owned()), "{shown:?}");
    let first = h.state().tabs()[0];
    h.state_mut().activate(first);
    h.run();
    assert_eq!(active_rows(&h), ["new 1"]);
}

#[test]
fn the_panel_and_its_width_are_remembered() {
    let dir = tempfile::tempdir().unwrap();
    let (config, data) = (dir.path().join("config"), dir.path().join("data"));
    let width = {
        let mut app = OchrepadApp::new();
        app.set_storage(&config, &data);
        let mut h = harness_with(app);
        h.state_mut().settings.show_document_list = true;
        h.run();
        let before = h.state().settings.document_list_width;
        // Drag the panel's left edge 60 points further left (grabbed just
        // inside it: the editor is on top just outside).
        let edge = row(&h, "new 1").rect().left_center() + Vec2::new(2.0, 0.0);
        h.event(Event::PointerMoved(edge));
        press(&mut h, edge, true);
        for dx in [20.0, 40.0, 60.0] {
            h.event(Event::PointerMoved(edge - Vec2::new(dx, 0.0)));
            h.step();
        }
        press(&mut h, edge - Vec2::new(60.0, 0.0), false);
        h.run();
        // The edge follows the pointer, which held it a few points inside.
        let width = h.state().settings.document_list_width;
        assert!((width - before - 60.0).abs() <= 4.0, "{before} -> {width}");
        h.state_mut().flush_disk();
        width
    };
    let mut app = OchrepadApp::new();
    app.set_storage(&config, &data);
    let h = harness_with(app);
    assert!(h.state().settings.show_document_list);
    assert_eq!(h.state().settings.document_list_width, width);
    assert_eq!(rows(&h), ["new 1"]);
}
