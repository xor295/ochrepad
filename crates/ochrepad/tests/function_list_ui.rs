//! View > Function List: the panel of what the document defines.

mod util;

use std::time::Duration;

use egui::Vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;

const CODE: &str = "struct Server {\n    port: u16,\n}\n\nimpl Server {\n    fn start(&self) {}\n\n    fn stop(&self) {}\n}\n\nfn main() {\n    let s = Server { port: 80 };\n    s.start();\n}\n";

fn harness() -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(900.0, 400.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.run();
    h
}

/// Steps in real time until the list is read (it is, on a thread).
fn wait_for_list(h: &mut Harness<'_, OchrepadApp>, len: usize) {
    for _ in 0..500 {
        h.step();
        let b = h.state().buffer_id();
        if h.state()
            .function_list
            .entries(b)
            .is_some_and(|e| e.len() == len)
        {
            h.run();
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out");
}

fn open(h: &mut Harness<'_, OchrepadApp>, dir: &tempfile::TempDir) {
    let file = dir.path().join("server.rs");
    std::fs::write(&file, CODE).unwrap();
    h.state_mut().settings.show_function_list = true;
    h.state_mut().open_path(&file);
    h.run();
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

#[test]
fn the_list_goes_to_what_is_clicked_and_filters() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness();
    open(&mut h, &dir);
    wait_for_list(&mut h, 5);
    let b = h.state().buffer_id();
    let names: Vec<String> = h
        .state()
        .function_list
        .entries(b)
        .unwrap()
        .iter()
        .map(|e| e.symbol.name.clone())
        .collect();
    assert_eq!(names, ["Server", "impl Server", "start", "stop", "main"]);
    h.get_by_label("stop").click();
    h.run();
    assert_eq!(caret_line(&h), 7);
    // The filter keeps what matches.
    h.state_mut().function_list.filter = "st".into();
    h.run();
    assert!(h.query_by_label("main").is_none());
    h.get_by_label("start");
    // Edits are read again after a pause.
    h.state_mut().function_list.filter.clear();
    let view = h.state().active_view();
    h.state_mut()
        .editor
        .apply(
            view,
            ochrepad_core::Action::InsertText("fn helper() {}\n".into()),
        )
        .unwrap();
    wait_for_list(&mut h, 6);
    // × closes the panel.
    h.get_by_label("×").click();
    h.run();
    assert!(!h.state().settings.show_function_list);
}

#[test]
fn the_function_list_looks_right() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness();
    open(&mut h, &dir);
    wait_for_list(&mut h, 5);
    let view = h.state().active_view();
    h.state_mut()
        .editor
        .apply(view, ochrepad_core::Action::GoToLine { line: 6, column: 5 })
        .unwrap();
    h.run();
    util::snapshot(&mut h, "function_list");
}
