//! egui_kittest tests driving keystrokes and mouse input into the editor and
//! asserting the buffer text and selections.

use egui::{Event, FontId, ImeEvent, Key, Modifiers, PointerButton, Pos2, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;
use ochrepad_core::{ByteOffset, Selection};

fn harness() -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(800.0, 600.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.run();
    h
}

fn with_text(text: &str) -> Harness<'static, OchrepadApp> {
    let mut h = harness();
    h.event(Event::Paste(text.to_owned()));
    h.run();
    h
}

fn text(h: &Harness<'_, OchrepadApp>) -> String {
    h.state().buffer().doc().to_string()
}

fn sels(h: &Harness<'_, OchrepadApp>) -> Vec<(usize, usize)> {
    let app = h.state();
    app.editor
        .view(app.view)
        .unwrap()
        .selections
        .ranges()
        .iter()
        .map(|s| (s.anchor.0, s.head.0))
        .collect()
}

fn typ(h: &mut Harness<'_, OchrepadApp>, s: &str) {
    h.event(Event::Text(s.to_owned()));
    h.run();
}

fn key(h: &mut Harness<'_, OchrepadApp>, m: Modifiers, k: Key) {
    h.key_press_modifiers(m, k);
    h.run();
}

/// Screen position of (line, column) in the editor, at the cell's center.
fn cell_pos(h: &Harness<'_, OchrepadApp>, line: usize, col: f32) -> Pos2 {
    let rect = h.get_by_label("Editor: new 1").rect();
    let font = FontId::monospace(14.0);
    let (cell_w, line_h) = h
        .ctx
        .fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
    // Gutter: 3 digits + 1 cell + 4 px padding; text starts 4 px further in.
    let x = rect.min.x + 4.0 * cell_w + 8.0 + col * cell_w;
    Pos2::new(x, rect.min.y + (line as f32 + 0.5) * line_h)
}

fn press(h: &mut Harness<'_, OchrepadApp>, pos: Pos2, pressed: bool, modifiers: Modifiers) {
    h.event(Event::PointerMoved(pos));
    h.event(Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers,
    });
}

fn click(h: &mut Harness<'_, OchrepadApp>, pos: Pos2, modifiers: Modifiers) {
    h.event(Event::ModifiersChanged(modifiers));
    press(h, pos, true, modifiers);
    h.step();
    press(h, pos, false, modifiers);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run();
}

#[test]
fn typing_newline_and_status_bar() {
    let mut h = harness();
    typ(&mut h, "hello");
    key(&mut h, Modifiers::NONE, Key::Enter);
    typ(&mut h, "wörld");
    assert_eq!(text(&h), "hello\nwörld");
    h.get_by_label("Ln 2, Col 6");
    h.get_by_label("Editor: new 1");
    assert!(h.state().title().starts_with("*new 1"));
}

#[test]
fn shift_arrows_select_and_typing_replaces() {
    let mut h = with_text("abc");
    key(&mut h, Modifiers::NONE, Key::Home);
    key(&mut h, Modifiers::SHIFT, Key::ArrowRight);
    key(&mut h, Modifiers::SHIFT, Key::ArrowRight);
    assert_eq!(sels(&h), vec![(0, 2)]);
    typ(&mut h, "X");
    assert_eq!(text(&h), "Xc");
}

#[test]
fn delete_word_undo_redo() {
    let mut h = with_text("foo bar");
    key(&mut h, Modifiers::COMMAND, Key::Backspace);
    assert_eq!(text(&h), "foo ");
    key(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(text(&h), "foo bar");
    key(&mut h, Modifiers::COMMAND, Key::Y);
    assert_eq!(text(&h), "foo ");
}

#[test]
fn tab_inserts_a_tab_instead_of_moving_focus() {
    let mut h = harness();
    key(&mut h, Modifiers::NONE, Key::Tab);
    key(&mut h, Modifiers::NONE, Key::Tab);
    typ(&mut h, "x");
    assert_eq!(text(&h), "\t\tx");
    h.get_by_label("Ln 1, Col 10");
}

#[test]
fn multi_cursor_typing() {
    let mut h = with_text("ab\ncd\nef");
    key(&mut h, Modifiers::COMMAND, Key::Home);
    key(&mut h, Modifiers::COMMAND | Modifiers::ALT, Key::ArrowDown);
    key(&mut h, Modifiers::COMMAND | Modifiers::ALT, Key::ArrowDown);
    typ(&mut h, "-");
    assert_eq!(text(&h), "-ab\n-cd\n-ef");
    h.get_by_label("3 selections");
    key(&mut h, Modifiers::NONE, Key::Escape);
    assert_eq!(sels(&h).len(), 1);
}

#[test]
fn copy_cut_paste() {
    let mut h = with_text("one two");
    key(&mut h, Modifiers::COMMAND, Key::A);
    h.event(Event::Copy);
    // `output()` is the last frame's; the copy happens on this one.
    h.step();
    let copied: Vec<_> = h
        .output()
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(copied, vec!["one two".to_owned()]);
    h.event(Event::Cut);
    h.run();
    assert_eq!(text(&h), "");
    // Pasted text is normalized to LF.
    h.event(Event::Paste("a\r\nb".into()));
    h.run();
    assert_eq!(text(&h), "a\nb");
}

#[test]
fn multi_caret_paste_distributes_lines() {
    let mut h = with_text("x\ny");
    key(&mut h, Modifiers::COMMAND, Key::Home);
    key(&mut h, Modifiers::COMMAND | Modifiers::ALT, Key::ArrowDown);
    h.event(Event::Paste("1\n2".into()));
    h.run();
    assert_eq!(text(&h), "1x\n2y");
}

#[test]
fn page_down_and_caret_stays_visible() {
    let lines: Vec<String> = (0..1000).map(|i| format!("line {i}")).collect();
    let mut h = with_text(&lines.join("\n"));
    key(&mut h, Modifiers::COMMAND, Key::Home);
    key(&mut h, Modifiers::NONE, Key::PageDown);
    let page_line = h.state().buffer().doc().line_of(ByteOffset(sels(&h)[0].1));
    assert!(page_line > 10, "page down moved to line {page_line}");
    key(&mut h, Modifiers::COMMAND, Key::End);
    let app = h.state();
    let scroll = app.editor.view(app.view).unwrap().scroll;
    assert!(scroll.first_line > 950, "scrolled to {}", scroll.first_line);
}

#[test]
fn click_places_caret_using_layout() {
    let mut h = with_text("ab\tc漢字x\nsecond");
    // Bytes: a0 b1 tab2 c3 漢4 字7 x10. Columns: a0 b1 tab2..4 c4 漢5..7 字7..9 x9.
    let p = cell_pos(&h, 0, 7.6);
    click(&mut h, p, Modifiers::NONE);
    // 7.6 is inside 字 (columns 7..9), nearer its start.
    assert_eq!(sels(&h), vec![(7, 7)]);
    let p = cell_pos(&h, 0, 3.1);
    click(&mut h, p, Modifiers::NONE);
    // Inside the tab (2..4), nearer its end.
    assert_eq!(sels(&h), vec![(3, 3)]);
    let p = cell_pos(&h, 1, 30.0);
    click(&mut h, p, Modifiers::SHIFT);
    assert_eq!(sels(&h), vec![(3, 18)]);
}

#[test]
fn double_and_triple_click() {
    let mut h = with_text("alpha beta gamma\nnext");
    let p = cell_pos(&h, 0, 7.5);
    click(&mut h, p, Modifiers::NONE);
    click(&mut h, p, Modifiers::NONE);
    assert_eq!(sels(&h), vec![(6, 10)]);
    click(&mut h, p, Modifiers::NONE);
    assert_eq!(sels(&h), vec![(0, 17)]);
}

#[test]
fn drag_selects() {
    let mut h = with_text("alpha beta\ngamma");
    let a = cell_pos(&h, 0, 1.0);
    let b = cell_pos(&h, 1, 3.0);
    press(&mut h, a, true, Modifiers::NONE);
    h.step();
    for i in 1..=5 {
        let t = i as f32 / 5.0;
        h.event(Event::PointerMoved(a + (b - a) * t));
        h.step();
    }
    press(&mut h, b, false, Modifiers::NONE);
    h.run();
    assert_eq!(sels(&h), vec![(1, 14)]);
}

#[test]
fn ime_preedit_is_shown_not_inserted_until_commit() {
    let mut h = with_text("a");
    h.event(Event::Ime(ImeEvent::Preedit {
        text: "ni".into(),
        active_range_chars: None,
    }));
    h.run();
    assert_eq!(text(&h), "a");
    assert_eq!(h.state().view_state.preedit(), Some("ni"));
    h.event(Event::Ime(ImeEvent::Commit("你".into())));
    h.run();
    assert_eq!(text(&h), "a你");
    assert_eq!(h.state().view_state.preedit(), None);
    let ime = h.output().platform_output.ime;
    assert!(ime.is_some(), "IME cursor position is reported");
}

#[test]
fn new_file_with_unsaved_changes_asks_first() {
    let mut h = with_text("draft");
    key(&mut h, Modifiers::COMMAND, Key::N);
    assert_eq!(text(&h), "draft");
    h.get_by_label("Save changes to new 1?");
    h.get_by_label("Cancel").click();
    h.run();
    assert_eq!(text(&h), "draft");
    key(&mut h, Modifiers::COMMAND, Key::N);
    h.get_by_label("Don't Save").click();
    h.run();
    assert_eq!(text(&h), "");
    assert!(!h.state().buffer().is_dirty());
}

#[test]
fn open_shows_format_and_save_writes_it_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("doc.txt");
    let original = b"\xff\xfeh\x00i\x00\r\x00\n\x00";
    std::fs::write(&path, original).unwrap();

    let mut h = harness();
    h.state_mut().open_path(&path);
    h.run();
    h.get_by_label("UTF-16LE BOM");
    h.get_by_label("Windows (CR LF)");
    assert_eq!(h.state().title(), "doc.txt - ochrepad");

    key(&mut h, Modifiers::COMMAND, Key::End);
    typ(&mut h, "!");
    assert_eq!(h.state().title(), "*doc.txt - ochrepad");
    key(&mut h, Modifiers::COMMAND, Key::S);
    assert!(!h.state().buffer().is_dirty());
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"\xff\xfeh\x00i\x00\r\x00\n\x00!\x00"
    );
}

#[test]
fn open_failure_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bin.dat");
    std::fs::write(&path, b"\x00\x01\x02").unwrap();
    let mut h = harness();
    h.state_mut().open_path(&path);
    h.run();
    assert!(h.state().message().unwrap().contains("binary"));
    // The previous (empty) buffer is still there.
    assert_eq!(text(&h), "");
}

#[test]
fn selection_api_round_trip() {
    // Sanity check that SetSelections from the widget clamps like core says.
    let mut h = with_text("é");
    let app = h.state_mut();
    let _ = app.editor.apply(
        app.view,
        ochrepad_core::Action::SetSelections(ochrepad_core::SelectionSet::single(Selection::new(
            ByteOffset(1),
            ByteOffset(50),
        ))),
    );
    h.run();
    assert_eq!(sels(&h), vec![(0, 2)]);
}
