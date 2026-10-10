//! M6: column selection and multi-cursor through keyboard and mouse.

use egui::{Event, FontId, Key, Modifiers, PointerButton, Pos2, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;

fn with_text(text: &str) -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(800.0, 600.0))
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

fn sels(h: &Harness<'_, OchrepadApp>) -> Vec<(usize, usize)> {
    let app = h.state();
    app.editor
        .view(app.active_view())
        .unwrap()
        .selections
        .ranges()
        .iter()
        .map(|s| (s.anchor.0, s.head.0))
        .collect()
}

fn key(h: &mut Harness<'_, OchrepadApp>, m: Modifiers, k: Key) {
    h.key_press_modifiers(m, k);
    h.run();
}

fn typ(h: &mut Harness<'_, OchrepadApp>, s: &str) {
    h.event(Event::Text(s.to_owned()));
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

/// Screen position of (line, column) in the editor, on the column boundary.
fn at(h: &Harness<'_, OchrepadApp>, line: usize, col: f32) -> Pos2 {
    let rect = h.get_by_label("Editor: new 1").rect();
    let font = FontId::monospace(14.0);
    let (cell_w, line_h) = h
        .ctx
        .fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
    Pos2::new(
        rect.min.x + 4.0 * cell_w + 8.0 + col * cell_w,
        rect.min.y + (line as f32 + 0.5) * line_h,
    )
}

fn pointer(h: &mut Harness<'_, OchrepadApp>, pos: Pos2, pressed: bool, modifiers: Modifiers) {
    h.event(Event::PointerMoved(pos));
    h.event(Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers,
    });
}

#[test]
fn alt_shift_arrows_then_typing_edits_every_line() {
    let mut h = with_text("abcd\nefgh\nijkl");
    key(&mut h, Modifiers::COMMAND, Key::Home);
    key(&mut h, Modifiers::NONE, Key::ArrowRight);
    key(&mut h, Modifiers::ALT | Modifiers::SHIFT, Key::ArrowDown);
    key(&mut h, Modifiers::ALT | Modifiers::SHIFT, Key::ArrowDown);
    key(&mut h, Modifiers::ALT | Modifiers::SHIFT, Key::ArrowRight);
    h.get_by_label("Column selection, 3 lines");
    typ(&mut h, "_");
    assert_eq!(text(&h), "a_cd\ne_gh\ni_kl");
    key(&mut h, Modifiers::NONE, Key::Backspace);
    assert_eq!(text(&h), "acd\negh\nikl");
}

#[test]
fn alt_drag_selects_a_rectangle() {
    let mut h = with_text("abcdef\nab\nabcdef");
    let alt = Modifiers::ALT;
    h.event(Event::ModifiersChanged(alt));
    let (start, end) = (at(&h, 0, 1.0), at(&h, 2, 4.0));
    pointer(&mut h, start, true, alt);
    h.step();
    for i in 1..=4 {
        let t = i as f32 / 4.0;
        let p = start + (end - start) * t;
        h.event(Event::PointerMoved(p));
        h.step();
    }
    pointer(&mut h, end, false, alt);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run();
    // Columns 1..4 on each line; the short middle line stops at its end.
    assert_eq!(sels(&h), vec![(1, 4), (8, 9), (11, 14)]);
    h.get_by_label("Column selection, 3 lines");
}

#[test]
fn ctrl_click_adds_carets_and_escape_collapses() {
    let mut h = with_text("one\ntwo\nthree");
    for (line, col) in [(0, 1.0), (1, 2.0)] {
        let p = at(&h, line, col);
        h.event(Event::ModifiersChanged(Modifiers::COMMAND));
        pointer(&mut h, p, true, Modifiers::COMMAND);
        h.step();
        pointer(&mut h, p, false, Modifiers::COMMAND);
        h.event(Event::ModifiersChanged(Modifiers::NONE));
        h.run();
        h.run_steps(40); // not a double-click
    }
    // The pasted text left a caret at the end; two more were added.
    assert_eq!(sels(&h), vec![(1, 1), (6, 6), (13, 13)]);
    typ(&mut h, "|");
    assert_eq!(text(&h), "o|ne\ntw|o\nthree|");
    key(&mut h, Modifiers::NONE, Key::Escape);
    assert_eq!(sels(&h).len(), 1);
}

#[test]
fn rectangular_copy_pastes_back_as_a_block() {
    let mut h = with_text("abcd\nx\nabcd");
    key(&mut h, Modifiers::COMMAND, Key::Home);
    key(&mut h, Modifiers::NONE, Key::ArrowRight);
    key(&mut h, Modifiers::NONE, Key::ArrowRight);
    for _ in 0..2 {
        key(&mut h, Modifiers::ALT | Modifiers::SHIFT, Key::ArrowDown);
    }
    for _ in 0..2 {
        key(&mut h, Modifiers::ALT | Modifiers::SHIFT, Key::ArrowRight);
    }
    h.event(Event::Copy);
    h.step();
    // The short middle line contributes an empty line.
    assert_eq!(copied(&h), vec!["cd\n\ncd".to_owned()]);
    h.run();

    // Paste at line 0, column 1: one clip line per document line.
    key(&mut h, Modifiers::COMMAND, Key::Home);
    key(&mut h, Modifiers::NONE, Key::ArrowRight);
    h.event(Event::Paste("cd\n\ncd".into()));
    h.run();
    assert_eq!(text(&h), "acdbcd\nx\nacdbcd");
}

#[test]
fn ordinary_copy_pastes_normally() {
    let mut h = with_text("ab\ncd");
    key(&mut h, Modifiers::COMMAND, Key::A);
    h.event(Event::Copy);
    h.run();
    key(&mut h, Modifiers::COMMAND, Key::Home);
    key(&mut h, Modifiers::NONE, Key::ArrowRight);
    h.event(Event::Paste("ab\ncd".into()));
    h.run();
    assert_eq!(text(&h), "aab\ncdb\ncd");
}
