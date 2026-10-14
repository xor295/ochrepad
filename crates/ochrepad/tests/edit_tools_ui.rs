//! Notepad++'s editing extras in the app: line keys, the Column Editor,
//! Blank Operations, the tab menu, Copy to Clipboard, full screen and
//! always on top.

use egui::{Event, Key, Modifiers, Vec2, ViewportCommand, ViewportId};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;

fn harness() -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(1000.0, 500.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.run();
    h
}

fn key(h: &mut Harness<'_, OchrepadApp>, m: Modifiers, k: Key) {
    h.key_press_modifiers(m, k);
    h.run();
}

fn text(h: &Harness<'_, OchrepadApp>) -> String {
    h.state().buffer().doc().to_string()
}

fn typ(h: &mut Harness<'_, OchrepadApp>, s: &str) {
    h.event(Event::Text(s.to_owned()));
    h.run();
}

fn click(h: &mut Harness<'_, OchrepadApp>, label: &str) {
    h.get_by_label(label).click();
    h.run();
}

/// Opens the `menu` menu and hovers its `submenu` (whose label also has
/// the submenu arrow).
fn submenu(h: &mut Harness<'_, OchrepadApp>, menu: &str, submenu: &str) {
    click(h, menu);
    h.get_by_label_contains(submenu).hover();
    h.run();
}

/// The text the last frame put on the clipboard.
fn copied(h: &Harness<'_, OchrepadApp>) -> Option<String> {
    h.output()
        .platform_output
        .commands
        .iter()
        .find_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        })
}

/// The window commands of the last frame.
fn viewport_commands(h: &Harness<'_, OchrepadApp>) -> Vec<ViewportCommand> {
    h.output()
        .viewport_output
        .get(&ViewportId::ROOT)
        .map(|v| v.commands.clone())
        .unwrap_or_default()
}

#[test]
fn notepad_plus_plus_line_keys() {
    let mut h = harness();
    h.event(Event::Paste("  one\ntwo\nthree".into()));
    h.run();
    // The caret is at the end of "three"; after the swap it is at the start
    // of its line, which now holds "two".
    key(&mut h, Modifiers::COMMAND, Key::T);
    assert_eq!(text(&h), "  one\nthree\ntwo");
    key(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::L);
    assert_eq!(text(&h), "  one\nthree");
    key(&mut h, Modifiers::COMMAND, Key::Home);
    key(
        &mut h,
        Modifiers::COMMAND | Modifiers::ALT | Modifiers::SHIFT,
        Key::Enter,
    );
    typ(&mut h, "x");
    assert_eq!(text(&h), "  one\n  x\nthree", "indented like the line");
    key(&mut h, Modifiers::COMMAND | Modifiers::ALT, Key::Enter);
    typ(&mut h, "y");
    assert_eq!(text(&h), "  one\n  y\n  x\nthree");
}

#[test]
fn the_column_editor_inserts_text_or_numbers_down_the_column() {
    let mut h = harness();
    h.event(Event::Paste("a\nb\nc\n".into()));
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::Home);
    key(&mut h, Modifiers::ALT, Key::C);
    h.get_by_label("Column Editor");
    // The text field has the keyboard.
    typ(&mut h, "> ");
    key(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(text(&h), "> a\n> b\n> c\n");
    assert!(h.query_by_label("Column Editor").is_none(), "closed");

    // Numbers, from the caret's column (after "> " on the first line).
    key(&mut h, Modifiers::ALT, Key::C);
    click(&mut h, "Number to Insert");
    key(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(text(&h), "> 1a\n> 2b\n> 3c\n");
    // One undo step.
    key(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(text(&h), "> a\n> b\n> c\n");

    // Bad numbers keep the dialog open with a reason, and Escape cancels.
    key(&mut h, Modifiers::ALT, Key::C);
    key(&mut h, Modifiers::COMMAND, Key::A);
    typ(&mut h, "x");
    key(&mut h, Modifiers::NONE, Key::Enter);
    h.get_by_label_contains("must be a whole number");
    key(&mut h, Modifiers::NONE, Key::Escape);
    assert!(h.query_by_label("Column Editor").is_none());
    assert_eq!(text(&h), "> a\n> b\n> c\n");
}

#[test]
fn blank_operations_from_the_edit_menu() {
    let mut h = harness();
    h.event(Event::Paste("\ta\n  b  ".into()));
    h.run();
    submenu(&mut h, "Edit", "Blank Operations");
    click(&mut h, "TAB to Space");
    assert_eq!(text(&h), "    a\n  b  ");
    submenu(&mut h, "Edit", "Blank Operations");
    click(&mut h, "Trim Leading and Trailing Space");
    assert_eq!(text(&h), "a\nb");
}

#[test]
fn the_tab_menu_closes_tabs_beside_and_unchanged() {
    let mut h = harness();
    key(&mut h, Modifiers::COMMAND, Key::N);
    key(&mut h, Modifiers::COMMAND, Key::N);
    let tabs = |h: &Harness<'_, OchrepadApp>| h.state().tabs().len();
    assert_eq!(tabs(&h), 3);
    h.get_by_label("new 2").click_secondary();
    h.run();
    click(&mut h, "Close All to the Right");
    assert_eq!(tabs(&h), 2);
    assert!(h.query_by_label("new 3").is_none());
    h.get_by_label("new 2").click_secondary();
    h.run();
    click(&mut h, "Close All to the Left");
    assert_eq!(tabs(&h), 1);
    assert!(h.query_by_label("new 1").is_none());

    // Unchanged tabs go; a tab with unsaved changes stays.
    typ(&mut h, "unsaved");
    key(&mut h, Modifiers::COMMAND, Key::N);
    key(&mut h, Modifiers::COMMAND, Key::N);
    assert_eq!(tabs(&h), 3);
    h.get_by_label("new 2 *").click_secondary();
    h.run();
    click(&mut h, "Close All Unchanged");
    assert_eq!(tabs(&h), 1);
    h.get_by_label("new 2 *");
}

#[test]
fn ctrl_i_splits_lines_at_the_vertical_edge() {
    let mut h = harness();
    h.state_mut().settings.edge_column = Some(10);
    h.run();
    h.event(Event::Paste("one two three four five".into()));
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::I);
    assert_eq!(text(&h), "one two\nthree four\nfive");
    // From the menu too, after undoing.
    key(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(text(&h), "one two three four five");
    submenu(&mut h, "Edit", "Line Operations");
    h.get_by_label_contains("Split Lines").click();
    h.run();
    assert_eq!(text(&h), "one two\nthree four\nfive");
}

#[test]
fn the_tab_menu_saves_and_sets_read_only_for_its_tab() {
    let tmp = tempfile::tempdir().unwrap();
    let (a, b) = (tmp.path().join("a.txt"), tmp.path().join("b.txt"));
    std::fs::write(&a, "a").unwrap();
    std::fs::write(&b, "b").unwrap();
    let mut h = harness();
    h.state_mut().open_path(&a);
    h.run();
    typ(&mut h, "new ");
    h.state_mut().open_path(&b);
    h.run();
    assert_eq!(h.state().file_name(), "b.txt");

    // Save on a tab in the background saves that tab's document.
    h.get_by_label("a.txt *").click_secondary();
    h.run();
    click(&mut h, "Save");
    h.state_mut().flush_disk();
    h.run();
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "new a");
    assert_eq!(h.state().file_name(), "a.txt");
    assert!(!h.state().buffer().is_dirty());

    // Read-Only, for the tab right-clicked.
    h.get_by_label("b.txt").click_secondary();
    h.run();
    click(&mut h, "Read-Only");
    let app = h.state();
    let read_only = |name: &str| {
        app.editor
            .buffers()
            .find(|buf| buf.meta.path.as_deref().and_then(|p| p.file_name()) == Some(name.as_ref()))
            .is_some_and(|buf| buf.read_only)
    };
    assert!(read_only("b.txt"));
    assert!(!read_only("a.txt"));
}

#[test]
fn copying_paths_from_the_tab_and_edit_menus() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("notes.txt");
    std::fs::write(&file, "x").unwrap();
    let mut h = harness();
    h.state_mut().open_path(&file);
    h.run();
    h.get_by_label("notes.txt").click_secondary();
    h.run();
    h.get_by_label("Copy Full File Path").click();
    h.step();
    assert_eq!(copied(&h), Some(file.display().to_string()));
    h.run();

    submenu(&mut h, "Edit", "Copy to Clipboard");
    h.get_by_label("Copy Folder Path").click();
    h.step();
    assert_eq!(copied(&h), Some(tmp.path().display().to_string()));
    h.run();
    submenu(&mut h, "Edit", "Copy to Clipboard");
    h.get_by_label("Copy File Name").click();
    h.step();
    assert_eq!(copied(&h), Some("notes.txt".to_owned()));
}

#[test]
fn reload_from_disk_asks_first_when_there_are_unsaved_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("r.txt");
    std::fs::write(&file, "on disk\n").unwrap();
    let mut h = harness();
    h.state_mut().open_path(&file);
    h.run();
    typ(&mut h, "edited ");
    h.get_by_label("r.txt *").click_secondary();
    h.run();
    click(&mut h, "Reload from Disk");
    h.get_by_label_contains("Reload r.txt from disk?");
    click(&mut h, "Reload");
    assert_eq!(text(&h), "on disk\n");
    assert!(!h.state().buffer().is_dirty());
    // Undo brings the edit back.
    key(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(text(&h), "edited on disk\n");
}

#[test]
fn full_screen_and_always_on_top_ask_the_window() {
    let mut h = harness();
    // Only the press: a step runs the release in a later pass, and the
    // harness keeps the last pass's output.
    h.event(Event::Key {
        key: Key::F11,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    assert!(
        viewport_commands(&h).contains(&ViewportCommand::Fullscreen(true)),
        "{:?}",
        viewport_commands(&h)
    );
    h.run();
    assert!(!h.state().settings.always_on_top);
    click(&mut h, "View");
    h.get_by_label("Always on Top").click();
    h.step();
    assert!(h.state().settings.always_on_top);
    assert!(
        viewport_commands(&h).contains(&ViewportCommand::WindowLevel(
            egui::WindowLevel::AlwaysOnTop
        )),
        "{:?}",
        viewport_commands(&h)
    );
}
