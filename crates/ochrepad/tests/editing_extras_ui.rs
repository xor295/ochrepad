//! More Notepad++ conveniences: the selection's size in the status bar,
//! Select and Find Next, Restore Recent Closed File, middle-click to close
//! a tab, Paste from History, Multi-select and Read-Only.

use egui::{Event, Key, Modifiers, PointerButton, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;

fn harness() -> Harness<'static, OchrepadApp> {
    // Tall enough for the whole Edit menu.
    let mut h = Harness::builder()
        .with_size(Vec2::new(900.0, 800.0))
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

fn selected(h: &Harness<'_, OchrepadApp>) -> String {
    let app = h.state();
    let v = app.editor.view(app.active_view()).unwrap();
    app.buffer().doc().slice(v.selections.primary().range())
}

fn primary(h: &Harness<'_, OchrepadApp>) -> (usize, usize) {
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

fn click(h: &mut Harness<'_, OchrepadApp>, label: &str) {
    h.get_by_label(label).click();
    h.run();
}

fn submenu(h: &mut Harness<'_, OchrepadApp>, menu: &str, submenu: &str) {
    click(h, menu);
    h.get_by_label_contains(submenu).hover();
    h.run();
}

#[test]
fn the_status_bar_shows_how_much_is_selected() {
    let mut h = harness();
    h.event(Event::Paste("héllo\nworld\nthree".into()));
    h.run();
    assert!(h.state().selection_status().is_none());
    key(&mut h, Modifiers::SHIFT, Key::ArrowLeft);
    h.get_by_label("1 selected");
    key(&mut h, Modifiers::COMMAND, Key::A);
    h.get_by_label("17 selected, 3 lines");
    // Whole lines end at the start of the next: that one doesn't count.
    key(&mut h, Modifiers::COMMAND, Key::Home);
    key(&mut h, Modifiers::SHIFT, Key::ArrowDown);
    h.get_by_label("6 selected");
}

#[test]
fn select_and_find_next_takes_the_word_at_the_caret() {
    let mut h = harness();
    h.event(Event::Paste("total = price * total;\nprint(total)".into()));
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::Home);
    key(&mut h, Modifiers::COMMAND, Key::F3);
    assert_eq!(selected(&h), "total");
    assert_eq!(primary(&h), (16, 21));
    // F3 goes on with it, without the find panel.
    key(&mut h, Modifiers::NONE, Key::F3);
    assert_eq!(primary(&h), (29, 34));
    assert!(!h.state().find.open);
    key(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::F3);
    assert_eq!(primary(&h), (16, 21));
}

#[test]
fn a_closed_file_comes_back_with_ctrl_shift_t() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    std::fs::write(&a, "a").unwrap();
    std::fs::write(&b, "b").unwrap();
    let mut h = harness();
    h.state_mut().open_path(&a);
    h.state_mut().open_path(&b);
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::W);
    assert_eq!(h.state().file_name(), "a.txt");
    key(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::T);
    assert_eq!(h.state().file_name(), "b.txt");
    // A middle click on a tab closes it.
    h.get_by_label("a.txt").click_button(PointerButton::Middle);
    h.run();
    assert!(h.query_by_label("a.txt").is_none());
    key(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::T);
    assert_eq!(h.state().file_name(), "a.txt");
}

#[test]
fn paste_from_history_offers_earlier_copies() {
    let mut h = harness();
    h.event(Event::Paste("one two".into()));
    h.run();
    key(
        &mut h,
        Modifiers::COMMAND | Modifiers::SHIFT,
        Key::ArrowLeft,
    );
    h.event(Event::Copy);
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::Home);
    key(
        &mut h,
        Modifiers::COMMAND | Modifiers::SHIFT,
        Key::ArrowRight,
    );
    h.event(Event::Copy);
    h.run();
    key(&mut h, Modifiers::NONE, Key::End);
    submenu(&mut h, "Edit", "Paste from History");
    click(&mut h, "two");
    assert_eq!(text(&h), "one twotwo");
}

#[test]
fn multi_select_next_from_the_edit_menu() {
    let mut h = harness();
    h.event(Event::Paste("cat Cat cat".into()));
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::Home);
    for _ in 0..3 {
        submenu(&mut h, "Edit", "Multi-select Next");
        click(&mut h, "Match Case & Whole Word");
    }
    h.event(Event::Text("dog".into()));
    h.run();
    assert_eq!(text(&h), "dog Cat dog");
}

#[test]
fn a_read_only_document_refuses_edits() {
    let mut h = harness();
    h.event(Event::Paste("keep".into()));
    h.run();
    click(&mut h, "Edit");
    click(&mut h, "Read-Only");
    assert!(h.state().buffer().read_only, "the menu item set it");
    key(&mut h, Modifiers::NONE, Key::Escape);
    h.event(Event::Text("x".into()));
    h.run();
    assert_eq!(text(&h), "keep");
    h.get_by_label("new 1 * (read-only)");
}

#[test]
fn view_summary_counts_the_document() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, "one two\r\nthree, four five.\r\n").unwrap();
    let mut h = harness();
    h.state_mut().open_path(&file);
    h.run();
    click(&mut h, "View");
    click(&mut h, "Summary…");
    h.get_by_label("Summary");
    // 7 + 17 characters on two lines and an empty third one; the length
    // with the file's CR LF line endings.
    for row in [
        "Characters (without line endings)",
        "Words",
        "Lines",
        "Modified",
    ] {
        h.get_by_label(row);
    }
    for value in ["24", "5", "3"] {
        h.get_by_label(value);
    }
    // The document's length and the file's size.
    assert_eq!(h.get_all_by_label("28 bytes").count(), 2);
    click(&mut h, "Close");
    assert!(h.query_by_label("Summary").is_none());
}

#[test]
fn open_file_from_the_right_click_menu_and_copy_results() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.rs"), "one\ntwo\nthree\n").unwrap();
    let log = dir.path().join("build.log");
    std::fs::write(&log, "error at lib.rs:3:2 here\n").unwrap();
    let mut h = harness();
    h.state_mut().open_path(&log);
    h.run();
    // Right-click on "lib.rs:3:2" (the 11th column).
    let rect = h.get_by_label("Editor: build.log").rect();
    let font = egui::FontId::monospace(14.0);
    let (cell_w, line_h) = h
        .ctx
        .fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
    let pos = egui::Pos2::new(
        rect.min.x + 5.0 * cell_w + 8.0 + 11.5 * cell_w,
        rect.min.y + 0.5 * line_h,
    );
    h.event(Event::PointerMoved(pos));
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    h.run();
    click(&mut h, "Open lib.rs at Line 3");
    assert_eq!(h.state().file_name(), "lib.rs");
    h.get_by_label("Ln 3, Col 2");

    // Search Results > Copy writes the results out as grep does.
    h.state_mut().find.open = true;
    h.state_mut().find.pattern = "t".into();
    h.state_mut().find_all(false);
    for _ in 0..50 {
        h.step();
        if !h.state().results.searches.iter().any(|s| !s.done) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    h.run();
    assert_eq!(
        h.state().results.as_text(),
        "lib.rs:2: two\nlib.rs:3: three\n"
    );
}

#[test]
fn the_right_click_menu_opens_a_web_link() {
    let mut h = harness();
    let view = h.state().active_view();
    h.state_mut()
        .editor
        .apply(
            view,
            ochrepad_core::Action::InsertText("see https://example.com/x here".into()),
        )
        .unwrap();
    h.run();
    // Right-click inside the link (the 9th column).
    let rect = h.get_by_label("Editor: new 1").rect();
    let font = egui::FontId::monospace(14.0);
    let (cell_w, line_h) = h
        .ctx
        .fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
    let pos = egui::Pos2::new(
        rect.min.x + 5.0 * cell_w + 8.0 + 8.5 * cell_w,
        rect.min.y + 0.5 * line_h,
    );
    h.event(Event::PointerMoved(pos));
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    h.run();
    h.get_by_label("Open Link").click();
    h.step();
    let opened: Vec<String> = h
        .output()
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::OpenUrl(u) => Some(u.url.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(opened, ["https://example.com/x"]);
}

#[test]
fn begin_end_select_selects_from_where_it_began() {
    let mut h = harness();
    let view = h.state().active_view();
    let apply = |h: &mut Harness<'_, OchrepadApp>, a: ochrepad_core::Action| {
        h.state_mut().editor.apply(view, a).unwrap();
        h.run();
    };
    let caret = |h: &mut Harness<'_, OchrepadApp>, at: usize| {
        let at = ochrepad_core::ByteOffset(at);
        h.state_mut()
            .editor
            .apply(
                view,
                ochrepad_core::Action::SetSelections(ochrepad_core::SelectionSet::caret(at)),
            )
            .unwrap();
        h.run();
    };
    apply(
        &mut h,
        ochrepad_core::Action::InsertText("hello world\nabcdefgh".into()),
    );
    caret(&mut h, 2);
    click(&mut h, "Edit");
    click(&mut h, "Begin/End Select");
    let begun = |h: &Harness<'_, OchrepadApp>| {
        let app = h.state();
        app.editor
            .view(app.active_view())
            .unwrap()
            .begin_select
            .is_some()
    };
    assert!(begun(&h));
    // Moving the caret, then ending it.
    caret(&mut h, 8);
    click(&mut h, "Edit");
    click(&mut h, "Begin/End Select");
    assert_eq!(selected(&h), "llo wo");
    assert!(!begun(&h));

    // In column mode: a rectangle from the first corner (line 1, column 1)
    // to the caret (line 2, column 3).
    caret(&mut h, 1);
    click(&mut h, "Edit");
    click(&mut h, "Begin/End Select in Column Mode");
    caret(&mut h, 15);
    click(&mut h, "Edit");
    click(&mut h, "Begin/End Select in Column Mode");
    let app = h.state();
    let v = app.editor.view(app.active_view()).unwrap();
    assert!(v.selections.rect().is_some());
    let doc = app.buffer().doc();
    let parts: Vec<String> = v
        .selections
        .ranges()
        .iter()
        .map(|s| doc.slice(s.range()))
        .collect();
    assert_eq!(parts, ["el", "bc"]);
}

/// A file that opens in the viewer (or not at all) takes no line: the
/// document clicked in keeps its caret.
#[test]
fn open_file_in_the_viewer_leaves_the_clicked_document_alone() {
    let dir = tempfile::tempdir().unwrap();
    let big: String = (0..40_000).map(|i| format!("line {i:>40}\n")).collect();
    std::fs::write(dir.path().join("big.log"), &big).unwrap();
    let log = dir.path().join("build.log");
    std::fs::write(&log, "error at big.log:3:2 here\na\nb\nc\nd\n").unwrap();
    let mut h = harness();
    h.state_mut().settings.viewer_threshold_mb = 1;
    h.state_mut().open_path(&log);
    h.run();
    let view = h.state().active_view();
    let rect = h.get_by_label("Editor: build.log").rect();
    let font = egui::FontId::monospace(14.0);
    let (cell_w, line_h) = h
        .ctx
        .fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
    let pos = egui::Pos2::new(
        rect.min.x + 5.0 * cell_w + 8.0 + 11.5 * cell_w,
        rect.min.y + 0.5 * line_h,
    );
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
    click(&mut h, "Open big.log at Line 3");
    let app = h.state();
    let v = app.editor.view(view).unwrap();
    let doc = app.editor.buffer(v.buffer()).unwrap().doc();
    assert_eq!(doc.line_of(v.selections.primary().head), 0);
}
