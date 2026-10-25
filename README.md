# ochrepad

[![CI](https://github.com/xor295/ochrepad/actions/workflows/ci.yml/badge.svg)](https://github.com/xor295/ochrepad/actions/workflows/ci.yml)

A fast, native text editor in the spirit of Notepad++, written in Rust with
[egui](https://github.com/emilk/egui). It runs on Windows and Linux.

![ochrepad with a folder open in the sidebar, a Rust file with bookmarks and the document map, and the Document List](docs/images/screenshot.png)

## Features

**Editing**

- **Multiple cursors and column selection**: Ctrl+click adds a caret,
  Alt+drag selects a rectangle ([more](docs/column-selection.md)).
- **Find and replace** with regular expressions, in one document, every
  open document or a whole folder (`.gitignore` respected)
  ([more](docs/find-and-replace.md)).
- **Macros, bookmarks and folding**, with Notepad++'s shortcuts
  ([macros](docs/macros.md), [bookmarks](docs/editing.md#bookmarks),
  [folding](docs/view.md)).
- **Notepad++'s editing tools**: line and blank operations, the Column
  Editor, case changes, sorting, and JSON, Base64 and hash tools
  ([editing](docs/editing.md), [text tools](docs/text-tools.md)).

**Viewing**

- **Syntax highlighting** for over 180 languages, incremental so typing
  stays smooth in large files.
- **Split panes**, with several views of one file. A **document map**, the
  **Function List** and **Document List** panels, **change history** in the
  gutter, and word wrap ([more](docs/view.md)).
- **Compare** two files side by side, both sides editable, with the diff
  updating as you type.
- **Markdown preview** that updates as you type and scrolls with the editor.

**Files**

- **Folder sidebar**: browse, create and rename files, with ignored files
  hidden ([more](docs/workspace.md)).
- **Files saved as they were found**: encoding, BOM and line endings are
  kept, and saves are atomic ([more](docs/file-handling.md)).
- **Nothing lost**: unsaved work is backed up every few seconds and comes
  back with your tabs next time. When another program changes an open
  file, ochrepad reloads it, or asks first if you have unsaved changes
  ([more](docs/backups-and-session.md)).
- **Logs and huge files**: filter lines, color log levels, follow a growing
  file (tail -f), and open multi-gigabyte files read-only in an instant
  ([more](docs/logs.md)).
- **A terminal** below your files: run builds, scripts or Claude Code, and
  Ctrl+click a compiler error to open the file at that line
  ([more](docs/terminal.md)).
- **Completion** as you type, with nothing to install: words and names from
  your documents and the open folder, keywords, and the functions and types
  the highlighting finds; `gtl` finds `getTextLength`
  ([more](docs/completion.md)).

Not included (by design, for now): right-to-left text, proportional fonts,
plugins and language servers.

## Download

Download the latest version from the
[releases page](https://github.com/xor295/ochrepad/releases):

- **Windows**: `ochrepad-<version>-windows-x86_64.zip`. Unzip it anywhere and
  run `ochrepad.exe`; there is nothing to install. The program isn't signed
  yet, so the first time, Windows SmartScreen may say it "protected your
  PC": choose **More info**, then **Run anyway**.
- **Linux**: `ochrepad-<version>-linux-x86_64.tar.gz`. Extract it and run
  `ochrepad`. It needs an X11 or Wayland desktop with Vulkan or OpenGL
  drivers, and runs on distributions as old as Ubuntu 22.04.

Each release lists SHA-256 checksums, and every download carries a signed
record that it was built by this repository's release workflow; the release
notes say how to check it.

## Building

You need a recent stable [Rust toolchain](https://rustup.rs) (1.89 or newer).
Install it with `cargo install ochrepad`, or from a clone:

```sh
git clone https://github.com/xor295/ochrepad.git
cd ochrepad
cargo build --release
```

The program is `target/release/ochrepad` (`ochrepad.exe` on Windows). Pass
file names to open them: `ochrepad notes.md src/main.rs`.

- **Windows**: the build embeds the icon with the Windows SDK's resource
  compiler (installed with the Visual Studio C++ build tools). Without it the
  build still succeeds, with a warning.
- **Linux**: install eframe's development packages first (Debian and Ubuntu
  names). File dialogs go through the XDG desktop portal.

  ```sh
  sudo apt-get install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev
  ```

macOS is not supported.

## Keyboard shortcuts

Most of Notepad++'s shortcuts work as they do there. The everyday ones:

| | |
|---|---|
| Find, Replace, Find in Files | Ctrl+F, Ctrl+H, Ctrl+Shift+F |
| Find next/previous | F3, Shift+F3 |
| Go to line | Ctrl+G |
| Add a caret, column selection | Ctrl+click or Ctrl+Alt+Up/Down, Alt+drag or Alt+Shift+arrows |
| Duplicate, move, delete lines | Ctrl+D, Ctrl+Shift+Up/Down, Ctrl+Shift+L |
| Toggle comment | Ctrl+/ or Ctrl+Q |
| Bookmark a line, next, previous | Ctrl+F2, F2, Shift+F2 |
| Fold, unfold, fold all | Ctrl+Alt+F, Ctrl+Alt+Shift+F, Alt+0 |
| Record, play a macro | Ctrl+Shift+R, Ctrl+Shift+P |
| Next/previous tab, reopen closed tab | Ctrl+Tab, Ctrl+Shift+Tab, Ctrl+Shift+T |
| Markdown preview, sidebar | Ctrl+Shift+M, Ctrl+Shift+E |
| Terminal, new terminal | Ctrl+\`, Ctrl+Shift+\` |
| Filter lines | Alt+Shift+L |
| Next/previous difference or change | F8, Shift+F8 |
| Zoom | Ctrl+plus, Ctrl+minus, Ctrl+0 (or Ctrl+wheel) |

**Settings > Edit Keymap** lists every binding and lets you change them.

## Configuration

**Settings > Preferences** covers the editor font and size, tabs or spaces,
theme, backups and automatic saving. Everything is stored as JSON:

| | Windows | Linux |
|---|---|---|
| Settings, keymap, macros, session | `%APPDATA%\ochrepad\config` | `~/.config/ochrepad` |
| Backups of unsaved work | `%APPDATA%\ochrepad\data\backup` | `~/.local/share/ochrepad/backup` |

`keymap.json` holds your key bindings. Each entry replaces the default for
its keys, and `"command": null` removes one; [Key bindings](docs/keymap.md)
lists every command:

```json
[
  { "keys": "Ctrl+D", "command": { "Action": "SelectLine" } },
  { "keys": "Ctrl+J", "command": null }
]
```

The rest of the documentation is in [docs/](docs/README.md).

## Development

A Cargo workspace of four crates: `ochrepad-core` (the text model and every
editing operation, with no UI or file access), `ochrepad-fileio` (loading,
saving, backups), `ochrepad-syntax` (incremental highlighting on syntect)
and `ochrepad` (the egui app).

```sh
cargo test --workspace
cargo clippy --all-targets -- -D warnings
```

The app's tests drive the real UI with
[egui_kittest](https://github.com/emilk/egui/tree/main/crates/egui_kittest),
and snapshot tests compare rendered images (`UPDATE_SNAPSHOTS=1` regenerates
them). Benchmarks for large files are in `crates/ochrepad/examples`.

## License

[MIT](LICENSE)
