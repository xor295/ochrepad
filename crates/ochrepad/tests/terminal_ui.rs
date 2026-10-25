//! Terminal tabs in the app: View > Terminal and Ctrl+`, the keys a
//! terminal takes from the app, closing. A real shell runs (`cmd.exe` on
//! Windows, `/bin/sh` on Linux).

use std::time::Duration;

use egui::{Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;
use ochrepad::terminal::TerminalId;

fn harness() -> Harness<'static, OchrepadApp> {
    let mut app = OchrepadApp::new();
    if cfg!(windows) {
        app.settings.terminal_shell = "cmd.exe".to_owned();
        app.settings.terminal_args = "/Q".to_owned();
    } else {
        app.settings.terminal_shell = "/bin/sh".to_owned();
    }
    let mut h = Harness::builder()
        .with_size(Vec2::new(900.0, 600.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), app);
    h.run();
    h
}

fn key(h: &mut Harness<'_, OchrepadApp>, m: Modifiers, k: Key) {
    h.key_press_modifiers(m, k);
    h.run();
}

/// Steps in real time until `done` holds (the shell answers on its own
/// thread), for at most 20 seconds.
fn wait_for(h: &mut Harness<'_, OchrepadApp>, done: impl Fn(&OchrepadApp) -> bool) -> bool {
    for _ in 0..1000 {
        h.step();
        if done(h.state()) {
            h.run();
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn screen(app: &OchrepadApp, id: TerminalId) -> Vec<String> {
    app.terminals
        .get(id)
        .map(|t| t.screen_text())
        .unwrap_or_default()
}

fn shows(app: &OchrepadApp, id: TerminalId, line: &str) -> bool {
    screen(app, id).iter().any(|l| l == line)
}

fn only_terminal(h: &Harness<'_, OchrepadApp>) -> TerminalId {
    let ids: Vec<_> = h.state().terminals.iter().map(|t| t.id).collect();
    assert_eq!(ids.len(), 1, "one terminal");
    ids[0]
}

#[test]
fn ctrl_backtick_opens_a_terminal_below_the_editor_that_runs_commands() {
    let mut h = harness();
    key(&mut h, Modifiers::COMMAND, Key::Backtick);
    let id = only_terminal(&h);
    assert!(h.state().terminal_is_below_editors(id));
    assert_eq!(h.state().focused_terminal(), Some(id));
    h.get_by_label_contains("Terminal: ");

    h.event(Event::Text("echo typed-in-ochrepad".into()));
    key(&mut h, Modifiers::NONE, Key::Enter);
    assert!(
        wait_for(&mut h, |a| shows(a, id, "typed-in-ochrepad")),
        "screen:\n{}",
        screen(h.state(), id).join("\n")
    );

    // Keys the editor would take go to the shell instead.
    let tabs = h.state().tabs().len();
    key(&mut h, Modifiers::COMMAND, Key::W);
    key(&mut h, Modifiers::COMMAND, Key::N);
    assert_eq!(h.state().tabs().len(), tabs, "no tab closed or opened");
    assert!(h.state().terminals.get(id).is_some());
    assert_eq!(h.state().focused_terminal(), Some(id));

    // Ctrl+` goes back to the editor, and again to the same terminal.
    key(&mut h, Modifiers::COMMAND, Key::Backtick);
    assert_eq!(h.state().focused_terminal(), None);
    key(&mut h, Modifiers::COMMAND, Key::Backtick);
    assert_eq!(h.state().focused_terminal(), Some(id));
    assert_eq!(h.state().terminals.iter().count(), 1);
}

#[test]
fn a_second_terminal_joins_the_first_and_closing_ends_them() {
    let mut h = harness();
    h.get_by_label("View").click();
    h.run();
    h.get_by_label_contains("New Terminal").click();
    h.run();
    let first = only_terminal(&h);
    key(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::Backtick);
    let ids: Vec<_> = h.state().terminals.iter().map(|t| t.id).collect();
    assert_eq!(ids.len(), 2);
    let second = ids[1];
    assert_eq!(h.state().focused_terminal(), Some(second));
    // Both in one pane, below the editor.
    assert!(h.state().terminal_is_below_editors(first));
    assert!(h.state().terminal_is_below_editors(second));

    h.state_mut().close_terminal(second);
    h.run();
    h.state_mut().close_terminal(first);
    h.run();
    assert!(h.state().terminals.is_empty());
    assert_eq!(h.state().focused_terminal(), None);
    // The editor has the keyboard again.
    h.event(Event::Text("x".into()));
    h.run();
    assert_eq!(h.state().buffer().doc().to_string(), "x");
}

#[test]
fn ctrl_c_copies_a_selection_or_interrupts() {
    let mut h = harness();
    key(&mut h, Modifiers::COMMAND, Key::Backtick);
    let id = only_terminal(&h);
    h.event(Event::Text("echo copy-me".into()));
    key(&mut h, Modifiers::NONE, Key::Enter);
    assert!(wait_for(&mut h, |a| shows(a, id, "copy-me")));

    // Select All from the code side, then Ctrl+C copies instead of
    // interrupting.
    {
        use alacritty_terminal::grid::Dimensions;
        use alacritty_terminal::index::{Column, Point, Side};
        use alacritty_terminal::selection::{Selection, SelectionType};
        let t = h.state().terminals.get(id).unwrap();
        let mut term = t.term().lock();
        let mut all = Selection::new(
            SelectionType::Lines,
            Point::new(term.topmost_line(), Column(0)),
            Side::Left,
        );
        all.update(
            Point::new(term.bottommost_line(), term.last_column()),
            Side::Right,
        );
        term.selection = Some(all);
    }
    h.event(Event::Copy);
    h.step();
    let copied = h
        .output()
        .platform_output
        .commands
        .iter()
        .any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t.contains("copy-me")));
    assert!(copied, "the selection was copied");
    let t = h.state().terminals.get(id).unwrap();
    assert!(t.term().lock().selection.is_none(), "and let go");
}
