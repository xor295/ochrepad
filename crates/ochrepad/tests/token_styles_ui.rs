//! Search > Style All Occurrences of Token, Clear Style, Jump Up/Down.

mod util;

use egui::{Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;

const CODE: &str =
    "let total = price * qty;\nlet tax = total * rate;\nlet sum = total + tax;\nprint(sum, tax);\n";

fn harness() -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(760.0, 420.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.run();
    h.event(Event::Paste(CODE.into()));
    h.run();
    h
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

/// Puts the caret at byte `at`.
fn caret(h: &mut Harness<'_, OchrepadApp>, at: usize) {
    let view = h.state().active_view();
    h.state_mut()
        .editor
        .apply(
            view,
            ochrepad_core::Action::SetSelections(ochrepad_core::SelectionSet::caret(
                ochrepad_core::ByteOffset(at),
            )),
        )
        .unwrap();
    h.run();
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

fn style(h: &mut Harness<'_, OchrepadApp>, at: usize, name: &str) {
    caret(h, at);
    submenu(h, "Search", "Style All Occurrences of Token");
    click(h, &format!("Using {name} Style"));
}

#[test]
fn style_jump_and_clear() {
    let mut h = harness();
    // The caret in "total": the word is styled.
    style(&mut h, 6, "1st");
    let b = h.state().buffer_id();
    assert_eq!(h.state().token_styles.tokens(b), vec![("total", 0)]);
    // Jump Down goes from one to the next.
    caret(&mut h, 0);
    submenu(&mut h, "Search", "Jump Down");
    click(&mut h, "1st Style");
    assert_eq!(selection(&h), (4, 9));
    submenu(&mut h, "Search", "Jump Down");
    click(&mut h, "1st Style");
    assert_eq!(selection(&h), (35, 40));
    submenu(&mut h, "Search", "Jump Up");
    click(&mut h, "1st Style");
    assert_eq!(selection(&h), (4, 9));
    // Clear Style takes it away.
    submenu(&mut h, "Search", "Clear Style");
    click(&mut h, "Clear 1st Style");
    assert!(h.state().token_styles.tokens(b).is_empty());
}

#[test]
fn copy_styled_text_copies_each_occurrence() {
    let mut h = harness();
    // Tall enough for the whole Search menu.
    h.set_size(Vec2::new(760.0, 700.0));
    h.run();
    style(&mut h, 6, "1st");
    style(&mut h, 14, "2nd");
    submenu(&mut h, "Search", "Copy Styled Text");
    h.get_by_label("Using 1st Style").click();
    h.step();
    let copied = h
        .output()
        .platform_output
        .commands
        .iter()
        .find_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        });
    assert_eq!(copied.as_deref(), Some("total\ntotal\ntotal"));
    h.run();
    submenu(&mut h, "Search", "Copy Styled Text");
    h.get_by_label("Using All Styles").click();
    h.step();
    let copied = h
        .output()
        .platform_output
        .commands
        .iter()
        .find_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        });
    assert_eq!(copied.as_deref(), Some("total\nprice\ntotal\ntotal"));
}

#[test]
fn the_right_click_menu_styles_too() {
    let mut h = harness();
    // On "tax" (line 2, columns 5 to 7): the gutter's 3 digits and fold
    // margin, 4 px of padding, then the text.
    let rect = h.get_by_label("Editor: new 1").rect();
    let font = egui::FontId::monospace(14.0);
    let (cell_w, line_h) = h
        .ctx
        .fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
    let pos = egui::Pos2::new(
        rect.min.x + 5.0 * cell_w + 8.0 + 5.5 * cell_w,
        rect.min.y + 1.5 * line_h,
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
    h.get_by_label_contains("Style All Occurrences of Token")
        .hover();
    h.run();
    click(&mut h, "Using 3rd Style");
    let b = h.state().buffer_id();
    assert_eq!(h.state().token_styles.tokens(b), vec![("tax", 2)]);
}

#[test]
fn styled_tokens_look_right() {
    let mut h = harness();
    for (at, name) in [
        (6, "1st"),
        (14, "2nd"),
        (49, "3rd"),
        (31, "4th"),
        (20, "5th"),
    ] {
        style(&mut h, at, name);
    }
    h.key_press_modifiers(Modifiers::COMMAND, Key::End);
    h.run();
    util::snapshot(&mut h, "token_styles");
}
