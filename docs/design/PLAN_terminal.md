# Plan: a terminal in ochrepad

**Status: done** (2026-10-24). T1–T7 as planned. Beyond the plan: box-drawing
and block characters drawn as shapes, `grep -n` locations, and a 40 % first
pane (30 % was too small for Claude Code). `CLAUDE.md` is the current guide.

Written 2026-10-24, after 0.3.1. `CLAUDE.md` is the current guide; read its
architecture rules before starting. This plan adds a real terminal (a
pseudo-terminal and a terminal emulator) as a dock tab, so that `cargo build`,
a Python script or Claude Code runs next to the files it works on. Where it
changes a rule in `CLAUDE.md`, the step that makes the change updates it.

## 1. Goal and scope

1. **Terminal tabs:** a shell in a dock tab, docked below the editors by
   default, in the open folder (else the active file's folder). Several
   terminals; split and moved like any tab.
2. **Good enough for full-screen programs:** colours (256 and true colour),
   the alternate screen, scrollback, mouse reporting, bracketed paste,
   resizing with the pane, focus reporting. Claude Code, `vim`, `htop` and
   `less` must work.
3. **The editor side:** files changed by programs in the terminal reload
   without asking when they have no unsaved changes; `path:line:col` in the
   output opens the file at that place (Ctrl+click), and URLs open in the
   browser.
4. **Remembered:** the session keeps terminal tabs (their folders), not their
   processes.

### Not in this plan

- The kitty keyboard protocol (`alacritty_terminal` supports the modes, but
  ConPTY on Windows translates input itself; revisit if Claude Code needs it)
- Claude Code's IDE integration (selection sharing, diffs in the editor)
- A Run menu with per-language commands, tasks files read from a folder
  (a folder must never make ochrepad run anything)
- Shell integration (prompt marks, command decorations), split panes inside
  a terminal tab, search in the scrollback
- Ligatures, images (sixel, kitty graphics)

## 2. Decisions

| Question | Decision |
|---|---|
| Emulator and PTY | `alacritty_terminal =0.26.0`, no default features (no serde). It brings the VT parser (`vte`), the grid with scrollback, selection, and its own PTY layer: ConPTY on Windows, `openpty` on Linux, with an I/O thread per terminal (`EventLoop`). This is what Zed does. No `portable-pty`. `egui_term` (egui 0.31, unmaintained) is only a reference. |
| Where it lives | All in the app crate. `ochrepad-core` stays free of processes and egui; `fileio` stays about files. |
| Files changed on disk | New preference **Reload unmodified files silently**, on by default (Notepad++'s "Update silently"). A file with unsaved changes, or one that was deleted, still asks. |
| Default shell | Windows: `pwsh` when it is on `PATH`, else Windows PowerShell. Linux: `$SHELL`, else `/bin/sh`. Preferences can name another program and its arguments. |
| Closing | Closing a terminal tab, or exiting, ends its shell without asking, the way closing a terminal window does: Linux hangs up the shell (SIGHUP; the shell passes it to its jobs, and the kernel hangs up the foreground job); Windows closes the pseudo console, which ends the programs attached to it. Programs that left the terminal on purpose (`nohup`, `start`, GUI programs) keep running, as with any terminal. |
| Shortcuts | **Ctrl+\`** shows the terminal: it focuses the last used terminal, opens one when there is none, and from a terminal goes back to the editor. **Ctrl+Shift+\`** opens a new terminal. Both are in View > Terminal. |
| Keyboard ownership | While a terminal has the keyboard, it gets every key except Ctrl+\`, Ctrl+Shift+\`, Ctrl+Tab, Ctrl+Shift+Tab, Ctrl+PageUp/PageDown (switch tabs) and F11. So Ctrl+W, Ctrl+N, Ctrl+R and the rest reach the shell. Menus still work with the mouse. |
| Copy and paste | Ctrl+C copies when there is a selection, otherwise it sends an interrupt (Windows Terminal's rule). Ctrl+Shift+C always copies. Ctrl+V and Ctrl+Shift+V paste (bracketed when the program asks). Selecting with the mouse does not copy by itself. |
| Shift+Enter | Sends Alt+Enter (ESC CR), as Claude Code's `/terminal-setup` configures for VS Code, so Claude Code takes it as a new line. |
| Look | The editor's font family and zoomed size; the theme's editor background and text colour as the default colours; a 16-colour ANSI palette tuned to the warm dark and light themes; the caret in the accent colour. |
| Environment | `TERM=xterm-256color`, `COLORTERM=truecolor`, `TERM_PROGRAM=ochrepad`, passed to the shell only (not set on ochrepad's own process). |
| When the shell exits | The tab stays, with a line saying how it ended; Enter starts a new shell in the same folder. |

## 3. Design

### `terminal.rs`: one terminal

- `Terminal { term: Arc<FairMutex<Term<Listener>>>, notifier, size, cwd, title, exit: Option<ExitStatus>, events: mpsc::Receiver<Event> }`.
- `Listener` forwards `alacritty_terminal::event::Event` to the receiver and
  asks egui for a repaint. `poll` (each frame) handles `Title`,
  `ClipboardStore`, `PtyWrite` (answers to queries), `ChildExit`, `Bell`.
  `ClipboardLoad` (OSC 52 read) is refused.
- `Terminal::spawn(options, ctx)` resolves the shell, builds `tty::Options`
  (working directory, environment) and starts the `EventLoop`. Spawn errors
  surface in the tab and the status bar, never a panic.
- `resize(cols, rows, cell)` sends `Msg::Resize` and resizes the `Term`;
  `write(bytes)` sends input; `Drop` sends `Msg::Shutdown`, and the I/O thread
  drops the PTY (hang up, close) by itself, so a frame never waits.
- `Terminals`: id → `Terminal`, like `Viewers`; the last focused terminal.

### `terminal_view.rs`: the widget

- Fills the tab. Cell size from the editor font (`glyph_width('M')`,
  `row_height`); the grid is `floor(width / cell)` × `floor(height / row)`,
  resized when that changes. A thin scrollbar shows the scrollback.
- Painting locks the `Term` once per frame and walks `renderable_content()`:
  backgrounds as rectangles merged per run, text per run of same-styled
  printable ASCII (one galley), other characters placed one per cell (wide
  characters over two), as the editor does. Bold, italic, underline,
  strikethrough, dim, inverse and hidden. Selection over the cells, the
  cursor as a block (hollow when unfocused), bar or underline.
- Keyboard: `terminal_keys.rs` turns egui key events into bytes (xterm
  encoding: arrows and Home/End honour application cursor mode, modifiers as
  `CSI 1;m X`, function keys, Ctrl+letter as control codes, Alt as an ESC
  prefix). Text and IME commits are sent as UTF-8.
- Mouse: drag selects (double-click words, triple-click lines, Shift+click
  extends); when the program asks for mouse reports (SGR or the older
  encoding), clicks, drags and the wheel go to it instead (Shift bypasses).
  The wheel scrolls the scrollback, or sends arrow keys on the alternate
  screen (alternate scroll mode).
- Focus reporting (`CSI I`/`CSI O`) when asked. The focus lock filter keeps
  Tab, arrows and Escape in the widget.
- AccessKit: a node labelled `Terminal: <title>`; tests read the screen
  through `Terminal::screen_text`.

### App integration

- `Tab::Terminal(TerminalId)`. The first terminal splits the main surface
  below the editors (30 % of the height); later ones join the leaf of the
  last used terminal.
- Tab title: the program's title (OSC 0/2) when it sets one, else the shell's
  name; " (exited)" after it ends. Right-click menu: New Terminal, Close.
- `focused_terminal()` next to `focused_viewer()`: the status bar shows the
  shell, folder and grid size; the window title names the terminal; the idle
  focus return and the find panel's Escape treat a focused terminal like a
  focused editor.
- Links: on Ctrl+hover, the word under the pointer is matched as a URL
  (`http`, `https`) or a path with an optional `:line[:col]` (cargo's
  `--> src/main.rs:3:5`, Python's `File "x.py", line 3`), resolved against the
  terminal's folder. Ctrl+click opens it. Paths go through the same local-only
  check as the right-click menu's Open File (no network paths).
- Session: `Session::terminals` (`{ cwd }`), after the viewers in the
  layout's indices. Restoring starts new shells.
- Settings: `terminal_shell` (empty = default), `terminal_args`,
  `terminal_scrollback` (default 10 000 lines), `reload_silently`.

## 4. Steps

| Step | Content | Done when |
|---|---|---|
| T1 | Reload unmodified files silently (preference, `external.rs`), docs. | A test changes an open clean file on disk and sees it reloaded without a dialog; a dirty one still asks. |
| T2 | Dependency; `terminal.rs` (spawn, events, resize, input, shutdown); `terminal_keys.rs` with unit tests. | A test starts a shell, writes `echo` and reads the output from the grid, on Windows and Linux. |
| T3 | `terminal_view.rs`: painting, keyboard, paste/copy, selection, wheel, mouse reports, focus reports, resize. | Snapshot of a terminal with colours, selection and cursor; key encoding tests. |
| T4 | App: tab kind, View > Terminal, shortcuts and keyboard ownership, docking, titles, status bar, exit and restart, closing. | kittest: Ctrl+\` opens a terminal below the editor, typing runs a command, Ctrl+W reaches the shell, closing the tab ends it. |
| T5 | Links (Ctrl+click on paths and URLs). | Unit tests for the matcher (cargo, rustc, Python, plain paths, URLs, UNC refused); a kittest opens a file at a line. |
| T6 | Session, preferences (shell, arguments, scrollback). | Session round trip with a terminal tab. |
| T7 | Docs (`docs/terminal.md`, README, CHANGELOG, CLAUDE.md), review pass, Linux check. | fmt, clippy (also the Linux target), tests green. |

## 5. Risks

- **ConPTY**: needs Windows 10 1809 or later (ochrepad already needs
  Windows 10). Resizing reflows differently from Linux; ConPTY also rewrites
  some output. Accept its behaviour rather than fight it.
- **Keyboard**: egui reports AltGr as Ctrl+Alt and sends no text for it on
  Windows; layouts that type characters with AltGr may not reach the shell.
  The editor has the same limit today.
- **Tests with real shells**: CI has `pwsh` on Windows and `bash` on Linux;
  the tests run a fixed shell (`cmd.exe` / `/bin/sh`) to stay predictable.
- **Repaint cost**: painting locks the terminal while the I/O thread may be
  parsing a flood of output; `FairMutex` keeps the frame from starving, and
  only the visible screen is walked.
