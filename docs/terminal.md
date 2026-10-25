# The terminal

ochrepad has a terminal in a tab, so you can run `cargo build`, a Python
script or Claude Code next to the files you are working on. It is a real
terminal: colours, full-screen programs such as `vim`, `less`, `htop` and
Claude Code, the mouse, and a scrollback all work.

## Opening one

**View > Terminal** (Ctrl+\`) shows the terminal. The first time, it opens
one in a new pane below the editors. Pressed again from the terminal, it
goes back to the editor; pressed from the editor, it goes back to the
terminal you used last.

**View > New Terminal** (Ctrl+Shift+\`) opens another one, as a tab next to
the last terminal. Terminal tabs can be dragged, split and moved like any
other tab, and the right-click menu on a terminal's tab has **New Terminal**
and **Close**.

A new terminal starts in the open folder (**File > Open Folder**), or else
in the current file's folder, or else your home folder. The status bar shows
the shell, the folder it started in, whether it is running, and the
terminal's size in columns and rows.

## The shell

On Windows the shell is PowerShell 7 (`pwsh`) when it is installed, else
Windows PowerShell. On Linux it is your `$SHELL`, else `/bin/sh`. To use
another one, set **Settings > Preferences > Terminal shell** (for example
`cmd.exe`, `bash` or `C:\Program Files\Git\bin\bash.exe`) and its
arguments (for example `-NoLogo`). The change applies to terminals opened
afterwards.

Programs in the terminal see `TERM=xterm-256color`, `COLORTERM=truecolor`
and `TERM_PROGRAM=ochrepad`.

When the shell exits, the tab stays, marked "(ended)", with a line saying
how it ended. Press Enter in it to start a new shell in the same folder.

Closing a terminal's tab, or exiting ochrepad, ends its shell without
asking, as closing a terminal window does: the shell and the programs
running in it are told to stop. Programs that were started to run on their
own (with `nohup`, `start`, or a program that opens its own window) keep
running.

## Keys

While a terminal has the keyboard, it gets every key, including the ones
ochrepad would otherwise use (Ctrl+W, Ctrl+N, Ctrl+R and so on), except:

| Keys | |
|---|---|
| Ctrl+\`, Ctrl+Shift+\` | Show the editor or the terminal, open a new terminal |
| Ctrl+Tab, Ctrl+Shift+Tab, Ctrl+PageUp, Ctrl+PageDown | Switch tabs |
| F11 | Full screen |

The menus still work with the mouse.

| Keys | In the terminal |
|---|---|
| Ctrl+C | Copies the selection if there is one, else interrupts the program |
| Ctrl+Shift+C | Copies the selection |
| Ctrl+V, Ctrl+Shift+V | Paste (also Shift+Insert on Windows) |
| Shift+Enter | A new line without sending, in programs that take Alt+Enter that way (Claude Code does) |
| Shift+PageUp, Shift+PageDown | Scroll the scrollback a page |
| Shift+Home, Shift+End | Go to the top or the bottom of the scrollback |
| Ctrl+wheel | Zoom (the terminal uses the editor's font and zoom) |

Typing scrolls back to the bottom. Pasted text is marked as pasted for the
programs that ask for it (bash, zsh, fish, Claude Code, `vim`), so they can
tell it from typing: a pasted line break does not run a command there.

## The mouse

Drag to select; double-click selects a word, triple-click a line,
Shift+click extends the selection and Alt+drag selects a rectangle.
Selecting does not copy by itself: press Ctrl+C or Ctrl+Shift+C, or use the
right-click menu (**Copy**, **Paste**, **Select All**, **Clear**).

The wheel scrolls the scrollback. In a full-screen program it scrolls what
that program shows. A program that asks for the mouse (such as `htop`, or
`vim` with `set mouse=a`) gets clicks and the wheel; hold Shift to select
text instead.

## Links

Hold Ctrl and point at a web link, or at a file named in the output, and it
is underlined. Ctrl+click opens it: web links in your browser, files in
ochrepad at the line and column the output gave. ochrepad understands the
places compilers and tools write:

| Tool | Looks like |
|---|---|
| Rust, Go, gcc, TypeScript, grep -n | `src/main.rs:12:5`, `src/main.rs:12` |
| Python tracebacks | `File "app.py", line 7` |
| MSVC, .NET | `Program.cs(12,5)` |

A relative path is looked for in the folder the terminal started in, then
in the open folder. As with the editor's **Open File**, only files on this
computer open: network paths (`\\server\share\...`) are never looked at.

## Remembered

The session keeps your terminal tabs and the folders they started in, not
what was running in them. Next time each one starts a new shell when you
first look at it.

**Settings > Preferences > Terminal scrollback** sets how many lines each
terminal keeps above its screen (10,000 at first).

## Files the terminal changes

Programs in the terminal often change files you have open (a formatter, a
code generator, Claude Code). A file with no unsaved changes reloads by
itself; one with unsaved changes asks first. See
[Files changed by other programs](backups-and-session.md#files-changed-by-other-programs).

## Limits

- On Windows, a terminal needs Windows 10 version 1809 or later (for its
  pseudo console). Output passes through Windows' console, as in Windows
  Terminal, so a program's screen can be redrawn a little differently than
  on Linux.
- If ochrepad itself was started from Git Bash (MSYS), Windows may pass on
  Git Bash's "ignore Ctrl+C" setting, and Ctrl+C then does not interrupt
  programs in its terminals. Start ochrepad another way (the Start menu,
  Explorer, PowerShell).
- Keyboard layouts that type characters with AltGr may not reach the
  terminal; the editor has the same limit.
- The terminal does not know the shell's current folder after `cd`, so the
  status bar and relative links use the folder it started in.
