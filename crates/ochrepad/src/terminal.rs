//! Terminal tabs (PLAN_terminal): a shell on a pseudo-terminal, emulated by
//! `alacritty_terminal`. Its I/O thread reads the program's output into the
//! `Term` (behind a `FairMutex`) and writes our input; the widget
//! (`terminal_view.rs`) locks the `Term` once a frame to draw it.
//!
//! A terminal starts its shell on the first frame it is drawn, when the
//! grid's size is known. Dropping it shuts the I/O thread down, which hangs
//! up the shell (Linux) or closes the pseudo console (Windows), the way
//! closing a terminal window does.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use alacritty_terminal::event::{Event as TermEvent, EventListener, Notify, OnResize, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, Msg, Notifier};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Processor, Rgb};

/// Identifies a terminal tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TerminalId(pub u64);

/// A program and its arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellCommand {
    pub program: String,
    pub args: Vec<String>,
}

impl ShellCommand {
    /// The shell to start: `program` (with whitespace-separated `args`) when
    /// set, else the platform's default.
    pub fn new(program: &str, args: &str) -> Self {
        let program = program.trim();
        if program.is_empty() {
            return default_shell();
        }
        Self {
            program: program.to_owned(),
            args: args.split_whitespace().map(str::to_owned).collect(),
        }
    }

    /// The program's file name without `.exe`, for titles.
    pub fn name(&self) -> String {
        let file = Path::new(&self.program).file_name().map_or_else(
            || self.program.clone(),
            |n| n.to_string_lossy().into_owned(),
        );
        match file.rsplit_once('.') {
            Some((stem, ext)) if ext.eq_ignore_ascii_case("exe") => stem.to_owned(),
            _ => file,
        }
    }
}

/// Windows: PowerShell 7 when it is on `PATH`, else Windows PowerShell.
#[cfg(windows)]
pub fn default_shell() -> ShellCommand {
    let on_path = |exe: &str| {
        std::env::var_os("PATH")
            .is_some_and(|p| std::env::split_paths(&p).any(|dir| dir.join(exe).is_file()))
    };
    let program = if on_path("pwsh.exe") {
        "pwsh.exe"
    } else {
        "powershell.exe"
    };
    ShellCommand {
        program: program.to_owned(),
        args: Vec::new(),
    }
}

/// Linux: `$SHELL`, else `/bin/sh`.
#[cfg(not(windows))]
pub fn default_shell() -> ShellCommand {
    let program = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "/bin/sh".to_owned());
    ShellCommand {
        program,
        args: Vec::new(),
    }
}

/// The grid in cells, and a cell in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSize {
    pub cols: u16,
    pub rows: u16,
    pub cell_w: u16,
    pub cell_h: u16,
}

impl GridSize {
    fn window_size(self) -> WindowSize {
        WindowSize {
            num_lines: self.rows,
            num_cols: self.cols,
            cell_width: self.cell_w,
            cell_height: self.cell_h,
        }
    }

    fn term_size(self) -> TermSize {
        TermSize::new(usize::from(self.cols), usize::from(self.rows))
    }
}

/// Hands the I/O thread's events to the app and wakes it up.
#[derive(Clone)]
pub struct Listener {
    tx: Sender<TermEvent>,
    ctx: Option<egui::Context>,
}

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        let _ = self.tx.send(event);
        if let Some(ctx) = &self.ctx {
            ctx.request_repaint();
        }
    }
}

/// How the shell ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ended {
    /// It exited, with this code when the system gave one.
    Exited(Option<i32>),
    /// It could not be started.
    Failed(String),
}

/// One terminal: the emulator, the shell's I/O and what the tab shows.
pub struct Terminal {
    pub id: TerminalId,
    term: Arc<FairMutex<Term<Listener>>>,
    events: Receiver<TermEvent>,
    listener: Listener,
    /// Talks to the I/O thread; `None` before the shell starts and in tests.
    io: Option<Notifier>,
    /// For tests: parses bytes fed to a terminal with no shell.
    parser: Option<Processor>,
    pub shell: ShellCommand,
    /// The folder the shell starts in.
    pub cwd: PathBuf,
    scrollback: usize,
    size: GridSize,
    started: bool,
    /// The title the program set (OSC 0/2), if any.
    pub title: Option<String>,
    pub ended: Option<Ended>,
    /// The line saying the shell ended was written.
    announced: bool,
    /// The widget's state.
    pub view: crate::terminal_view::TerminalViewState,
}

impl Terminal {
    /// A terminal that will run `shell` in `cwd` once it is drawn.
    pub fn new(id: TerminalId, shell: ShellCommand, cwd: PathBuf, scrollback: usize) -> Self {
        let size = GridSize {
            cols: 80,
            rows: 24,
            cell_w: 8,
            cell_h: 16,
        };
        let (tx, events) = mpsc::channel();
        let listener = Listener { tx, ctx: None };
        let term = new_term(size, scrollback, listener.clone());
        Self {
            id,
            term,
            events,
            listener,
            io: None,
            parser: None,
            shell,
            cwd,
            scrollback,
            size,
            started: false,
            title: None,
            ended: None,
            announced: false,
            view: Default::default(),
        }
    }

    /// The emulator, for drawing and selecting. Hold the lock briefly: the
    /// I/O thread waits for it.
    pub fn term(&self) -> &Arc<FairMutex<Term<Listener>>> {
        &self.term
    }

    pub fn size(&self) -> GridSize {
        self.size
    }

    /// Whether the shell was started (or failed to start).
    pub fn started(&self) -> bool {
        self.started
    }

    /// Whether the shell is running.
    pub fn running(&self) -> bool {
        self.io.is_some() && self.ended.is_none()
    }

    /// The tab's title: the program's, else the shell's name; marked when
    /// the shell has ended.
    pub fn tab_title(&self) -> String {
        let name = self
            .title
            .as_deref()
            .filter(|t| !t.trim().is_empty())
            .map_or_else(|| self.shell.name(), short_title);
        if self.ended.is_some() {
            format!("{name} (ended)")
        } else {
            name
        }
    }

    /// Starts the shell with the grid at `size` (the first frame).
    pub fn start(&mut self, ctx: &egui::Context, size: GridSize) {
        if self.started {
            return;
        }
        self.started = true;
        self.listener.ctx = Some(ctx.clone());
        self.size = size;
        let (tx, events) = mpsc::channel();
        self.events = events;
        self.listener = Listener {
            tx,
            ctx: Some(ctx.clone()),
        };
        self.term = new_term(size, self.scrollback, self.listener.clone());
        match spawn(
            &self.shell,
            &self.cwd,
            size,
            self.id,
            self.term.clone(),
            self.listener.clone(),
        ) {
            Ok(notifier) => self.io = Some(notifier),
            Err(e) => {
                let why = format!("Could not start {}: {e}", self.shell.program);
                self.feed(format!("\x1b[31m{why}\x1b[0m\r\n").as_bytes());
                self.ended = Some(Ended::Failed(why));
            }
        }
    }

    /// Starts a new shell after the last one ended (the screen is cleared).
    pub fn restart(&mut self, ctx: &egui::Context) {
        self.shutdown();
        self.started = false;
        self.ended = None;
        self.announced = false;
        self.title = None;
        self.start(ctx, self.size);
    }

    /// Changes the grid's size, telling the program.
    pub fn resize(&mut self, size: GridSize) {
        if size == self.size {
            return;
        }
        self.size = size;
        self.term.lock().resize(size.term_size());
        if let Some(io) = &mut self.io {
            io.on_resize(size.window_size());
        }
    }

    /// Sends input to the program.
    pub fn write(&self, bytes: Vec<u8>) {
        if let Some(io) = &self.io
            && self.ended.is_none()
        {
            io.notify(bytes);
        }
    }

    /// The program's modes (cursor keys, mouse reports, bracketed paste…).
    pub fn mode(&self) -> TermMode {
        *self.term.lock().mode()
    }

    /// Handles what the I/O thread reported since the last frame: titles,
    /// clipboard stores, answers to queries, the shell's exit. `palette`
    /// answers colour queries.
    pub fn poll(&mut self, ctx: &egui::Context, palette: &dyn Fn(usize) -> Rgb) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                TermEvent::Title(t) => self.title = Some(t),
                TermEvent::ResetTitle => self.title = None,
                // A program copying (OSC 52); reading the clipboard is
                // refused (`ClipboardLoad` is ignored).
                TermEvent::ClipboardStore(_, text) => ctx.copy_text(text),
                TermEvent::PtyWrite(text) => self.write(text.into_bytes()),
                TermEvent::ColorRequest(index, format) => {
                    self.write(format(palette(index)).into_bytes());
                }
                TermEvent::TextAreaSizeRequest(format) => {
                    let size = self.size.window_size();
                    self.write(format(size).into_bytes());
                }
                TermEvent::ChildExit(status) => {
                    self.ended = Some(Ended::Exited(status.code()));
                }
                TermEvent::Exit if self.ended.is_none() => {
                    self.ended = Some(Ended::Exited(None));
                }
                _ => {}
            }
        }
        if let Some(Ended::Exited(code)) = self.ended
            && !self.announced
        {
            self.announced = true;
            let how = code.map_or_else(String::new, |c| format!(" with code {c}"));
            let line = format!(
                "\r\n\x1b[0;2m[The shell ended{how}. Press Enter to start a new one.]\x1b[0m"
            );
            self.feed(line.as_bytes());
        }
    }

    /// Stops the I/O thread, which ends the shell.
    fn shutdown(&mut self) {
        if let Some(io) = self.io.take() {
            let _ = io.0.send(Msg::Shutdown);
        }
    }

    /// Feeds bytes to the emulator as if the program had written them (tests,
    /// and the message when a shell cannot start).
    pub fn feed(&mut self, bytes: &[u8]) {
        let parser = self.parser.get_or_insert_with(Processor::new);
        parser.advance(&mut *self.term.lock(), bytes);
    }

    /// The text on screen (at the bottom of the scrollback), one string per
    /// row, without trailing blanks.
    pub fn screen_text(&self) -> Vec<String> {
        let term = self.term.lock();
        let grid = term.grid();
        (0..grid.screen_lines())
            .map(|row| {
                let line = &grid[Line(row as i32)];
                let mut s: String = (0..grid.columns()).map(|c| line[Column(c)].c).collect();
                s.truncate(s.trim_end().len());
                s
            })
            .collect()
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// A program's title, with a path to an executable (what Windows shells
/// set, `C:\WINDOWS\system32\cmd.exe`) cut to the program's name.
fn short_title(title: &str) -> String {
    let lower = title.to_ascii_lowercase();
    if lower.ends_with(".exe") && title.contains(['\\', '/']) {
        let file = title.rsplit(['\\', '/']).next().unwrap_or(title);
        return file[..file.len() - 4].to_owned();
    }
    title.to_owned()
}

fn new_term(
    size: GridSize,
    scrollback: usize,
    listener: Listener,
) -> Arc<FairMutex<Term<Listener>>> {
    let config = Config {
        scrolling_history: scrollback,
        ..Config::default()
    };
    Arc::new(FairMutex::new(Term::new(
        config,
        &size.term_size(),
        listener,
    )))
}

/// Starts `shell` on a new pseudo-terminal and its I/O thread.
fn spawn(
    shell: &ShellCommand,
    cwd: &Path,
    size: GridSize,
    id: TerminalId,
    term: Arc<FairMutex<Term<Listener>>>,
    listener: Listener,
) -> std::io::Result<Notifier> {
    let env = HashMap::from([
        ("TERM".to_owned(), "xterm-256color".to_owned()),
        ("COLORTERM".to_owned(), "truecolor".to_owned()),
        ("TERM_PROGRAM".to_owned(), "ochrepad".to_owned()),
        (
            "TERM_PROGRAM_VERSION".to_owned(),
            env!("CARGO_PKG_VERSION").to_owned(),
        ),
    ]);
    let options = tty::Options {
        shell: Some(tty::Shell::new(
            program_arg(&shell.program),
            shell.args.clone(),
        )),
        working_directory: cwd.is_dir().then(|| cwd.to_path_buf()),
        drain_on_exit: true,
        env,
        #[cfg(windows)]
        escape_args: true,
    };
    let pty = tty::new(&options, size.window_size(), id.0)?;
    let event_loop = EventLoop::new(term, listener, pty, options.drain_on_exit, false)?;
    let notifier = Notifier(event_loop.channel());
    // Not joined: the thread ends after `Msg::Shutdown` or the shell's exit,
    // and drops the pseudo-terminal itself.
    let _ = event_loop.spawn();
    Ok(notifier)
}

/// The program as the command line's first word: on Windows the command
/// line is built by joining, so a path with spaces needs quotes.
fn program_arg(program: &str) -> String {
    if cfg!(windows) && program.contains(char::is_whitespace) && !program.starts_with('"') {
        format!("\"{program}\"")
    } else {
        program.to_owned()
    }
}

/// The open terminals.
#[derive(Default)]
pub struct Terminals {
    terminals: Vec<Terminal>,
    next_id: u64,
    /// The terminal used last (Ctrl+` goes back to it).
    pub last: Option<TerminalId>,
}

impl Terminals {
    /// Adds a terminal that runs `shell` in `cwd`.
    pub fn add(&mut self, shell: ShellCommand, cwd: PathBuf, scrollback: usize) -> TerminalId {
        let id = TerminalId(self.next_id);
        self.next_id += 1;
        self.terminals
            .push(Terminal::new(id, shell, cwd, scrollback));
        id
    }

    pub fn get(&self, id: TerminalId) -> Option<&Terminal> {
        self.terminals.iter().find(|t| t.id == id)
    }

    pub fn get_mut(&mut self, id: TerminalId) -> Option<&mut Terminal> {
        self.terminals.iter_mut().find(|t| t.id == id)
    }

    /// Removes (and so ends) terminal `id`.
    pub fn remove(&mut self, id: TerminalId) {
        self.terminals.retain(|t| t.id != id);
        if self.last == Some(id) {
            self.last = None;
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Terminal> {
        self.terminals.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Terminal> {
        self.terminals.iter_mut()
    }

    pub fn is_empty(&self) -> bool {
        self.terminals.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_names_drop_folders_and_exe() {
        let s = ShellCommand::new(r"C:\Program Files\PowerShell\7\pwsh.exe", "-NoLogo");
        assert_eq!(s.args, ["-NoLogo"]);
        if cfg!(windows) {
            assert_eq!(s.name(), "pwsh");
            assert_eq!(
                program_arg(&s.program),
                r#""C:\Program Files\PowerShell\7\pwsh.exe""#
            );
        }
        assert_eq!(ShellCommand::new("/usr/bin/fish", "").name(), "fish");
        assert!(
            !ShellCommand::new("  ", "").program.is_empty(),
            "the default"
        );
    }

    #[test]
    fn titles_that_are_executables_are_shortened() {
        assert_eq!(short_title(r"C:\WINDOWS\SYSTEM32\cmd.exe"), "cmd");
        assert_eq!(
            short_title(r"C:\Program Files\PowerShell\7\pwsh.EXE"),
            "pwsh"
        );
        assert_eq!(short_title("✳ Claude Code"), "✳ Claude Code");
        assert_eq!(short_title("vim notes.exe"), "vim notes.exe");
        let mut t = Terminal::new(TerminalId(0), default_shell(), PathBuf::from("."), 100);
        t.title = Some(r"C:\WINDOWS\SYSTEM32\cmd.exe".to_owned());
        assert_eq!(t.tab_title(), "cmd");
        t.ended = Some(Ended::Exited(Some(0)));
        assert_eq!(t.tab_title(), "cmd (ended)");
    }

    #[test]
    fn fed_output_shows_on_screen() {
        let mut t = Terminal::new(TerminalId(0), default_shell(), PathBuf::from("."), 100);
        t.feed(b"hello\r\n\x1b[31mred\x1b[0m world");
        let screen = t.screen_text();
        assert_eq!(screen[0], "hello");
        assert_eq!(screen[1], "red world");
        assert_eq!(screen.len(), 24);
    }
}
