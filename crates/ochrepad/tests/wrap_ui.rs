//! Word wrap in the editor: the View menu toggle, rows that follow the
//! pane's width, clicking on wrapped rows, Home/End and the look.

mod util;

use egui::{Event, FontId, Key, Modifiers, PointerButton, Pos2, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;
use ochrepad_core::{ByteOffset, DisplayMap, RowPos};

fn harness(text: &str, width: f32) -> Harness<'static, OchrepadApp> {
    let mut app = OchrepadApp::new();
    app.settings.word_wrap = true;
    let mut h = Harness::builder()
        .with_size(Vec2::new(width, 260.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .wgpu()
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), app);
    h.event(Event::Paste(text.to_owned()));
    h.run();
    h
}

/// Runs `f` with the active view's display rows.
fn with_rows<R>(h: &Harness<'_, OchrepadApp>, f: impl FnOnce(&DisplayMap) -> R) -> R {
    let app = h.state();
    let v = app.editor.view(app.active_view()).unwrap();
    let rows = DisplayMap::new(app.buffer(), v);
    f(&rows)
}

fn head(h: &Harness<'_, OchrepadApp>) -> ByteOffset {
    let app = h.state();
    app.editor
        .view(app.active_view())
        .unwrap()
        .selections
        .primary()
        .head
}

fn key(h: &mut Harness<'_, OchrepadApp>, m: Modifiers, k: Key) {
    h.key_press_modifiers(m, k);
    h.run();
}

/// A point `col` cells into the text of screen row `row`. The gutter is 3
/// digits, the 2-cell fold margin and 4 px; the text starts 4 px further in.
fn at(h: &Harness<'_, OchrepadApp>, row: usize, col: f32) -> Pos2 {
    let rect = h.get_by_label("Editor: new 1").rect();
    let font = FontId::monospace(14.0);
    let (w, lh) = h
        .ctx
        .fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
    Pos2::new(
        rect.min.x + 5.0 * w + 8.0 + col * w,
        rect.min.y + (row as f32 + 0.5) * lh,
    )
}

fn click(h: &mut Harness<'_, OchrepadApp>, pos: Pos2) {
    h.event(Event::PointerMoved(pos));
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    h.run();
}

const LONG: &str = "The quick brown fox jumps over the lazy dog and keeps on running across the field until the sun goes down.\nshort";

#[test]
fn the_view_menu_turns_wrap_on_and_off() {
    let mut h = harness(LONG, 500.0);
    h.state_mut().settings.word_wrap = false;
    // Tall enough for the whole View menu.
    h.set_size(Vec2::new(500.0, 600.0));
    h.run();
    assert!(with_rows(&h, |r| !r.wraps()));
    h.get_by_label("View").click();
    h.run();
    h.get_by_label("Word Wrap").click();
    h.run();
    assert!(h.state().settings.word_wrap);
    assert!(with_rows(&h, |r| r.rows_in_line(0) > 1));
}

#[test]
fn rows_follow_the_pane_width() {
    let mut h = harness(LONG, 500.0);
    let narrow = with_rows(&h, |r| r.rows_in_line(0));
    h.set_size(Vec2::new(900.0, 260.0));
    h.run();
    let wide = with_rows(&h, |r| r.rows_in_line(0));
    assert!(wide < narrow, "{wide} rows at 900 px, {narrow} at 500 px");
}

#[test]
fn clicks_and_home_end_work_on_wrapped_rows() {
    let mut h = harness(LONG, 500.0);
    // A click on the second screen row, two cells in.
    let p = at(&h, 1, 2.2);
    click(&mut h, p);
    let expected = with_rows(&h, |r| r.row_range(RowPos::new(0, 1)).start.0 + 2);
    assert_eq!(head(&h).0, expected);

    // End: the end of that row, then of the line.
    key(&mut h, Modifiers::NONE, Key::End);
    let row_end = with_rows(&h, |r| r.row_end(RowPos::new(0, 1)));
    assert_eq!(head(&h), row_end);
    key(&mut h, Modifiers::NONE, Key::End);
    assert_eq!(head(&h).0, LONG.find('\n').unwrap());

    // Home: the start of the last row, then of the line.
    key(&mut h, Modifiers::NONE, Key::Home);
    let last_start = with_rows(&h, |r| {
        let last = r.rows_in_line(0) - 1;
        r.row_range(RowPos::new(0, last)).start
    });
    assert_eq!(head(&h), last_start);
    key(&mut h, Modifiers::NONE, Key::Home);
    assert_eq!(head(&h).0, 0);

    // Down from the first row goes to the second row, not the next line.
    key(&mut h, Modifiers::NONE, Key::ArrowDown);
    assert_eq!(with_rows(&h, |r| r.row_of(head(&h))), RowPos::new(0, 1));
}

#[test]
fn wrapped_text_looks_right() {
    let text = "fn main() {\n    let message = \"a long string that will not fit on one row of this narrow window\";\n    println!(\"{message}\");\n}\n";
    let mut h = harness(text, 500.0);
    key(&mut h, Modifiers::COMMAND, Key::Home);
    key(&mut h, Modifiers::NONE, Key::ArrowDown);
    key(&mut h, Modifiers::SHIFT, Key::End);
    util::snapshot(&mut h, "wrap");
}
