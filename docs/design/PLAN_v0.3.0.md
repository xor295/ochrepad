# Plan: ochrepad v0.3.0

**Status: done** (2026-10-14). P1–P8 as planned, then closing brackets as
you type; P9 prepared the release. Beyond the plan, the same
release has bookmarks (marked "not now" in section 1), change history,
Replace in Files, the Function and Document Lists, sessions and more of
Notepad++'s editing commands (see `CHANGELOG.md`). The command palette
was not built. `CLAUDE.md` is the current guide.

Written 2026-10-13, after 0.2.0. `CLAUDE.md` is the current guide; read its
architecture rules before starting. This plan adds the everyday editing
features ochrepad still lacks next to Notepad++, and two things that set it
apart: working with logs and huge files, and undo that survives a restart.
Where it changes a rule in `CLAUDE.md`, section 3 says so, and `CLAUDE.md` is
updated in the step that makes the change.

## 1. Goal and scope

1. **Editing basics:** Go to Line, Tab/Shift+Tab indent and unindent,
   toggle comment, matching brackets.
2. **Per-document indentation, line endings and encoding:** detected on open,
   shown and changed from the status bar, with convert commands.
3. **Zoom** and **Show All Characters**.
4. **Text tools:** JSON format/minify/validate, Base64 and URL encode/decode,
   hashes.
5. **Undo across sessions.**
6. **Log tools:** filter view, highlight rules, follow mode. They work on any
   file.
7. **Huge files:** a read-only viewer for files above a size threshold,
   memory-mapped with a background line index, so a multi-gigabyte log opens
   at once and never exhausts memory.

Items 1–5 are smaller and come first. Items 6 and 7 are the release's
headline.

### Not in 0.3.0

- Bookmarks, a command palette, a toolbar, plugins
- Local history (saved versions of a file); undo across sessions only
- A CSV/TSV column view
- Editing files above the huge-file threshold
- UTF-16 files above the threshold (they are refused with a message)
- Skipping brackets inside strings and comments when matching (section 5,
  P1)

## 2. Decisions taken while planning

| Question | Decision |
|---|---|
| Go to Line | Yes (Ctrl+G); it is cheap. |
| Toggle comment | Ctrl+/ for line comments, Ctrl+Shift+/ for a block comment. (Shift+/ alone types `?`.) Comment markers come from a built-in table of common languages; syntect can read them from syntax metadata, but the two-face syntaxes we use do not include it. |
| Matching brackets | Yes: highlight the pair at the caret, Ctrl+B to jump (Notepad++'s key), Ctrl+Alt+B to select up to the match. |
| Tab / Shift+Tab | Tab with a selection over several lines indents them; Shift+Tab unindents the selected lines or the caret's line. |
| Ctrl+Tab | Already cycles tabs in tab order (Notepad++'s default); unchanged. |
| Indentation and line endings | Per document: detected on open (tabs or spaces, indent width; mixed indentation flagged like mixed line endings), shown in the status bar, and changed there for that document, with Convert Indentation and Convert Line Endings. The preferences stay the default for new files. |
| Encodings | The status bar's encoding offers Reopen with Encoding and Save with Encoding. |
| Zoom | One zoom for every editor's text (not the menus): Ctrl+wheel, Ctrl+plus/minus, Ctrl+0 to reset. Remembered. |
| Show All Characters | Notepad++ style: spaces, tabs and line endings drawn as symbols, plus control and invisible characters (NUL, ESC, no-break space, zero-width characters, BOM, bidi controls) made visible. |
| Bookmarks, command palette | Not now. |
| Text tools | JSON format/minify/validate (our own formatter, key order kept), Base64 and URL encode/decode, MD5/SHA-1/SHA-256 hashes (new dependencies). No CSV view. |
| Undo across sessions | Undo and redo history only, restored when the file on disk is unchanged since. |
| Huge files | Read-only above a threshold (default 256 MB): memory-mapped, line index built in the background. New dependency `memmap2`. Editable below the threshold, as now. |
| Log tools | Filter view, follow mode and highlight rules, for any file. |

## 3. Architecture changes

### 3.1 Per-document tab width and indentation

The tab width is a single editor setting today, read from
`editor.settings.tab_width` wherever columns are computed. It becomes a
property of the buffer: `Buffer::indent: Indent { use_spaces, width }`,
taken from the preferences for a new file and detected for an opened one.
Every caller that computes visual columns (resolve, display map, layout,
fold, wrap, the widget, the compare view) asks the buffer instead. This is a
mechanical change touching many call sites; it comes first in P2, on its
own, with no visible change, like N2 did for rows.

### 3.2 The huge-file viewer is a separate widget

The editor widget, the display map, search and every action work on core's
`Document`, a rope that holds the whole text in memory. A memory-mapped file
is not a rope, and teaching all of core to read from either would touch every
module for a read-only feature. Instead:

- **`ochrepad-fileio::bigfile`** owns the map (`memmap2`) and a sparse line
  index: the byte offset of every 1024th line, built on a background thread
  with `memchr`-style scanning and published as it grows. Finding any line is
  one index lookup plus a short forward scan, and the index for a 4 GB,
  40-million-line log is about 300 KB.
- **`ochrepad::viewer`** is a read-only widget for one such file. It draws
  visible lines with the same `LineLayout` and `paint_line_text` the editor
  uses (the one-layout rule holds), and supports scrolling, selecting with
  the mouse and keyboard, copying, Go to Line, find (through core's
  `Searcher`, which gains a byte-slice entry point; the map is one contiguous
  slice), and the log tools.
- A viewer is a dock tab like an editor (`Tab::Viewer`). It is not a
  `Buffer`, so it is not in Find All, autosave or compare; the session
  reopens it as a viewer.

The log tools (P6) are built once, on line numbers and a line-text accessor,
and used by both the editor (as hidden lines in the display map, like folds)
and the viewer (as a list of shown lines).

### 3.3 Undo history on disk

`History`, `Transaction` and `ChangeSet` gain serde derives (the time used
for typing coalescing is not saved). The app writes a buffer's history
through the `fileio` worker, as one file per document in the data folder,
keyed by the file's canonical path (or the backup id for an unsaved
buffer), together with a SHA-256 of the text the history ends at. On open,
the history is restored only if that hash matches the loaded text. Each
saved history keeps the newest steps that fit a size cap (default 2 MB per
document); the oldest are dropped.

## 4. Dependencies (to approve)

| Crate | Where | Why |
|---|---|---|
| `memmap2` | fileio | Memory-mapping huge files (P7). |
| `md-5`, `sha1`, `sha2` (RustCrypto) | app (hashes), fileio (`sha2` for undo history) | Hash tools (P4) and checking a history matches its file (P5). |
| `memchr` | fileio | Fast line counting while indexing huge files (P7). Already in the dependency tree (through `regex`); becomes a direct dependency. |

Each is pinned exactly, at a release at least two weeks old.

## 5. Milestones

Each ends green (fmt, clippy `-D warnings` for both targets after P2 and
P7, `cargo test --workspace`), with docs, a changelog entry and a commit.
Stop for the user after P2 (status bar controls), P6 (log tools) and P7
(huge files, with measurements).

### P1: Editing basics (M)

- **Go to Line** (Ctrl+G): a small dialog that takes `line` or `line:column`.
  It resolves to an `Action::GoToLine { line, column }` (recordable), which
  unfolds the target like any caret move does.
- **Tab / Shift+Tab:** `InsertTab` with a selection spanning lines indents
  each selected line (by the document's indent); a selection within one line
  is replaced, as now. New `Action::Unindent` (Shift+Tab) removes one indent
  level from each selected line, or the caret's line. Both are one undo step,
  and they keep the selections on the same text.
- **Toggle comment:** `Action::ToggleLineComment` (Ctrl+/) comments the
  selected lines, or the caret's line, at their common indent, or uncomments
  them if all are commented. `Action::ToggleBlockComment` (Ctrl+Shift+/)
  wraps the selection or unwraps it. The markers come from a table in
  `ochrepad-syntax` keyed by language name (Rust, C/C++, C#, Java, JS/TS,
  Go, Python, Ruby, shell, PowerShell, SQL, HTML/XML, CSS, YAML, TOML, INI,
  Lua, Haskell, Lisp, Markdown (HTML comments), and the rest of the common
  ~40). For a language without markers, the status bar says so. The core
  action takes the markers as data, so core stays free of syntect.
- **Matching brackets:** with the caret next to `(` `[` `{` (or the closing
  ones), both brackets are outlined in the accent colour, and an unmatched
  one in the error colour. Ctrl+B jumps to the match; Ctrl+Alt+B selects up
  to it. The search is plain text, bounded (100,000 bytes each way), and
  ignores whether a bracket is inside a string or comment. Skipping those
  needs scope information the highlighter does not keep today; it is a
  follow-up if it proves annoying.
- Tests: core unit tests for each action (tabs, empty lines, several
  selections, a rectangle); proptests for the new actions; docs
  examples in `keymap.md`.

### P2: Per-document indentation, line endings and encoding (M)

1. **Per-buffer tab width (no visible change):** section 3.1. Benchmarks
   and tests unchanged.
2. **Detection:** on open, the first 10,000 lines decide tabs or spaces
   and the indent width (the most common step between indent levels: 2, 4
   or 8). A file with both tab- and space-indented lines is *mixed*,
   flagged in the status bar like mixed line endings.
3. **Status bar:** the indentation (`Spaces: 4` / `Tabs: 4` / `Mixed`), the
   line ending and the encoding become menus:
   - Indentation: indent using spaces or tabs, width 2/4/8 (for this
     document), Convert Indentation to Spaces / to Tabs (one undo step;
     only leading whitespace changes).
   - Line ending: Windows (CR LF), Unix (LF), Macintosh (CR). Choosing one
     changes how the document is saved and marks it unsaved, as Notepad++'s
     EOL conversion does.
   - Encoding: Reopen with Encoding ▸ (decode the file again; refused for
     an unsaved document) and Save with Encoding ▸ (UTF-8, UTF-8 with BOM,
     UTF-16 LE/BE, and the common single-byte encodings), which marks it
     unsaved. Characters the new encoding cannot represent are listed before
     saving, never replaced silently.
4. Docs: `docs/file-handling.md`.

### P3: Zoom and Show All Characters (M)

- **Zoom:** a factor on the editor font size (10%–400%), from Ctrl+wheel
  over an editor, Ctrl+plus/minus and Ctrl+0. It applies to every editor,
  the compare view and the viewer, not the menus. Saved in settings; the
  status bar shows the percentage when it is not 100%.
- **Show All Characters** (View menu; a shortcut to be chosen): spaces as `·`, tabs as `→` across their cells, line ends as a
  small pill naming the document's line ending (`CRLF`, `LF`, `CR`), and
  each control or invisible character drawn as a tinted box with its
  abbreviation (`NUL`, `ESC`, `NBSP`, `ZWSP`, `BOM`, `RLO`, ...). Characters
  of zero width are drawn as a thin bar between cells, taking no space, so
  columns do not move. All of it is drawn from the line's `LineLayout`.
  Mixed line endings are normalized on load, so every line shows the
  document's ending; the existing status-bar note still says the file had
  mixed endings.

### P4: Text tools (M)

Edit > Text Tools:

- **JSON:** Format (2 or 4 spaces, or tabs, per the document), Minify,
  Validate. They work on the selection, or the whole document without one.
  An own tokenizer: object key order and number text are kept exactly, and
  strings are copied byte for byte. Invalid JSON is reported with its line
  and column, and the caret moves there. `Action::FormatJson { indent }` and
  `Action::MinifyJson` (one undo step each); Validate is not an edit.
- **Base64** encode/decode and **URL** encode/decode (percent-encoding) of
  each selection. Decoding to bytes that are not UTF-8 is refused with a
  message. Actions, so macros can use them.
- **Hashes:** MD5, SHA-1, SHA-256 of the selection (or the document), as
  UTF-8 bytes, shown in a small dialog with Copy. Not an edit.
- Docs: a new `docs/text-tools.md`; `keymap.md` examples for the new
  actions.

### P5: Undo across sessions (M)

Section 3.3. The history is written when a document is saved, closed or
backed up, and at exit; restored when a named file is reopened (unchanged
on disk since) or an unsaved backup is restored. Coalescing restarts (the
first edit after a restore starts a new step). A setting turns it off and a
command clears a document's history. Tests: round trip through the session
tests; a file changed on disk in between discards the history; the size cap
drops the oldest steps.

### P6: Log tools in the editor (L)

- **Filter view:** a filter bar (Search > Filter Lines…, Ctrl+Shift+L to be
  checked against Notepad++) with a pattern (the find panel's regex
  options), Show matching / Hide matching, and a count. Lines that do not
  pass are hidden through the display map, like folds; the gutter keeps the
  real line numbers. Edits re-filter after a pause. Carets in hidden lines
  follow the folding rule (they bring the line back) only while editing;
  with a filter on, Up/Down move between shown lines. The filter is per
  view and not saved.
- **Highlight rules:** a list of (pattern, colour, whole line or match only)
  in the settings, applied to files whose name matches a glob (default
  `*.log`, with ERROR/FATAL red, WARN amber, DEBUG/TRACE dimmed). A small
  editor for the rules in the preferences. Drawn through the widget's
  `RowStyle` hook (from the compare view), on visible lines only.
- **Follow mode** (View > Follow, or a status bar toggle): when the file
  grows on disk, a clean document appends the new text (reading only the
  added bytes, after checking the old end still matches) and stays scrolled
  to the end if it was there. A file that shrinks or changes before the end
  (rotation, truncation) is reloaded. Uses the existing watcher; appends are
  not undo steps.
- Docs: a new `docs/logs.md`.

### P7: The huge-file viewer (L)

Section 3.2.

- **Opening:** a file above the threshold (setting, default 256 MB) opens in
  a viewer tab instead of an editor, without reading it: the first screen
  shows at once, and the index grows in the background (the scrollbar and
  line count follow; Go to Line past the indexed part waits for it).
- **Text:** UTF-8 and ASCII as is; single-byte legacy encodings decoded per
  line as drawn; UTF-16 refused with a message. Lines longer than 10,000
  bytes are shown cut, with a marker. CR LF and CR endings are recognised
  when indexing, not normalized.
- **Using it:** scrolling, the mouse and keyboard selection, copy (capped at
  64 MB), Go to Line, the find panel (Find Next/Previous, Count, Find All
  into the results panel) through a byte-slice `Searcher`, and View > Open
  as Editable for a file that fits in memory (with a warning).
- **Robustness:** the file changing under the map (Windows locks a mapped
  file against truncation; Linux does not) must never crash: the viewer
  re-maps on a size change and treats a shrink as a reload; reads past the
  end are clamped.
- **Measurements** (in the PR, release build): time to first screen,
  index time and peak memory for 100 MB, 1 GB and 4 GB generated logs, and
  scroll frame times; the user compares against Notepad++ on the same
  files.
- Docs: `docs/logs.md`, `docs/file-handling.md`.

### P8: Log tools in the viewer (M)

The P6 tools on the viewer: the filter builds its list of shown lines in
the background as the index grows (with a count and a progress line);
highlight rules on visible lines; follow mode re-maps and indexes the new
tail.

### P9: Review and release (S)

Screenshots at 1.5× of the new UI (status bar menus, Show All Characters,
filter bar, viewer), reviewed with the user. Then `CLAUDE.md` (per-buffer
indent, the viewer, the history files, new settings, the "Not in scope" list
loses huge files), docs, version 0.3.0, the changelog section, and the
release steps.

## 6. Testing and performance

- The rules in `CLAUDE.md` still apply. The proptest generator gains the new
  actions and a random per-buffer indent.
- Benchmarks after P2 and P6 must stay within 10% of 0.2.0 on both fixtures.
- The perf fixture script gains 1 GB and 4 GB log generators for P7 (not
  committed; generated on demand).
- Huge-file tests use small files with the threshold lowered.

## 7. Risks

| Risk | Mitigation |
|---|---|
| The per-buffer tab width refactor misses a call site | Done on its own first (P2.1), tests and benchmarks unchanged as the exit; grep for `settings.tab_width` must find nothing afterwards. |
| A mapped file is truncated by another program (Linux) and reading it faults | Re-map on any size change, check the length before every read, treat a shrink as a reload; a test truncates a mapped file on Linux CI. |
| Filter view on a large editable document is slow | Matching runs on a background thread over a rope clone; the view shows the old filter until the new one is ready. |
| Comment markers missing or wrong for a language | The table is data with a test per entry; unknown languages say so instead of guessing. |
| A restored undo history no longer matches the file | Restored only when the SHA-256 of the loaded text matches the one saved with the history. |
