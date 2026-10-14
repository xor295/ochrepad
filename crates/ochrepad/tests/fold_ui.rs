//! Code folding in the editor: the fold margin, the folded-line marker, the
//! shortcuts and the View menu.

mod util;

use egui::{Event, FontId, Key, Modifiers, PointerButton, Pos2, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;

const CODE: &str = "fn main() {\n    let a = 1;\n    if a > 0 {\n        go();\n    }\n}\ndone\n";

fn harness(text: &str) -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(500.0, 260.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .wgpu()
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.event(Event::Paste(text.to_owned()));
    h.run();
    h
}

fn folded(h: &Harness<'_, OchrepadApp>) -> Vec<usize> {
    let app = h.state();
    let v = app.editor.view(app.active_view()).unwrap();
    v.folds().headers().collect()
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

fn key(h: &mut Harness<'_, OchrepadApp>, m: Modifiers, k: Key) {
    h.key_press_modifiers(m, k);
    h.run();
}

/// (cell width, line height, editor rect min).
fn metrics(h: &Harness<'_, OchrepadApp>) -> (f32, f32, Pos2) {
    let rect = h.get_by_label("Editor: new 1").rect();
    let font = FontId::monospace(14.0);
    let (w, lh) = h
        .ctx
        .fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
    (w, lh, rect.min)
}

/// The middle of the fold margin on screen row `row`. The gutter is 3
/// digits, then the 2-cell fold margin, then 4 px.
fn chevron(h: &Harness<'_, OchrepadApp>, row: usize) -> Pos2 {
    let (w, lh, min) = metrics(h);
    Pos2::new(min.x + 4.0 * w + 4.0, min.y + (row as f32 + 0.5) * lh)
}

/// The middle of cell `col` on screen row `row` of the text.
fn cell(h: &Harness<'_, OchrepadApp>, row: usize, col: f32) -> Pos2 {
    let (w, lh, min) = metrics(h);
    Pos2::new(
        min.x + 5.0 * w + 8.0 + (col + 0.5) * w,
        min.y + (row as f32 + 0.5) * lh,
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

#[test]
fn the_fold_margin_and_the_marker_fold_and_unfold() {
    let mut h = harness(CODE);
    let p = chevron(&h, 2);
    click(&mut h, p);
    assert_eq!(folded(&h), vec![2], "the `if` block");
    // A click on a line that starts no block does nothing.
    let p = chevron(&h, 1);
    click(&mut h, p);
    assert_eq!(folded(&h), vec![2]);

    // Up and Down skip the hidden line.
    key(&mut h, Modifiers::COMMAND, Key::Home);
    key(&mut h, Modifiers::NONE, Key::ArrowDown);
    key(&mut h, Modifiers::NONE, Key::ArrowDown);
    key(&mut h, Modifiers::NONE, Key::ArrowDown);
    assert_eq!(caret_line(&h), 4);
    assert_eq!(folded(&h), vec![2]);

    // The marker after `    if a > 0 {` (14 cells) unfolds it.
    let p = cell(&h, 2, 16.0);
    click(&mut h, p);
    assert!(folded(&h).is_empty());
}

#[test]
fn shortcuts_fold_all_levels_and_unfold() {
    let mut h = harness(CODE);
    key(&mut h, Modifiers::ALT, Key::Num0);
    assert_eq!(folded(&h), vec![0, 2]);
    key(&mut h, Modifiers::ALT | Modifiers::SHIFT, Key::Num0);
    assert!(folded(&h).is_empty());
    key(&mut h, Modifiers::ALT, Key::Num2);
    assert_eq!(folded(&h), vec![2]);

    key(&mut h, Modifiers::ALT, Key::Num1);
    assert_eq!(folded(&h), vec![0, 2]);
    // With Shift held, the keyboard reports Alt+Shift+1 as "!"; the physical
    // key still matches the binding.
    h.event(Event::Key {
        key: Key::Exclamationmark,
        physical_key: Some(Key::Num1),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::ALT | Modifiers::SHIFT,
    });
    h.run();
    assert_eq!(folded(&h), vec![2], "level 1 unfolded");

    // Fold Current with the caret inside the `if` block.
    key(&mut h, Modifiers::ALT | Modifiers::SHIFT, Key::Num0);
    key(&mut h, Modifiers::COMMAND, Key::Home);
    for _ in 0..3 {
        key(&mut h, Modifiers::NONE, Key::ArrowDown);
    }
    key(&mut h, Modifiers::COMMAND | Modifiers::ALT, Key::F);
    assert_eq!(folded(&h), vec![2]);
    assert_eq!(caret_line(&h), 2, "the caret moved to the folded line");
}

#[test]
fn the_view_menu_folds_everything() {
    let mut h = harness(CODE);
    // Tall enough for the View menu.
    h.set_size(Vec2::new(500.0, 700.0));
    h.run();
    h.get_by_label("View").click();
    h.run();
    h.get_by_label_contains("Fold All").click();
    h.run();
    assert_eq!(folded(&h), vec![0, 2]);
}

#[test]
fn folded_code_looks_right() {
    let mut h = harness(CODE);
    let p = chevron(&h, 2);
    click(&mut h, p);
    // Hover the gutter so the open chevrons show too.
    h.event(Event::PointerMoved(chevron(&h, 6)));
    h.run();
    util::snapshot(&mut h, "fold");
}
