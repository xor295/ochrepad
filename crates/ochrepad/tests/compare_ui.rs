//! M12: the compare view.

use egui::{Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;
use ochrepad::app::Tab;
use ochrepad::compare::CompareId;
use ochrepad_core::diff::RowKind;
use ochrepad_core::{Action, BufferId};

const LEFT: &str = "fn total(price: u32, qty: u32) -> u32 {\n    let sum = price * qty;\n    sum\n}\n\nfn main() {\n    println!(\"{}\", total(2, 3));\n}\n";
const RIGHT: &str = "fn total(price: u32, qty: u32) -> u32 {\n    let sum = price * qty + tax;\n    sum\n}\n\n// entry point\nfn main() {\n    println!(\"{}\", total(2, 3));\n}\n";

fn harness() -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(1100.0, 500.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.run();
    h
}

/// Two untitled buffers with `LEFT` and `RIGHT`, compared.
fn compared(h: &mut Harness<'_, OchrepadApp>) -> (BufferId, BufferId, CompareId) {
    h.event(Event::Paste(LEFT.into()));
    h.run();
    let left = h.state().buffer_id();
    h.key_press_modifiers(Modifiers::COMMAND, Key::N);
    h.run();
    h.event(Event::Paste(RIGHT.into()));
    h.run();
    let right = h.state().buffer_id();
    let id = h.state_mut().compare(left, right);
    h.run();
    (left, right, id)
}

fn kinds(h: &Harness<'_, OchrepadApp>, id: CompareId) -> Vec<RowKind> {
    let c = h.state().compares.get(id).unwrap();
    c.diff.rows.iter().map(|r| r.kind).collect()
}

#[test]
fn comparing_two_buffers_aligns_rows_and_summarises() {
    let mut h = harness();
    let (_, _, id) = compared(&mut h);
    h.get_by_label("Compare: new 1 vs new 2");
    h.get_by_label("1 added, 0 removed, 1 changed");
    let k = kinds(&h, id);
    assert_eq!(k[1], RowKind::Changed);
    assert_eq!(k[5], RowKind::Added);
    assert_eq!(k.iter().filter(|k| **k == RowKind::Equal).count(), 8);
    // The "added" row has a filler row opposite it on the left.
    let c = h.state().compares.get(id).unwrap();
    assert_eq!((c.diff.rows[5].left, c.diff.rows[5].right), (None, Some(5)));
}

#[test]
fn next_and_previous_change() {
    let mut h = harness();
    let (_, _, id) = compared(&mut h);
    h.get_by_label("Next change").click();
    h.run();
    assert_eq!(h.state().compares.get(id).unwrap().current, Some(0));
    h.get_by_label("Next change").click();
    h.run();
    assert_eq!(h.state().compares.get(id).unwrap().current, Some(1));
    // F8 / Shift+F8 work while the comparison has focus.
    h.key_press_modifiers(Modifiers::SHIFT, Key::F8);
    h.run();
    assert_eq!(h.state().compares.get(id).unwrap().current, Some(0));
    h.key_press(Key::F8);
    h.run();
    assert_eq!(h.state().compares.get(id).unwrap().current, Some(1));
}

#[test]
fn editing_a_side_updates_the_diff_after_a_pause() {
    let mut h = harness();
    let (_, right, id) = compared(&mut h);
    // Make the right side's "+ tax" line identical to the left again.
    let view = h
        .state()
        .editor
        .views_of(right)
        .next()
        .map(|v| v.id())
        .unwrap();
    {
        let app = h.state_mut();
        let q = ochrepad_core::SearchQuery::new(" + tax", Default::default());
        app.editor
            .apply(
                view,
                Action::ReplaceAll {
                    query: q,
                    replacement: String::new(),
                },
            )
            .unwrap();
    }
    h.run();
    assert_eq!(
        h.state().compares.get(id).unwrap().diff.stats.changed,
        1,
        "not yet"
    );
    h.run_steps(30);
    h.get_by_label("1 added, 0 removed, 0 changed");
}

#[test]
fn compare_with_the_saved_version() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("a.txt");
    std::fs::write(&file, "one\ntwo\nthree\n").unwrap();
    let mut h = harness();
    h.state_mut().open_path(&file);
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::End);
    h.event(Event::Text("four".into()));
    h.run();
    let id = h.state_mut().compare_with_saved().unwrap();
    h.run();
    h.get_by_label("Compare: a.txt (saved) vs a.txt");
    assert_eq!(kinds(&h, id).last(), Some(&RowKind::Changed));
}

#[test]
fn closing_a_compared_buffer_closes_the_comparison() {
    let mut h = harness();
    let (left, _, id) = compared(&mut h);
    let view = h
        .state()
        .editor
        .views_of(left)
        .next()
        .map(|v| v.id())
        .unwrap();
    h.state_mut().close_view(view);
    h.run();
    h.get_by_label("Don't Save").click();
    h.run();
    assert!(h.state().compares.get(id).is_none());
    assert!(h.state().dock.find_tab(&Tab::Compare(id)).is_none());
}

#[test]
fn snapshot_compare_view() {
    let mut h = Harness::builder()
        .with_size(Vec2::new(1100.0, 330.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .wgpu()
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.run();
    let (_, _, _) = compared(&mut h);
    h.get_by_label("Next change").click();
    h.run();
    h.snapshot("compare_view");
}
