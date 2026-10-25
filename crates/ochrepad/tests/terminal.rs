//! The terminal's backend with a real shell: `cmd.exe` on Windows, `/bin/sh`
//! on Linux (fixed shells, so the tests do not depend on the user's).

use std::time::{Duration, Instant};

use alacritty_terminal::vte::ansi::Rgb;
use ochrepad::terminal::{Ended, GridSize, ShellCommand, Terminal, TerminalId};

fn shell() -> ShellCommand {
    if cfg!(windows) {
        ShellCommand::new("cmd.exe", "/Q")
    } else {
        ShellCommand::new("/bin/sh", "")
    }
}

const SIZE: GridSize = GridSize {
    cols: 80,
    rows: 24,
    cell_w: 8,
    cell_h: 16,
};

fn palette(_: usize) -> Rgb {
    Rgb { r: 0, g: 0, b: 0 }
}

/// Polls `t` until `done` holds, for at most 20 seconds.
fn wait(ctx: &egui::Context, t: &mut Terminal, done: impl Fn(&Terminal) -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(20) {
        t.poll(ctx, &palette);
        if done(t) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn a_shell_runs_a_command_and_exits() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut t = Terminal::new(TerminalId(1), shell(), dir.path().to_path_buf(), 1000);
    t.start(&ctx, SIZE);
    assert!(t.running());
    t.write(b"echo hello-from-ochrepad\r".to_vec());
    let shown = wait(&ctx, &mut t, |t| {
        // Typed before the first prompt, the output can share the
        // prompt's line (`$ hello-from-ochrepad`).
        t.screen_text().iter().any(|l| {
            l == "hello-from-ochrepad"
                || (l.ends_with(" hello-from-ochrepad") && !l.contains("echo"))
        })
    });
    assert!(shown, "screen:\n{}", t.screen_text().join("\n"));

    // The shell starts in the folder it was given.
    let pwd: &[u8] = if cfg!(windows) { b"cd\r" } else { b"pwd\r" };
    t.write(pwd.to_vec());
    let name = dir
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let in_dir = wait(&ctx, &mut t, |t| {
        t.screen_text()
            .iter()
            .any(|l| l.ends_with(&name) && !l.contains('>') && !l.contains('$'))
    });
    assert!(in_dir, "screen:\n{}", t.screen_text().join("\n"));

    t.write(b"exit\r".to_vec());
    let ended = wait(&ctx, &mut t, |t| t.ended.is_some());
    assert!(ended, "screen:\n{}", t.screen_text().join("\n"));
    assert!(matches!(t.ended, Some(Ended::Exited(_))));
    assert!(!t.running());
    assert!(
        t.screen_text()
            .iter()
            .any(|l| l.contains("Press Enter to start a new one")),
        "the tab says how to go on"
    );
}

#[test]
fn a_shell_that_cannot_start_says_why() {
    let ctx = egui::Context::default();
    let shell = ShellCommand::new("ochrepad-no-such-shell", "");
    let mut t = Terminal::new(TerminalId(2), shell, std::env::temp_dir(), 1000);
    t.start(&ctx, SIZE);
    assert!(matches!(t.ended, Some(Ended::Failed(_))));
    assert!(
        t.screen_text()
            .iter()
            .any(|l| l.contains("Could not start ochrepad-no-such-shell"))
    );
}

#[test]
fn resizing_reaches_the_program() {
    let ctx = egui::Context::default();
    let mut t = Terminal::new(TerminalId(3), shell(), std::env::temp_dir(), 1000);
    t.start(&ctx, SIZE);
    t.resize(GridSize { cols: 50, ..SIZE });
    let ask: &[u8] = if cfg!(windows) {
        b"mode con\r"
    } else {
        b"stty size\r"
    };
    t.write(ask.to_vec());
    let told = wait(&ctx, &mut t, |t| {
        t.screen_text().iter().any(|l| {
            let l = l.trim();
            // `stty size` prints "rows cols"; `mode con` a "Columns: 50" line.
            l == "24 50" || (l.starts_with("Columns:") && l.ends_with(" 50"))
        })
    });
    assert!(told, "screen:\n{}", t.screen_text().join("\n"));
}
