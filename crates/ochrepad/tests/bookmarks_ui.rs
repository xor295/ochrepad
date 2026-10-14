//! Bookmarks in the app: the keys, Search > Bookmark, the find panel's
//! Bookmark button, the right-click menu and the session.

mod util;

use egui::{Event, FontId, Key, Modifiers, PointerButton, Pos2, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;

fn app_harness(app: OchrepadApp) -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(900.0, 400.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), app);
    h.run();
    h
}

fn harness() -> Harness<'static, OchrepadApp> {
    app_harness(OchrepadApp::new())
}

fn key(h: &mut Harness<'_, OchrepadApp>, m: Modifiers, k: Key) {
    h.key_press_modifiers(m, k);
    h.run();
}

fn text(h: &Harness<'_, OchrepadApp>) -> String {
    h.state().buffer().doc().to_string()
}

fn marks(h: &Harness<'_, OchrepadApp>) -> Vec<usize> {
    h.state().buffer().bookmarks().lines().to_vec()
}

fn caret_line(h: &Harness<'_, OchrepadApp>) -> usize {
    let app = h.state();
    let head = app
        .editor
        .view(app.active_view())
        .unwrap()
        .selections
        .primary()
        .head;
    app.buffer().doc().line_of(head)
}

fn click(h: &mut Harness<'_, OchrepadApp>, label: &str) {
    h.get_by_label(label).click();
    h.run();
}

/// Opens Search > Bookmark.
fn bookmark_menu(h: &mut Harness<'_, OchrepadApp>) {
    click(h, "Search");
    h.get_by_label_contains("Bookmark ⏵").hover();
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

/// The middle of line `line`'s number in the gutter (zoom 100%).
fn gutter_pos(h: &Harness<'_, OchrepadApp>, line: usize) -> Pos2 {
    let rect = h.get_by_label("Editor: new 1").rect();
    let font = FontId::monospace(14.0);
    let (cell_w, line_h) = h
        .ctx
        .fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
    Pos2::new(
        rect.min.x + 1.5 * cell_w,
        rect.min.y + (line as f32 + 0.5) * line_h,
    )
}

fn right_click(h: &mut Harness<'_, OchrepadApp>, pos: Pos2) {
    h.event(Event::PointerMoved(pos));
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    h.run();
}

const LOG: &str = "INFO start\nERROR disk\nINFO retry\nERROR gave up\nINFO done";

#[test]
fn keys_bookmark_and_go_from_one_to_the_next() {
    let mut h = harness();
    h.event(Event::Paste(LOG.into()));
    h.run();
    // The caret is on the last line.
    key(&mut h, Modifiers::COMMAND, Key::F2);
    key(&mut h, Modifiers::COMMAND, Key::Home);
    key(&mut h, Modifiers::NONE, Key::ArrowDown);
    key(&mut h, Modifiers::COMMAND, Key::F2);
    assert_eq!(marks(&h), vec![1, 4]);
    key(&mut h, Modifiers::NONE, Key::F2);
    assert_eq!(caret_line(&h), 4);
    key(&mut h, Modifiers::NONE, Key::F2);
    assert_eq!(caret_line(&h), 1, "round to the first");
    key(&mut h, Modifiers::SHIFT, Key::F2);
    assert_eq!(caret_line(&h), 4);
    // Ctrl+F2 again removes it.
    key(&mut h, Modifiers::COMMAND, Key::F2);
    assert_eq!(marks(&h), vec![1]);
}

#[test]
fn the_find_panel_bookmarks_the_matching_lines_and_the_menu_copies_them() {
    let mut h = harness();
    h.event(Event::Paste(LOG.into()));
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::F);
    h.event(Event::Text("ERROR".into()));
    h.run();
    click(&mut h, "Bookmark");
    assert_eq!(marks(&h), vec![1, 3]);
    h.get_by_label("2 lines bookmarked");

    bookmark_menu(&mut h);
    h.get_by_label("Copy Bookmarked Lines").click();
    h.step();
    assert_eq!(copied(&h).as_deref(), Some("ERROR disk\nERROR gave up\n"));
    h.run();

    bookmark_menu(&mut h);
    click(&mut h, "Remove Unmarked Lines");
    // The last line goes with the line break before it.
    assert_eq!(text(&h), "ERROR disk\nERROR gave up");
    key(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(text(&h), LOG);

    bookmark_menu(&mut h);
    h.get_by_label("Cut Bookmarked Lines").click();
    h.step();
    assert_eq!(copied(&h).as_deref(), Some("ERROR disk\nERROR gave up\n"));
    h.run();
    assert_eq!(text(&h), "INFO start\nINFO retry\nINFO done");
    assert!(marks(&h).is_empty());
}

#[test]
fn paste_to_bookmarked_lines_replaces_their_text() {
    let mut h = harness();
    h.event(Event::Paste("a\nb\nc".into()));
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::F2);
    bookmark_menu(&mut h);
    click(&mut h, "Paste to (Replace) Bookmarked Lines");
    // The clipboard's text arrives as a paste.
    h.event(Event::Paste("X".into()));
    h.run();
    assert_eq!(text(&h), "a\nb\nX");
    // The next paste is an ordinary one.
    h.event(Event::Paste("!".into()));
    h.run();
    assert_eq!(text(&h), "a\nb\nX!");
}

#[test]
fn right_click_on_a_line_number_bookmarks_that_line() {
    let mut h = harness();
    h.event(Event::Paste("one\ntwo\nthree".into()));
    h.run();
    let pos = gutter_pos(&h, 1);
    right_click(&mut h, pos);
    // The caret went to the line clicked.
    assert_eq!(caret_line(&h), 1);
    h.get_by_label_contains("Add Bookmark").click();
    h.run();
    assert_eq!(marks(&h), vec![1]);
    right_click(&mut h, pos);
    h.get_by_label_contains("Remove Bookmark").click();
    h.run();
    assert!(marks(&h).is_empty());
}

#[test]
fn right_click_keeps_the_selection_and_copies_it() {
    let mut h = harness();
    h.event(Event::Paste("one two".into()));
    h.run();
    // Select "two" (the caret is at the end): Ctrl+Shift+Left.
    key(
        &mut h,
        Modifiers::COMMAND | Modifiers::SHIFT,
        Key::ArrowLeft,
    );
    let rect = h.get_by_label("Editor: new 1").rect();
    let font = FontId::monospace(14.0);
    let (cell_w, line_h) = h
        .ctx
        .fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
    // On "two": gutter (3 digits, fold margin, padding), then 5 cells in.
    let on_two = Pos2::new(
        rect.min.x + 5.0 * cell_w + 8.0 + 5.5 * cell_w,
        rect.min.y + 0.5 * line_h,
    );
    right_click(&mut h, on_two);
    h.get_by_label_contains("Copy").click();
    h.step();
    assert_eq!(copied(&h).as_deref(), Some("two"));
    h.run();
    // Elsewhere, the caret moves there and the selection goes.
    let on_one = Pos2::new(on_two.x - 5.0 * cell_w, on_two.y);
    right_click(&mut h, on_one);
    let app = h.state();
    let sel = app
        .editor
        .view(app.active_view())
        .unwrap()
        .selections
        .primary()
        .clone();
    assert!(sel.is_empty());
    // With no selection, the caret's line.
    h.get_by_label_contains("UPPERCASE").click();
    h.run();
    assert_eq!(text(&h), "ONE TWO");
}

#[test]
fn the_session_keeps_bookmarks() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, "a\nb\nc\nd\n").unwrap();
    let config = dir.path().join("config");
    let data = dir.path().join("data");
    let mut app = OchrepadApp::new();
    app.set_storage(&config, &data);
    let mut h = app_harness(app);
    h.state_mut().open_path(&file);
    h.run();
    let b = h.state().buffer_id();
    h.state_mut()
        .editor
        .buffer_mut(b)
        .unwrap()
        .set_bookmarks([1, 3]);
    h.state_mut().request_exit();
    h.run();
    h.state_mut().flush_disk();
    drop(h);
    let mut app = OchrepadApp::new();
    app.set_storage(&config, &data);
    let h = app_harness(app);
    assert_eq!(h.state().file_name(), "notes.txt");
    assert_eq!(marks(&h), vec![1, 3]);
}

#[test]
fn a_bookmark_looks_right() {
    let mut h = harness();
    h.event(Event::Paste(
        "fn main() {\n    let x = 1;\n    println!(\"{x}\");\n}\n".into(),
    ));
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::Home);
    key(&mut h, Modifiers::NONE, Key::ArrowDown);
    key(&mut h, Modifiers::COMMAND, Key::F2);
    key(&mut h, Modifiers::NONE, Key::ArrowDown);
    key(&mut h, Modifiers::NONE, Key::ArrowDown);
    key(&mut h, Modifiers::COMMAND, Key::F2);
    // The right-click menu, opened on the text so the gutter shows.
    let font = FontId::monospace(14.0);
    let cell_w = h.ctx.fonts_mut(|f| f.glyph_width(&font, 'M'));
    let pos = gutter_pos(&h, 2) + Vec2::new(24.0 * cell_w, 0.0);
    right_click(&mut h, pos);
    util::snapshot(&mut h, "bookmarks");
}
