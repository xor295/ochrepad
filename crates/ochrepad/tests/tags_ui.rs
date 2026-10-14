//! Matching tags: in markup, the names in the start and end tag of the
//! element at the caret are outlined.

use std::ops::Range;

use egui::{Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use ochrepad::OchrepadApp;
use ochrepad_core::{Action, ByteOffset, Selection, SelectionSet};

const PAGE: &str = "<!DOCTYPE html>\n<html>\n<body>\n  <div class=\"box\">\n    <p>Hello <b>world</b></p>\n  </div>\n</body>\n</html>\n";

fn harness() -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(700.0, 400.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.run();
    h
}

fn open(h: &mut Harness<'_, OchrepadApp>, dir: &tempfile::TempDir, name: &str, text: &str) {
    let file = dir.path().join(name);
    std::fs::write(&file, text).unwrap();
    h.state_mut().open_path(&file);
    h.run();
}

fn select(h: &mut Harness<'_, OchrepadApp>, sels: SelectionSet) {
    let view = h.state().active_view();
    h.state_mut()
        .editor
        .apply(view, Action::SetSelections(sels))
        .unwrap();
    h.run();
}

fn caret(h: &mut Harness<'_, OchrepadApp>, at: usize) {
    select(h, SelectionSet::caret(ByteOffset(at)));
}

/// The byte range of `name` where `context` (which holds it) first occurs
/// in `text`.
fn name_at(text: &str, context: &str, name: &str) -> Range<usize> {
    let start = text.find(context).unwrap() + context.find(name).unwrap();
    start..start + name.len()
}

/// The names outlined, as byte ranges.
fn outlined(h: &Harness<'_, OchrepadApp>) -> Option<(Range<usize>, Range<usize>)> {
    let pair = h.state().view_state()?.matched_tags()?;
    Some((
        pair.open.start.0..pair.open.end.0,
        pair.close.start.0..pair.close.end.0,
    ))
}

#[test]
fn the_tags_of_the_element_at_the_caret_are_outlined() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness();
    open(&mut h, &dir, "page.html", PAGE);
    assert_eq!(h.state().buffer().language.as_deref(), Some("HTML"));

    // In the start tag (its name or attributes) or the end tag.
    let div = (name_at(PAGE, "<div", "div"), name_at(PAGE, "</div>", "div"));
    for at in [
        PAGE.find("<div").unwrap() + 2,
        PAGE.find("class").unwrap(),
        PAGE.find("</div>").unwrap() + 3,
    ] {
        caret(&mut h, at);
        assert_eq!(outlined(&h), Some(div.clone()));
    }
    // Between tags: the innermost element around the caret.
    caret(&mut h, PAGE.find("world").unwrap() + 2);
    let b = (name_at(PAGE, "<b>", "b"), name_at(PAGE, "</b>", "b"));
    assert_eq!(outlined(&h), Some(b));
    caret(&mut h, PAGE.find("Hello").unwrap() + 2);
    let p = (name_at(PAGE, "<p>", "p"), name_at(PAGE, "</p>", "p"));
    assert_eq!(outlined(&h), Some(p.clone()));
    caret(&mut h, PAGE.find("<!DOCTYPE").unwrap() + 3);
    assert_eq!(outlined(&h), None);

    // Not with a selection or more than one caret.
    let at = PAGE.find("Hello").unwrap();
    select(
        &mut h,
        SelectionSet::single(Selection::new(ByteOffset(at), ByteOffset(at + 5))),
    );
    assert_eq!(outlined(&h), None);
    select(
        &mut h,
        SelectionSet::new(
            vec![
                Selection::caret(ByteOffset(at + 2)),
                Selection::caret(ByteOffset(PAGE.find("world").unwrap())),
            ],
            0,
        ),
    );
    assert_eq!(outlined(&h), None);
    caret(&mut h, at + 2);
    assert_eq!(outlined(&h), Some(p));
}

#[test]
fn editing_a_tag_finds_its_partner_again() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness();
    open(&mut h, &dir, "page.html", PAGE);
    // The caret after the `b` of `<b>`: typing makes it `<bi>`, which has
    // no end tag.
    caret(&mut h, PAGE.find("<b>").unwrap() + 2);
    assert!(outlined(&h).is_some());
    h.event(Event::Text("i".into()));
    h.run();
    assert_eq!(outlined(&h), None);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(h.state().buffer().doc().to_string(), PAGE);
    assert_eq!(
        outlined(&h),
        Some((name_at(PAGE, "<b>", "b"), name_at(PAGE, "</b>", "b")))
    );
}

#[test]
fn xml_names_match_in_their_case_and_plain_text_has_none() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness();
    let text = "<?xml version=\"1.0\"?>\n<list>\n  <Item>one</item>\n</list>\n";
    open(&mut h, &dir, "list.xml", text);
    assert_eq!(h.state().buffer().language.as_deref(), Some("XML"));
    // `<Item>` has no end tag in XML, so in it nothing is outlined, and in
    // its text the list is.
    caret(&mut h, text.find("Item").unwrap() + 1);
    assert_eq!(outlined(&h), None);
    caret(&mut h, text.find("one").unwrap() + 1);
    let list = (
        name_at(text, "<list", "list"),
        name_at(text, "</list", "list"),
    );
    assert_eq!(outlined(&h), Some(list));

    // As HTML, the names match in any case.
    h.state_mut().set_language("HTML");
    h.run();
    let item = (
        name_at(text, "<Item", "Item"),
        name_at(text, "</item", "item"),
    );
    assert_eq!(outlined(&h), Some(item));
    h.state_mut().set_language("Plain Text");
    h.run();
    assert_eq!(outlined(&h), None);
}
