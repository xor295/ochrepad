//! The document map: clicking and dragging it scrolls the editor, and it
//! hides when the pane is narrow or the View menu turns it off.

mod util;

use egui::{Event, Modifiers, PointerButton, Pos2, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;

/// A Rust-like document of `n` short functions (5 lines each).
fn code(n: usize) -> String {
    (0..n)
        .map(|i| {
            format!(
                "fn item_{i}(x: u32) -> u32 {{\n    let y = x * {i};\n    // add one\n    y + 1\n}}\n"
            )
        })
        .collect()
}

fn harness(text: &str, size: Vec2) -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(size)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .wgpu()
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.event(Event::Paste(text.to_owned()));
    h.run();
    // Back to the top, as when a file is opened.
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::Home);
    h.run();
    h
}

fn first_line(h: &Harness<'_, OchrepadApp>) -> usize {
    let app = h.state();
    app.editor
        .view(app.active_view())
        .unwrap()
        .scroll
        .first_line
}

/// The map strip: 100 px left of the 12 px scrollbar, from the editor's top.
fn strip(h: &Harness<'_, OchrepadApp>) -> egui::Rect {
    let rect = h.get_by_label("Editor: new 1").rect();
    egui::Rect::from_min_max(
        Pos2::new(rect.max.x - 12.0 - 100.0, rect.min.y),
        Pos2::new(rect.max.x - 12.0, rect.max.y - 12.0),
    )
}

fn press(h: &mut Harness<'_, OchrepadApp>, pos: Pos2, pressed: bool) {
    h.event(Event::PointerMoved(pos));
    h.event(Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    });
    h.step();
}

#[test]
fn clicking_the_map_centres_the_editor_there() {
    let mut h = harness(&code(40), Vec2::new(800.0, 600.0));
    let s = strip(&h);
    // 200 lines at 2 px each fit the strip, so the map does not scroll: the
    // row at y is line (y - top) / 2.
    let p = Pos2::new(s.center().x, s.min.y + 300.0);
    press(&mut h, p, true);
    press(&mut h, p, false);
    h.run();
    let first = first_line(&h);
    assert!((120..=150).contains(&first), "first line {first}");
}

#[test]
fn dragging_the_marker_scrolls_like_a_scrollbar() {
    let mut h = harness(&code(400), Vec2::new(800.0, 400.0));
    let s = strip(&h);
    let grab = Pos2::new(s.center().x, s.min.y + 4.0);
    press(&mut h, grab, true);
    for y in [40.0, 120.0, 400.0] {
        h.event(Event::PointerMoved(Pos2::new(grab.x, s.min.y + y)));
        h.step();
    }
    press(&mut h, Pos2::new(grab.x, s.min.y + 400.0), false);
    h.run();
    // Dragged to the bottom of the strip: the end of the document.
    assert!(first_line(&h) > 1900, "first line {}", first_line(&h));
    // A selection was not started by the drag.
    let app = h.state();
    assert!(
        app.editor
            .view(app.active_view())
            .unwrap()
            .selections
            .primary()
            .is_empty()
    );
}

#[test]
fn the_map_hides_when_narrow_or_turned_off() {
    // At 500 px, 40 columns of text would not fit beside it: clicking where
    // it would be places the caret instead of scrolling.
    let mut h = harness(&code(40), Vec2::new(500.0, 400.0));
    let s = strip(&h);
    let p = Pos2::new(s.center().x, s.min.y + 300.0);
    press(&mut h, p, true);
    press(&mut h, p, false);
    h.run();
    assert_eq!(first_line(&h), 0);

    // Tall enough for the whole View menu.
    let mut h = harness(&code(40), Vec2::new(800.0, 720.0));
    h.get_by_label("View").click();
    h.run();
    h.get_by_label("Document Map").click();
    h.run();
    assert!(!h.state().settings.document_map);
}

#[test]
fn the_map_looks_right() {
    let mut h = harness(&code(60), Vec2::new(800.0, 400.0));
    util::snapshot(&mut h, "doc_map");
}
