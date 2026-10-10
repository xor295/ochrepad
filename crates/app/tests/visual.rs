//! Snapshot tests of the editor in fixed states (PLAN 6). References live in
//! tests/snapshots; regenerate with `UPDATE_SNAPSHOTS=1 cargo test -p ochrepad`.
//! They use egui's built-in fonts only, so they do not depend on which system
//! CJK font is installed.

use egui::{Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use ochrepad::OchrepadApp;
use ochrepad_core::{Action, ByteOffset, Selection, SelectionSet};

const SAMPLE: &str = "fn main() {\n\tlet greeting = \"héllo\";\n\tprintln!(\"{greeting}, wörld\");\n}\n// e\u{301} 👍🏽 👨‍👩‍👧 end\n";

fn harness() -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(520.0, 240.0))
        .with_step_dt(1.0 / 60.0)
        // Background highlighting runs on a wall-clock budget; let `run`
        // wait for it even on a loaded machine.
        .with_max_steps(10_000)
        .wgpu()
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.event(Event::Paste(SAMPLE.to_owned()));
    h.run();
    h
}

fn set(h: &mut Harness<'_, OchrepadApp>, ranges: &[(usize, usize)]) {
    let app = h.state_mut();
    let ranges = ranges
        .iter()
        .map(|&(a, b)| Selection::new(ByteOffset(a), ByteOffset(b)))
        .collect();
    let _ = app.editor.apply(
        app.active_view(),
        Action::SetSelections(SelectionSet::new(ranges, 0)),
    );
    h.run();
}

#[test]
fn snapshot_selection() {
    let mut h = harness();
    set(&mut h, &[(16, 44)]);
    h.snapshot("selection");
}

#[test]
fn snapshot_multi_cursor() {
    let mut h = harness();
    set(&mut h, &[(0, 0)]);
    for _ in 0..3 {
        h.key_press_modifiers(Modifiers::COMMAND | Modifiers::ALT, Key::ArrowDown);
        h.run();
    }
    h.snapshot("multi_cursor");
}

#[test]
fn snapshot_highlighting() {
    let mut h = harness();
    h.state_mut().set_language("Rust");
    h.run();
    set(&mut h, &[(0, 0)]);
    h.snapshot("highlighting");
}

#[test]
fn snapshot_split_view() {
    let mut h = harness();
    h.state_mut().set_language("Rust");
    h.run();
    set(&mut h, &[(0, 2)]);
    h.state_mut().clone_to_other_view();
    h.run();
    set(&mut h, &[(16, 24)]);
    h.snapshot("split_view");
}

/// Not a snapshot: renders with the system CJK font to a PNG for a human to
/// look at. `cargo test -p ochrepad --test visual -- --ignored`
#[test]
#[ignore]
fn render_with_system_fonts() {
    let mut h = Harness::builder()
        .with_size(Vec2::new(700.0, 260.0))
        .with_step_dt(1.0 / 60.0)
        // Background highlighting runs on a wall-clock budget; let `run`
        // wait for it even on a loaded machine.
        .with_max_steps(10_000)
        .wgpu()
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    let font = ochrepad::fonts::install(&h.ctx);
    println!("CJK font: {font:?}");
    h.event(Event::Paste(format!(
        "{SAMPLE}漢字かな交じり|after\n\tx漢x\n"
    )));
    h.run();
    set(&mut h, &[(0, 2), (60, 60), (100, 108)]);
    let image = h.render().expect("render");
    let out = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("system_fonts.png");
    image.save(&out).expect("save png");
    println!("wrote {}", out.display());
}
