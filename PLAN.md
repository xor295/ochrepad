# Project plan: a Notepad++-style text editor in Rust (egui)

This document is the handoff brief for building the editor. Read it fully before writing code. The architecture decisions in section 3 are deliberate; do not change them without checking with the user first.

## 1. Goal and scope

A native desktop text editor, written in Rust on egui/eframe, recreating these Notepad++ features:

- Syntax highlighting for most common languages
- Vertical (column / rectangular) selection, plus multi-cursor
- Regex find and replace
- Recordable, replayable macros
- Markdown preview
- Tabbed layout
- Autosave
- Convert selection to uppercase / lowercase (plus title case and invert case)
- Line sorting
- Split screen (two views of the same or different files)
- File diff / compare

### Non-goals for v1

Each of these is expensive and would leak into every layer. Do not implement them. Section 3 notes where to leave a seam so they can be added later.

- Word wrap
- Code folding
- Right-to-left and bidirectional text
- Proportional fonts and font ligatures
- Virtual space (caret past the end of a line)
- Plugins, LSP, autocomplete
- Special handling for files over ~200 MB (they may be slow; they must not corrupt)
- Full screen-reader accessibility (a basic AccessKit node is required, see section 6)

## 2. What we are building, and why it is the hard part

Notepad++ gets its editing surface from Scintilla. Rust has no drop-in equivalent, and egui's built-in `TextEdit` is not suitable: it lays out the whole text every frame, has a single cursor, and has no column selection. So we write our own editing widget. That widget is most of the work; the feature list mostly sits on top of it.

The editor has six layers:

1. **Storage** — a rope (`ropey`).
2. **Position math** — converting between byte offsets, char indices, line/column and visual (on-screen) columns.
3. **Layout and painting** — draw only the visible lines, with styled runs, selection backgrounds, carets, and a line-number gutter.
4. **Hit testing** — the exact inverse of layout: pixel to text position.
5. **Input** — keyboard, mouse, drag, double/triple click, autoscroll, clipboard, IME.
6. **Style storage** — syntax highlighting state cached so edits re-highlight incrementally.

## 3. Architecture decisions (fixed)

### 3.1 Workspace layout

A Cargo workspace. The `core` crate must not depend on egui, so the editing logic is fully testable headless.

```
ochrepad/
  Cargo.toml            # [workspace]
  crates/
    core/               # no egui, no filesystem: text, positions, selections,
                        # actions, transactions, undo, macros, search, transforms, diff
    syntax/             # syntect wrapper + highlight checkpoint cache
    fileio/             # load/save, encoding & line-ending detection, atomic writes,
                        # autosave snapshots, session restore, config dir
    app/                # eframe binary: editor widget, dock/tabs, menus, panels,
                        # markdown preview, diff view
```

### 3.2 One canonical position type: byte offset

Every stored position (selections, transactions, search results, highlight spans) is a **byte offset into the document's UTF-8 text**, always on a char boundary. Other position kinds are derived on demand.

Wrap each kind in a newtype so the compiler rejects mixing them:

- `ByteOffset(usize)` — canonical. Always valid on a char boundary.
- `CharIdx(usize)` — only used at the boundary with Ropey 1.x APIs that take char indices.
- `LineCol { line: usize, col: ByteOffset }` — column as a byte offset within the line.
- `VisualCol(usize)` — on-screen cell column after tab expansion and wide-glyph widths.

Conversions are explicit functions in `core::position`, and they are the only place that knows about tabs (`tab_width`), grapheme clusters (`unicode-segmentation`) and display widths (`unicode-width`). Cursor movement steps by grapheme cluster, not by char.

Rationale: `regex-cursor`, `syntect` and `similar` all speak byte offsets, and Ropey 2 (currently in beta) switches to byte indexing. Using bytes internally keeps a later Ropey 2 migration contained to one module.

### 3.3 Selections are always a set

`SelectionSet` holds a non-empty, sorted, non-overlapping `Vec<Selection>` plus the index of the primary selection. A `Selection` is `{ anchor: ByteOffset, head: ByteOffset }` plus an optional remembered `VisualCol` for vertical movement.

- A plain cursor is a set of one empty selection. There is no separate single-cursor code path.
- After every transaction, the set is normalized: sorted, overlapping or touching selections merged.
- Column selection is an action that *produces* a set (one selection per line); it is not a separate selection type. Keep the originating rectangle (`start line, end line, start VisualCol, end VisualCol`) alongside the set so rectangular copy/paste and column-keyed sort can use it. Any action that is not column-aware clears the rectangle.

### 3.4 Buffer, View, Workspace

- `Buffer` (in `core`): the rope, undo history, revision counter, saved-revision marker, language id, and file metadata (path, encoding, BOM, line ending). Identified by `BufferId`.
- `View` (in `core`): a `BufferId`, a `SelectionSet`, scroll position (first visible line, horizontal offset). Identified by `ViewId`. Many views may point at one buffer.
- `Workspace` (in `app`): owns all buffers and views, and the dock layout. Tabs and split panes in the dock hold `ViewId`s.

When a transaction is applied to a buffer, every *other* view of that buffer maps its selections through the transaction (3.5) so carets in the other pane stay put relative to the text.

Seam for later word wrap and diff alignment: views never assume "document line N is screen row N". Route that mapping through a single `DisplayMap` trait (line ↔ display row). The v1 implementation is the identity.

### 3.5 Two-layer edit model: Actions and Transactions

This is the most important decision. There are two distinct layers:

**`Action`** — what the user intends, independent of where the cursor is. Examples: `InsertText(String)`, `DeleteBackward`, `MoveCursor { motion: Motion, extend: bool }`, `SelectAll`, `AddCursorBelow`, `ColumnSelect { .. }`, `ConvertCase(CaseMode)`, `SortLines(SortOptions)`, `ReplaceAll { .. }`, `Undo`, `Redo`. Key bindings map to Actions. **Macros record Actions.** Actions are `serde`-serializable so macros can be saved.

**`Transaction`** — the concrete result of resolving an Action against the current state: a `ChangeSet` (a list of non-overlapping `(byte range, replacement text)` edits against a specific revision), plus the selection set before and after. **Undo/redo stores Transactions** (with their inverses).

All mutation goes through one function:

```
fn apply(workspace_or_editor, view: ViewId, action: Action) -> Result<(), ActionError>
```

which resolves the action to a transaction, applies it to the rope, pushes undo, maps other views' selections, notifies the highlighter of the dirty line range, bumps the revision, and appends the action to the macro recorder if recording. Nothing else mutates a rope.

Why two layers: recording concrete edits would make macros replay at the original positions; recording intents lets "move to line end, type a semicolon, move down" replay correctly from wherever the caret is.

`ChangeSet` must provide `map_position(ByteOffset, Bias) -> ByteOffset` so selections in other views, search-match lists and highlight checkpoints can be shifted after an edit.

Undo details:

- Consecutive single-character inserts (and consecutive backspaces) coalesce into one undo step, broken by cursor movement, a pause over ~1 s, or a newline.
- A macro playback is one undo step. "Replace all" is one undo step.
- Dirty state is `buffer.revision != buffer.saved_revision`, so undoing back to the saved state makes the tab clean again.

### 3.6 Encoding and line endings at load time

On open (`fileio`):

1. Detect BOM (UTF-8, UTF-16 LE/BE). If none, try strict UTF-8; if that fails, guess with `chardetng` and decode with `encoding_rs`.
2. If the file looks binary (NUL bytes in the first 8 KB) or decoding fails, refuse to open and say why. Never silently replace bytes.
3. Detect the dominant line ending (CRLF, LF, CR). Normalize the buffer to LF internally. Record the original ending and whether the file had mixed endings (show a status-bar note).
4. Store encoding, BOM and line ending on the `Buffer`. Save writes back with exactly those, unless the user changes them from the status bar.

Nothing outside `fileio` ever checks for `\r`.

### 3.7 Rendering: one layout function for paint and hit test

The bug class to prevent is "click lands one character off". Build a `LineLayout` for each visible line (in `app`), computed once per frame per line and cached by `(buffer id, line index, revision of that line's text, highlight generation, font size)`. It contains the x position of every grapheme boundary. Painting uses it to place text, selection rectangles and carets; hit testing uses the *same* struct to map x back to a byte offset. Never compute x positions in two places.

Use a monospace font. Positions are `VisualCol × cell_width`, where `VisualCol` comes from `core::position` (tabs expand to the next tab stop, wide glyphs count as 2). Draw styled runs with egui `LayoutJob`s; for any run containing wide glyphs or fallback-font glyphs, place graphemes individually so they stay on the grid.

Only lines in the viewport (plus a small margin) are laid out. Scrolling is virtual: total height = line count × line height.

## 4. Dependencies

Use `cargo add` so you get the current versions, then pin them. Do **not** rely on memory for egui APIs: egui 0.34 moved the app entry point from `Context` to a whole-app `Ui`, and much online example code predates that. Check docs.rs for the exact version in `Cargo.lock`.

The egui-ecosystem crates must all target the same egui minor version. At the time of writing that is egui/eframe 0.36, with `egui_dock` 0.21 and `egui_commonmark` 0.25. If one lags, hold everything at the newest egui version all of them support.

| Purpose | Crate | Notes |
|---|---|---|
| App shell | `eframe`, `egui` | Enable eframe's `inspection` feature behind a dev cargo feature (section 6). |
| Rope | `ropey` 1.x | Stable. Ropey 2 is in beta and byte-indexed; do not adopt it in v1, but keep Ropey calls inside `core::text` so a migration is local. |
| Unicode | `unicode-segmentation`, `unicode-width` | Grapheme stepping, display width. |
| Highlighting | `syntect`, `two-face` | `two-face` adds many extra syntaxes and themes. Start with syntect's pure-Rust `default-fancy` regex feature to avoid a C toolchain; switch to the Oniguruma backend only if profiling demands it. |
| Regex search over a rope | `regex-cursor` | Searches the rope's chunks without copying to a `String`. It is a prototype-stage fork of `regex-automata`; wrap it behind a `core::search` trait. Confirm its `ropey` feature matches our Ropey version; if not, implement its `Cursor` trait over Ropey chunks ourselves (small). |
| Regex replacement expansion | `regex` | For `$1` / `${name}` expansion on each match. |
| Tabs and splits | `egui_dock` | |
| Markdown preview | `egui_commonmark` | Enable its syntect-based code-block highlighting feature if it shares our syntect version. |
| Diff | `similar` | Line diff, then word diff within changed line pairs. |
| Encoding | `encoding_rs`, `chardetng` | |
| Atomic writes | `tempfile` | Write to a temp file in the same directory, then persist (rename) over the target. |
| Config/data dirs | `directories` | Settings, macros, autosave snapshots, session. |
| Serialization | `serde`, `serde_json` or `toml` | Settings, keymap, macros, session. |
| File dialogs | `rfd` | |
| File watching (late milestone) | `notify` | External change detection. |
| Tests | `proptest`, `insta`, `egui_kittest` | |

## 5. Milestones

Each milestone ends with `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test --workspace` passing, and a short summary to the user of what works and what doesn't. Sizes are relative (S/M/L).

### M0 — Rendering spike (S, throwaway branch)

Prove egui can do what section 3.7 needs before building on it.

- eframe window showing 200,000 lines of generated monospace text in a virtual-scrolling area with a gutter, a blinking caret, and a few colored runs per line.
- Mouse click places the caret using the same layout data used for painting.
- Measure frame time while scrolling fast, in a release build.

**Exit:** report frame times and any egui text-measurement limitations to the user and **stop for their go-ahead** before M1. Don't merge the spike.

### M1 — Headless core (L)

In `core`, with no UI:

- `Document` wrapper over the rope; position newtypes and conversions (3.2).
- `SelectionSet` with normalization (3.3).
- `ChangeSet`, `Transaction`, inverse, `map_position` (3.5).
- `Action` enum covering: insert text, newline, delete backward/forward, delete word, all motions (char/grapheme, word, line start/end with smart home, document start/end, page up/down given a page height, up/down with remembered visual column), select variants of each, select all, select word/line, add cursor above/below.
- `apply()`, undo/redo with coalescing, revisions, saved-revision dirty logic.
- `Buffer`, `View`, mapping other views' selections on edit.

**Tests:** unit tests for every motion including tabs, combining marks (e.g. `e\u{301}`), emoji ZWJ sequences, CJK; and property tests (`proptest`) asserting:

- applying a random action sequence then undoing everything restores the original text and selections;
- undo then redo is the identity;
- selections are always sorted, non-overlapping, on char boundaries, and in range;
- `ByteOffset → VisualCol → ByteOffset` round-trips at grapheme boundaries.

### M2 — File I/O (M)

In `fileio`: open with encoding/BOM/line-ending detection, save with atomic write preserving the original format, "Save As", refusing binary files. Round-trip tests over a fixtures folder: UTF-8, UTF-8 BOM, UTF-16 LE/BE with BOM, Windows-1252, CRLF, LF, mixed endings, empty file, file with no trailing newline. Bytes in must equal bytes out when nothing was edited.

### M3 — Editor widget, single file (L)

In `app`: a custom egui widget rendering one `View`.

- Layout cache, painting, gutter with line numbers, current-line highlight, selection rectangles, multiple carets, caret blink.
- Virtual vertical scroll and horizontal scroll; keep primary caret in view after actions.
- Keyboard → `Action` through a keymap table (default bindings matching Notepad++ where sensible; configurable later).
- Mouse: click, shift-click, drag-select, double-click word, triple-click line, autoscroll when dragging past edges.
- Clipboard: copy/cut/paste; with multiple selections, copy joins with newlines; paste distributes one line per selection if the line count matches the selection count.
- IME: handle egui's IME events and show the pre-edit text inline at the caret.
- Open/save via menu and `rfd`; title shows file name and dirty marker; status bar with line:col, encoding, line ending.

**Tests:** `egui_kittest` tests driving keystrokes into the widget and asserting the resulting text and selections.

### M4 — Syntax highlighting (M)

In `syntax`:

- Load syntaxes and themes from `syntect` defaults plus `two-face`. Detect language by extension, then first line (shebang). Language picker in the status bar.
- Highlight incrementally: store a `(ParseState, HighlightState)` checkpoint every 128 lines. On an edit, invalidate checkpoints from the edited line onward and re-highlight forward from the nearest earlier checkpoint, stopping once a recomputed checkpoint equals the old one.
- Highlight the viewport first; continue the rest in the background (a worker thread or time-sliced per frame, never blocking a frame for more than a few ms).
- Above a size threshold (default 10 MB), turn highlighting off and say so in the status bar.
- Theme selection (light/dark), matching the egui visuals.

**Exit check:** typing at the top of a 50,000-line Rust file stays smooth.

### M5 — Tabs and split screen (M)

- `Workspace` with `egui_dock`: tabs of views, drag tabs to split horizontally or vertically.
- "Clone to other view": a second view of the same buffer in the other pane; edits in one appear live in the other, each pane keeps its own carets and scroll.
- New, open (multiple files), close, close others, close all. Prompt to save dirty buffers when their last view closes and on app exit.
- Ctrl+Tab / Ctrl+Shift+Tab tab cycling.

### M6 — Column selection and multi-cursor UI (M)

- Alt+drag and Alt+Shift+arrow keys create a rectangular selection in visual columns. Lines shorter than the rectangle get an empty selection at their end (no virtual space in v1).
- Typing, backspace and delete apply on every line of the selection.
- Rectangular copy produces lines; pasting a rectangular clip into a single caret inserts it as a block on successive lines.
- Ctrl+click adds a cursor; Escape collapses to the primary.

### M7 — Find and replace (M)

- Find panel (docked at bottom or top): find text, replace text, toggles for regex, match case, whole word, wrap around, in selection.
- Search runs on the rope via `core::search` (`regex-cursor`). Literal mode escapes the pattern.
- Find next/previous, count, select all matches (creates a selection per match), highlight all matches within the viewport.
- Replace and Replace All, with `$1` / `${name}` capture expansion. Replace All is one transaction (one undo step).
- Show invalid-regex errors inline.
- Document the dialect difference from Notepad++ (Boost regex): no lookaround or backreferences in v1. Leave a hook for a `fancy-regex` fallback that materializes the text, behind a toggle, as a later addition.

### M8 — Text transforms (S)

Each is an `Action`, applies to every selection (or the current line when a selection is empty), and is one undo step.

- Case: UPPERCASE, lowercase, Title Case, iNVERT cASE. Handle length changes (e.g. `ß` → `SS`); selections must cover the transformed text afterwards.
- Sort lines over the selected lines (or the whole document with no selection): ascending/descending; options for case-insensitive, numeric (integer and decimal), and remove duplicates. If a column-selection rectangle is active, sort whole lines keyed by the text inside the rectangle, as Notepad++ does.
- Cheap extras while here: trim trailing whitespace, duplicate line, move line up/down, join lines.

### M9 — Macros (S)

- Start recording, stop, play back, play back N times, play until end of file.
- The recorder captures `Action`s passed to `apply()` while recording, excluding UI-only actions (opening panels, switching tabs, and other actions not tied to the active buffer). Find-panel searches are recorded as actions with their parameters, so a macro can include "find next X".
- Save named macros to the config dir as JSON; list them in a Macro menu; optional key binding per macro.
- Playback is one undo step. Abort playback on the first `ActionError` and report which step failed.

### M10 — Autosave and session (M)

Notepad++'s autosave is really a backup system; implement that as the default, with write-through as an option.

- **Snapshots (default on):** every N seconds (default 7) after a change, write each dirty buffer's content plus metadata to the app data dir via atomic write. Delete a snapshot when the buffer is saved or closed without saving.
- **Session restore:** on exit, save the list of open files, unsaved snapshots, tab layout, and per-view caret/scroll. On launch, restore them; unsaved buffers come back dirty.
- **Write-through (option, default off):** save dirty buffers that have a path to their own files on a timer and/or when the window loses focus.
- Do all disk writes off the UI thread; never block typing on I/O.

### M11 — Markdown preview (S)

- "Preview" opens a dock tab bound to a `BufferId`, rendered with `egui_commonmark`. Re-render on buffer change, debounced (~200 ms).
- Relative image paths resolve against the file's directory.
- Scroll syncing between editor and preview is out of scope for v1.

### M12 — Diff / compare (L)

- "Compare with…": pick two open buffers (or a buffer against its on-disk version). Opens a dedicated side-by-side compare view, read-only in v1.
- Line diff with `similar`; for replaced line pairs, word-level diff to highlight changed spans.
- Align the two sides by inserting blank filler rows opposite insertions/deletions. Implement this as a diff-specific `DisplayMap` (3.4) for the compare view, not as changes to normal views.
- Synchronized scrolling, next/previous change navigation, summary counts (added/removed/changed).
- Reuse the editor's line layout and painting code rather than a second renderer.

### M13 — Polish (M)

- Settings panel: font and size, tab width, tabs vs spaces, theme, autosave interval, highlight size threshold. Persist with `serde`.
- Editable keymap file.
- External change detection with `notify`: prompt to reload if a clean buffer changed on disk, warn if a dirty one did.
- Recent files list. Drag-and-drop files onto the window to open.

## 6. Testing and verification

- **Core logic** (most of the risk): unit tests and property tests in `core`, no UI needed. Aim for the property tests in M1 to run thousands of random cases.
- **Widget behavior:** `egui_kittest` harness tests that type and click into the editor widget and assert buffer text and selections. Give the editor widget an AccessKit node (role text input, with a label naming the file) so tests and tools can find it; expose caret line:col in the status bar as labelled text so it can be asserted.
- **Visual checks:** `egui_kittest` snapshot tests for the editor in a few fixed states (selection, multi-cursor, highlighting, diff view).
- **Agent-driven checks (optional but recommended):** egui 0.35+ ships an inspection protocol and an `egui_mcp` server that lets an agent read the app's AccessKit tree, send clicks and keystrokes, and take screenshots. Gate eframe's `inspection` feature behind a `dev-inspect` cargo feature, run with `EGUI_INSPECTION=1`, and use it to verify UI milestones end to end. Setup, if the user wants it: `cargo install --git https://github.com/rerun-io/kittest_inspector egui_mcp` then `claude mcp add egui egui-mcp`. Screenshots need the window visible on screen.
- **Performance fixtures:** keep a generated 50,000-line source file and a 100 MB plain text file in a fixtures script (not committed). Check scroll, typing and find-all on both at M3, M4 and M7.

## 7. Working agreements

- Keep `core` free of egui and filesystem access. If you need either there, the design is wrong; ask.
- No `unwrap()` on user data paths (file contents, regex patterns, config). Errors surface in the UI.
- Never write to a user's file except by atomic replace.
- Prefer small, reviewed PR-sized steps within a milestone; commit at each green test run.
- Stop and ask the user: after M0; before changing any decision in section 3; before adding a dependency not listed in section 4; and if a listed crate turns out unmaintained or incompatible with the chosen egui version.
