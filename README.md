# ochrepad

A fast, native text editor in the spirit of Notepad++, written in Rust with
[egui](https://github.com/emilk/egui).

![ochrepad with a Rust file and a Markdown preview side by side](docs/images/screenshot.png)

## Features

- **Syntax highlighting** for over 180 languages (syntect with the
  [two-face](https://github.com/CosmicHorrorDev/two-face) syntax set), detected
  by extension, file name or shebang. Highlighting is incremental, so typing
  stays smooth in very large files; files over a size limit (10 MB by default)
  open without it.
- **Column selection and multiple cursors**: Alt+drag or Alt+Shift+arrows for
  a rectangle, Ctrl+click to add a caret, Ctrl+Alt+Up/Down to add one above or
  below. Rectangular copy and paste work as blocks.
- **Find and replace** with regular expressions, match case, whole word, wrap
  around and in-selection, `$1`/`${name}` in replacements, Count, Select All
  (one caret per match) and Replace All as a single undo step. See
  [docs/find-and-replace.md](docs/find-and-replace.md) for the regex dialect.
- **Macros**: record, play back once, N times or to the end of the file, save
  them with a name and a shortcut.
- **Tabs and split screen**: drag tabs into side-by-side or stacked panes, or
  open a second view of the same file; edits show in both.
- **Compare**: a side-by-side diff of two files (or a file against its saved
  version) with aligned lines, changed words highlighted, and F8/Shift+F8 to
  jump between changes.
- **Markdown preview** beside the editor, updating as you type, with images
  (PNG, JPEG, WebP, SVG) loaded from the document's folder.
- **Backups and session restore**: unsaved work is backed up every few seconds
  and comes back, with your tabs and layout, the next time you start. Optional
  automatic saving of named files.
- **Text tools**: UPPER/lower/Title/iNVERT case, sort lines (ascending,
  descending, case-insensitive, numeric, by a column selection), duplicate,
  move, join lines and trim trailing whitespace.
- **Faithful file handling**: UTF-8, UTF-16 and legacy encodings are detected,
  and files are saved back with the same encoding, BOM and line endings
  (CRLF, LF or CR). Saves are atomic, so a crash never leaves a half-written
  file. Binary files are refused rather than mangled.
- **Watches open files**: if another program changes or deletes one, ochrepad
  offers to reload it (or warns you if you have unsaved changes).
- Preferences for font, size, tabs or spaces, tab width, theme, and backups;
  an editable keymap; recent files; drag and drop to open.

Not included (by design, for now): word wrap, code folding, right-to-left
text, proportional fonts, plugins and language servers.

## Building

You need a recent stable [Rust toolchain](https://rustup.rs) (1.88 or newer).

```sh
git clone https://github.com/xor295/ochrepad.git
cd ochrepad
cargo build --release
```

The program is `target/release/ochrepad` (`ochrepad.exe` on Windows). Pass file
names to open them:

```sh
ochrepad notes.md src/main.rs
```

On Windows the build embeds the icon into the executable; that needs the
Windows SDK's resource compiler (installed with the Visual Studio C++ build
tools that Rust uses). Without it the build still succeeds, with a warning.

ochrepad is developed and tested on Windows 11. The code is cross-platform
(eframe, rfd and notify all support Linux and macOS), but those platforms have
not been tested yet. On Linux, file dialogs go through the XDG desktop
portal.

## Keyboard shortcuts

| | |
|---|---|
| New, Open, Save, Save As | Ctrl+N, Ctrl+O, Ctrl+S, Ctrl+Shift+S |
| Close tab, next/previous tab | Ctrl+W, Ctrl+Tab, Ctrl+Shift+Tab |
| Undo, Redo | Ctrl+Z, Ctrl+Y (or Ctrl+Shift+Z) |
| Find, Replace | Ctrl+F, Ctrl+H |
| Find next/previous | F3, Shift+F3 (Enter, Shift+Enter in the panel) |
| Column selection | Alt+drag, Alt+Shift+arrows |
| Add a caret | Ctrl+click, Ctrl+Alt+Up/Down |
| Back to one caret | Esc |
| Select a line | click its line number (drag for several) |
| Duplicate, move, join lines | Ctrl+D, Ctrl+Shift+Up/Down, Ctrl+J |
| UPPERCASE, lowercase | Ctrl+Shift+U, Ctrl+U |
| Title Case, iNVERT cASE | Alt+U, Ctrl+Alt+U |
| Record/stop macro, play macro | Ctrl+Shift+R, Ctrl+Shift+P |
| Markdown preview | Ctrl+Shift+M |
| Next/previous difference | F8, Shift+F8 |

Double-click the empty part of a tab bar to open a new tab there. Sorting and
the other line tools are in **Edit > Line Operations**.

## Configuration

**Settings > Preferences** covers the editor font and size, tab width, tabs or
spaces, light/dark theme, syntax theme, backups and automatic saving, and the
size above which highlighting is turned off.

Everything is stored as JSON in your configuration folder:

| | Windows | Linux | macOS |
|---|---|---|---|
| Settings, keymap, macros, session | `%APPDATA%\ochrepad\config` | `~/.config/ochrepad` | `~/Library/Application Support/ochrepad` |
| Backups of unsaved work | `%APPDATA%\ochrepad\data` | `~/.local/share/ochrepad` | `~/Library/Application Support/ochrepad` |

- `settings.json`: the preferences and the recent files list.
- `keymap.json`: your key bindings. **Settings > Edit Keymap** opens it,
  starting from every default binding, and saving it applies it. Each entry
  replaces the default binding for its keys; `"command": null` removes one:

  ```json
  [
    { "keys": "Ctrl+D", "command": { "Action": "SelectLine" } },
    { "keys": "Ctrl+J", "command": null }
  ]
  ```
- `macros.json`: saved macros and their shortcuts.
- `session.json`: open tabs, layout, carets and scroll positions.

## Development

The editor is a Cargo workspace:

| Crate | What it does |
|---|---|
| `crates/ochrepad-core` | The text model and every editing operation: positions, selections, actions, undo, search, transforms, macros, diff. No UI and no file access, so it is tested headlessly. |
| `crates/ochrepad-fileio` | Loading and saving: encoding and line-ending detection, atomic writes, the background writer, backups and config files. |
| `crates/ochrepad-syntax` | Incremental syntax highlighting on syntect. |
| `crates/ochrepad` | The egui application: the editor widget, tabs and splits, panels, preview, compare view. |

```sh
cargo test --workspace                        # all tests
cargo clippy --all-targets -- -D warnings     # lints
UPDATE_SNAPSHOTS=1 cargo test -p ochrepad     # regenerate UI snapshot images
```

The app's tests drive the real UI with
[egui_kittest](https://github.com/emilk/egui/tree/main/crates/egui_kittest):
they type, click and drag, then check the text and selections; snapshot tests
compare rendered images. Benchmarks for scrolling, find and diff on large files
are under `crates/ochrepad/examples` (generate the test files first with
`cargo run --release -p ochrepad --example gen_perf_fixtures`).

## License

[MIT](LICENSE)
