# ochrepad

A Notepad++-style text editor in Rust on egui/eframe 0.36. Version 1 is feature-complete: every milestone of the original plan (M0–M13) is done. Work now is maintenance, polish and new features. `PLAN.md` is the original design brief, kept for its rationale; this file is the current guide.

## Commands

```sh
cargo build --release                         # target/release/ochrepad(.exe)
cargo test --workspace                        # everything (~1 min in debug; props + incremental are the slow ones)
cargo fmt
cargo clippy --all-targets -- -D warnings
UPDATE_SNAPSHOTS=1 cargo test -p ochrepad --test visual --test compare_ui --test find_ui --test preview
cargo run --release -p ochrepad --example gen_perf_fixtures   # 50k-line .rs + 100 MB .txt in target/perf-fixtures
cargo run --release -p ochrepad -- --bench <file>             # scroll/typing frame times, prints a table
cargo run --release -p ochrepad --example find_bench -- <file>
cargo run --release -p ochrepad --example diff_bench -- <file>
cargo run -p ochrepad --features dev-inspect                  # egui inspection protocol, with EGUI_INSPECTION=1
```

A change is done when fmt, clippy (`-D warnings`) and `cargo test --workspace` pass. Commit at green; the user pushes.

The release exe is locked while it runs, so `cargo build --release` fails with "Access is denied" if the user has ochrepad open. That is not a build error.

## Layout

| Crate (folder = package) | Contents |
|---|---|
| `crates/ochrepad-core` | Text model and all editing. **No egui and no filesystem access.** `text` (ropey wrapper, the only place that touches ropey), `position` (newtypes and conversions, the only place that knows tabs, graphemes and display widths), `selection`, `changeset`, `action` + `resolve` (Action → Transaction), `history` (undo), `buffer`, `view`, `editor` (`Editor::apply`, macro playback, reload), `search` (regex-cursor behind a `Matcher` trait), `transform` (case, sort, line ops), `macros`, `diff` (similar; aligned rows, `SideMap` DisplayMap). |
| `crates/ochrepad-fileio` | Load (BOM, strict UTF-8, chardetng fallback, UTF-16, binary refusal, line-ending detection and LF normalization) and save (re-encode with the original encoding, BOM and ending, atomic temp file + rename). `worker`: the background thread that does **all** disk writes. Config and data dirs (`directories`), JSON helpers, snapshot files. |
| `crates/ochrepad-syntax` | syntect (pure-Rust `default-fancy` regex) + two-face syntaxes and themes. Language detection, and a per-buffer incremental highlighter with `(ParseState, HighlightState)` checkpoints every 128 lines that re-highlights until a checkpoint converges. |
| `crates/ochrepad` | The app. `app/mod.rs`: `OchrepadApp` (dock of `Tab::{Editor, Preview, Compare}`, menus, open/save/close, dialogs, status bar, `Tabs` TabViewer). `app/autosave.rs`: snapshots, session, write-through, async save bookkeeping. `app/external.rs`: notify watcher and reload prompts. `app/prefs.rs`: settings, keymap file, recent files, drag and drop. `editor_view.rs`: the editor widget (input, hit testing, painting, scrolling). `layout.rs`: `LineLayout` and `paint_line_text`. `highlight.rs`, `find_panel.rs`, `macro_ui.rs`, `compare.rs`, `preview.rs`, `safe_images.rs`, `keymap.rs`, `settings.rs`, `theme.rs`, `logo.rs`, `fonts.rs`. `build.rs` embeds the icon on Windows. |

## Architecture rules

These are load-bearing. Check with the user before changing any of them.

- **Byte offsets are the canonical position.** Every stored position is a `ByteOffset` into the UTF-8 text, on a char boundary. `CharIdx`, `LineCol` and `VisualCol` are newtypes derived on demand in `core::position`. The cursor moves by grapheme cluster.
- **Selections are always a `SelectionSet`**: non-empty, sorted, non-overlapping, with a primary index. A column selection is a set (one selection per line) plus an optional `Rectangle`. Any action that is not column-aware clears the rectangle.
- **Two-layer edits.** An `Action` is the user's intent (serde-serializable; key bindings and macros use Actions). It resolves to a `Transaction` (a `ChangeSet` plus selections before and after); undo stores Transactions. **`Editor::apply` is the only path that mutates a rope.** It pushes undo, maps other views' selections with `ChangeSet::map_position`, records the `LineEdit`s for the highlighter, bumps the revision and records macros.
- Dirty means `revision != saved_revision`, so undoing back to the save makes the tab clean again. Typing coalesces into one undo step; macro playback, Replace All and every transform are one step each.
- **Line endings are LF inside.** `fileio` normalizes on load and restores on save. Nothing outside `fileio` looks at `\r`. Mixed endings are saved as the dominant one, with a status-bar note.
- **One layout for paint and hit test.** `LineLayout` holds the x of every grapheme boundary; painting, carets, selections and clicks all use it, and the compare view reuses it. Never compute x positions in a second place. Monospace grid: x = `VisualCol × cell width`; wide and fallback glyphs are placed individually on the grid, and emoji are clipped to their cells.
- **Line ↔ row mapping goes through `DisplayMap`.** It is the identity for editors and `SideMap` for diffs, and it is the seam for future word wrap.
- The editor widget does its own virtual scrolling (first line + fraction). Do not use egui `ScrollArea` for it: f32 pixel offsets lose precision in big files.

## Working rules

- `ochrepad-core` stays free of egui and of filesystem access. If you need either there, the design is wrong; ask.
- No `unwrap()`/`expect()` on user data (file contents, patterns, config, session). Errors surface in the UI (`self.message` modal or inline).
- User files are written only by atomic replace, and only from the worker thread. Never block a frame on I/O.
- Dependencies are pinned exactly (`"=x.y.z"`). Ask before adding one. Approved additions since the plan: `image` (jpeg, webp), `winresource` (Windows build-dependency), `resvg` (no default features). All egui-ecosystem crates must stay on the same egui minor version (egui/eframe/egui_kittest 0.36, egui_dock 0.21, egui_commonmark 0.25).
- egui 0.36 is Ui-first: `App::ui(&mut self, ui, frame)`, and panels show inside a `Ui`. Online examples are often older; check the crate source in `~/.cargo/registry` rather than memory.
- Match Notepad++ behaviour where it has an opinion (shortcuts, autosave-as-backup, exit without prompting when backups are on).

## Settled decisions

- **Fonts**: a CJK fallback font is loaded from the system (else tofu stays on the grid). Menus use the system UI font (Segoe UI on Windows; Linux candidates; egui's font on macOS). The editor font is a choice among installed monospace fonts, or the built-in Hack.
- **Regex**: Rust `regex` syntax through `regex-cursor` (no lookaround or backreferences), documented in `docs/find-and-replace.md`. A `fancy-regex` fallback can be added behind the `Matcher` trait.
- **Look** (`theme.rs`): warm charcoal and paper palettes with ochre `#CC7722` accents (caret, selection, focus, current line number, active tab line, hovered split divider). Find matches are teal and changed compare rows blue, so they never look like the selection. The tab bar is 32 px tall.
- **Logo**: `logo.rs` is dependency-free and shared by the window icon and `build.rs` (included by path), which generates the `.ico` for `winresource`. A failed embed is a warning, not an error.
- **Markdown preview images load only from this machine** (`safe_images.rs`, from a security review). `LocalFilesOnly` sits above egui_extras' loaders and refuses `file://` URIs that are not on a local drive, because UNC paths would send NTLM credentials. `SafeSvgLoader` replaces egui_extras' SVG loader with an `<image>` resolver that reads only files inside the SVG's folder. Install order matters: egui asks the newest loader first, and `install()` draws an empty `CommonMarkViewer` first so egui_extras' loaders go in below the guards.
- **Mouse**: a double-click on an empty tab bar opens a new tab in that pane. A click on a line number selects the line; drag and Shift+click extend by lines. Hovering another menu title switches menus while one is open (egui's `MenuBar` does not do this; see `switch_menu_on_hover`).
- **Release builds** use the Windows GUI subsystem (no console), so `--bench` output must be redirected to a file.
- **Settings** live in `AppSettings` (`settings.json`) and are applied in `apply_settings` only when they change. `keymap.json` entries replace or remove (`"command": null`) default bindings and apply on save.

## Not in scope (unless the user asks)

Word wrap, code folding, RTL/bidi text, proportional editor fonts and ligatures, virtual space, plugins/LSP/autocomplete, special handling for files over ~200 MB, full screen-reader support (a basic AccessKit node exists).

## Testing

- **Core**: unit tests plus proptest (`crates/ochrepad-core/tests/props.rs`): random action sequences, undo/redo identity, selection invariants, position round-trips. Keep generated text short (`short_text()`); long Replace/ReplaceAll replacements grew documents exponentially and hung debug runs.
- **fileio**: byte-exact fixtures in `tests/fixtures` (marked `-text` in `.gitattributes`; never let git or an editor touch their line endings). Bytes in must equal bytes out.
- **App**: one kittest file per feature in `crates/ochrepad/tests`. Harnesses use `.with_step_dt(1/60)` and `.with_max_steps(10_000)`, because highlighting runs on a wall-clock budget. The editor exposes an AccessKit node labelled `Editor: <file name>`, and the status bar shows `Ln x, Col y` as a label; tests find both by label. Mouse tests compute positions from the font's cell size (see `cell_pos` in `widget.rs`).
- **Snapshots** (`tests/snapshots/*.png`) use egui's built-in fonts only, so they don't depend on system fonts. After a visual change, regenerate them and **look at the PNGs** before committing. The kittest harness draws a transparent 8 pt margin around the app; real windows have none.
- Assert timings only in release builds (`cfg!(debug_assertions)` guards). File-watcher tests wait in real time (`wait_for` in `polish.rs`).
- To review a UI change at full size, render the app at `with_pixels_per_point(1.5)` after `ochrepad::fonts::install(&h.ctx)` in a throwaway test, save `h.render()` to the scratchpad, and look at it. Delete the test afterwards.

## Gotchas learned the hard way

- **egui_dock**: `retain_tabs` can leave a stale focused node and panic; remove tabs with `find_tab` + `remove_tab`. There are no hooks for tab-bar double-clicks or per-tab accents; both are done after `DockArea::show` using tab rects collected in `on_tab_button`.
- **Windows file URIs** need an empty host (`file:///C:/dir/x.png`); `file://C:/...` reads `C:` as a host.
- **`Color32` is premultiplied.** Build translucent tints with `from_rgba_unmultiplied` from straight RGB; a weak tint cannot be made strong again.
- **egui key names** are `"Down"`, not `"ArrowDown"` (`Chord::parse` accepts both). Menu items with shortcut text have accessibility labels like `"Open…Ctrl+O"`, so tests use `get_by_label_contains`.
- **Config dir**: the keymap file's folder may not exist on a first run; create it before saving there.
- **Deleted files cannot be canonicalized.** The watcher matches paths by canonical parent + file name (`watch_key`).
- **Python dictionaries**: the first key after a line-ending `{` is coloured as a plain string. This is a limitation of two-face's Python grammar under syntect (it cannot look past the line), not our highlighter.
- Git prints LF→CRLF warnings on commit (autocrlf); they are harmless.

## Open ideas (not started)

- Allow only http(s) when a preview link is clicked (other schemes currently go to the browser).
- Release profile size options (`strip`, `lto`) — measured 20.2 → 17.7 MB; the user hasn't decided.
- macOS: system UI font (a variable font) and testing on Linux/macOS generally.
