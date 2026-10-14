# Plan: ochrepad v0.2.0

**Status: done** (2026-10-13). N1–N9 as planned; at the N9 review the
compare view was made editable as step N10a (its sides became editor views
with `Padding`, replacing `SideMap`); N10 prepared the release. No toolbar,
activity bar or icon font, as decided at the review. `CLAUDE.md` is the
current guide.

Written 2026-10-13, after v0.1.0 shipped every milestone of `PLAN_v0.1.0.md`. That document still explains why the core is built the way it is; read its section 3 before working on folding or wrap. This plan covers what changes for 0.2.0. Where it changes a rule in `CLAUDE.md`, the user has approved the change (section 3), and `CLAUDE.md` is updated in the milestone that makes it.

## 1. Goal and scope

0.2.0 adds the features that make ochrepad usable as an everyday editor for a project folder, not just for single files:

1. More line operations: remove empty lines, remove duplicate lines
2. Code folding
3. Word wrap
4. Document map (the zoomed-out strip beside the text)
5. Search across all open files, and Find in Files over a folder
6. Open a folder as a workspace, with a file tree in a left sidebar
7. A light spacing and typography pass, then a UI review

Items 1–6 are the release. Item 7 is the lowest priority: it comes last, and it can slip to 0.2.1 if the features take longer than expected.

### Not in 0.2.0

- Syntax-aware folding (brackets from syntect scopes), `#region` markers
- Word wrap or folding in the compare view
- Replacing text in files that are not open (Replace All works on open documents only)
- Deleting files from the tree, multiple root folders
- A toolbar, an icon font, an activity bar (reconsidered at the UI review, step N9)
- Everything in `CLAUDE.md`'s "Not in scope" list other than word wrap and code folding

## 2. Decisions taken while planning

| Question | Decision |
|---|---|
| Architecture changes for folding and wrap | Approved: rows below the line level in `DisplayMap`, display state on `View` in core, Up/Down through the display map, scroll anchored at (line, row in line, fraction). Section 3. |
| Fold detection | Indentation, plus headings for Markdown. Pure core code, no dependency on the highlighter. |
| Fold state | Per view. Saved in the session and restored. |
| Word wrap | Notepad++ style: one global toggle (View > Word Wrap, saved in settings), wraps at the pane's edge, breaks after whitespace where it can, continuation rows indented to match the line. Home and End go to the row first, then to the line on a second press. |
| Document map | Inside each editor pane, between the text and the scrollbar. Drawn as coloured blocks, not glyphs. |
| Cross-file search | Find All in all open documents, and Find in Files over the workspace folder. Results go in a bottom panel. |
| Cross-file replace | Open documents only, one undo step per buffer. Nothing is written to disk until the user saves. |
| Folder tree | One root. Browse, open, live refresh; new file, new folder, rename, copy path, reveal in the file manager. No delete. |
| Ignore rules | Respect `.gitignore`, `.ignore` and global git excludes using the `ignore` crate (new dependency, approved). Ignored entries are hidden, with a toggle to show them. |
| UI | Last. No toolbar. Spacing and typography only (the tab bar looks too short), then a review with screenshots after the tree and map are in. |

## 3. Architecture changes (approved)

### 3.1 Where we start

`DisplayMap` exists in `core::view`, but only the compare view uses it, through `SideMap`. The editor widget (`editor_view.rs`) assumes document line N is screen row N in at least eight places: the scroll position, `hit_test`, `line_col_at`, caret reveal, the page height, the paint loop, `caret_pos` and `thumbs`. In core, `Motion::Up`/`Down`/`PageUp`/`PageDown` move by document lines in `resolve::vertical`. Folding and wrap both break both assumptions, and the document map needs the row model too, so it is built first and on its own (N2).

### 3.2 Rows are addressed relative to lines

A display row is a `RowPos { line: usize, sub: usize }`: the `sub`-th wrapped row of a visible document line. There is no global row index. With wrap on, a global index would need every line of the file wrapped to compute, which is O(file) on every resize; a 100 MB file cannot afford that.

The new `DisplayMap` trait (in `core::view`, or a new `core::display` module) answers, for a view:

- `rows_in_line(line) -> usize`: 0 for a hidden (folded) line, otherwise 1 + its wrap breaks
- `row_range(RowPos) -> Range<ByteOffset>`: the text on that row
- `row_of(ByteOffset) -> RowPos`
- `advance(RowPos, n: isize) -> RowPos`: step n rows, skipping hidden lines. It jumps over a whole fold in O(log folds), so paging is O(rows moved).
- `visible_line_count()`: line count minus hidden lines, from the fold set. This sizes the scrollbar.

The identity case (no folds, no wrap) must cost nothing measurable. `IdentityDisplayMap` stays as the fast path.

`SideMap` keeps its global-row API. A compare view has filler rows and neither folds nor wraps, so it does not implement the new trait; its methods become inherent methods. `CLAUDE.md`'s line "Line ↔ row mapping goes through `DisplayMap`" is reworded to say this.

### 3.3 Display state lives on `View`, in core

`View` gains a `display: ViewDisplay`:

- `folds: FoldSet`: the folded regions, each stored as a byte range (the end of the header line .. the end of the last hidden line) so it maps through edits like selections do
- `wrap_cols: Option<usize>`: the wrap width in cells, or `None` when wrap is off. The widget sets it each frame from the pane width, the same way it already writes `scroll`.

Core stays free of egui. Wrapping on a monospace grid depends only on the text, the width in cells and the tab width, so `core::wrap::breaks` can compute it, using `core::position` for grapheme widths. The widget and the motions call the same function, which keeps the "one layout" rule: there is exactly one place that decides where a row breaks.

### 3.4 Vertical motions resolve through the view's display map

`resolve::vertical` takes the view's display map. Up and Down move one display row, skipping hidden lines and stepping through wrapped rows. The remembered `VisualCol` becomes a column relative to the row's start. `PageUp(n)`/`PageDown(n)` move n display rows. Macros still record `MoveCursor { motion: Up }`, so a recorded macro behaves the same whether or not the playing view is folded or wrapped. That is the reason this is solved in core and not in the widget.

### 3.5 Scroll is (line, row in line, fraction)

`ScrollPos` gains `first_sub: usize` (with `#[serde(default)]`) beside `first_line` and `line_fraction`, which keeps the integer line anchor that protects big files from f32 precision loss. When wrap reflows because the pane is resized, the top line stays at the top. The vertical scrollbar is proportional to visible lines, not rows: its thumb is slightly off in heavily wrapped files, which is the price of not wrapping the whole file. The docs say so.

### 3.6 Folding rules (enforced in `Editor::apply`)

1. **Carets are never hidden.** After every action, any fold that contains a selection head of that view is unfolded. This one rule covers Find Next into a fold, Go to Line, undo, Left/Right into the hidden text, macro playback, and a column selection dragged across a fold.
2. **Selections can span hidden text.** Copy and delete include the hidden lines, as VS Code does.
3. **Edits unfold what they touch.** After an edit, each view's folds are mapped with `ChangeSet::map_position`. A fold whose range intersects a changed range is removed. Folds that were not touched survive. Typing on a header line is not inside the hidden range, so it keeps the fold.
4. **Nesting is kept.** Folding an outer region hides the inner ones; inner fold state is remembered and comes back when the outer one is unfolded.
5. **Fold actions change view state only.** No undo entry, no revision bump, no dirty flag. They are not recorded in macros (they are UI actions, as in v0.1.0's recorder rules).

### 3.7 Unchanged

Byte offsets stay canonical. `SelectionSet` is unchanged. `Editor::apply` is still the only path that mutates a rope. `LineLayout` still holds the x of every grapheme boundary of a whole document line; a wrapped row draws a slice of it shifted left (section N4), so no second x computation appears.

## 4. Dependencies

| Crate | Where | Why |
|---|---|---|
| `ignore` | `ochrepad-fileio` | Directory walking with `.gitignore`/`.ignore`/global excludes, for the tree and Find in Files. Approved 2026-10-13. Pin it exactly (`=x.y.z`), choosing a release at least two weeks old. |

Nothing else. Fold arrows, tree chevrons and file/folder glyphs are painted with egui shapes, like `logo.rs`. Whether to add an icon font is a question for the N9 review.

## 5. Milestones

Each milestone ends green (fmt, `clippy --all-targets -D warnings`, `cargo test --workspace`), with its docs page updated, a `CHANGELOG.md` `[Unreleased]` entry for user-visible changes, and a commit. Sizes are relative (S/M/L). Run the Linux clippy target after N7 and N8 (paths, file watching).

N1 and N6–N8 do not depend on N2–N5 and can be done in a different order, or interleaved, if that suits.

**Stop for the user** after N2 (with a benchmark comparison), after N3 (to review folding by hand) and at N9 (screenshots).

### N1: Line operations (S)

Four commands from Notepad++'s Edit > Line Operations, as two `Action` variants:

- `RemoveEmptyLines { include_whitespace: bool }`: "Remove Empty Lines", and "Remove Empty Lines (Containing Blank characters)", which also removes lines made only of spaces and tabs
- `RemoveDuplicateLines { consecutive_only: bool }`: "Remove Duplicate Lines" keeps the first occurrence of each line anywhere and does not reorder. "Remove Consecutive Duplicate Lines" only removes a line equal to the one before it.

Behaviour:

- They work on the selected lines (a rectangle counts as its lines), or on the whole document when nothing is selected, as Sort and Trim Trailing Space do.
- Lines compare exactly (case-sensitive, whitespace counts). The last line's missing newline must not make it look different: `a\na` has a duplicate.
- Removing the document's last line also removes the newline before it.
- Each is one undo step. With nothing removed, there is no transaction and the tab stays clean.
- Implement them in `core::transform` next to `sort` (which already has a dedupe pass), reusing `selected_lines`.

Menu: Edit > Line Operations, under a separator after Trim Trailing Space.

Tests: unit tests in `transform` (empty document, all-blank document, CRLF file after load, last line with and without a newline, a selection inside the document, a rectangle). Add both variants to the proptest action generator. `docs/keymap.md` needs an example of each variant (`tests/docs.rs` checks this).

### N2: Display rows, with no visible change (M)

- Add `RowPos`, the new `DisplayMap` trait and `ViewDisplay` (3.2, 3.3) with only the identity case implemented. `wrap_cols` stays `None` and `folds` stays empty.
- Route every row assumption in `editor_view.rs` (3.1) through the view's display map: scroll, wheel, scrollbar thumbs, hit testing, `line_col_at`, caret reveal, page height, the paint loop, carets, the IME rectangle and the gutter.
- Thread the display map into `resolve::vertical` (3.4). For the identity map, Up/Down must give exactly the old results.
- `ScrollPos::first_sub`.
- `SideMap` becomes inherent methods (3.2). The compare view is otherwise untouched.

**Exit:** all existing tests and snapshots pass **unchanged**. Record `--bench` on the 50k-line `.rs` file and the 100 MB `.txt` file before the change and after it, both in release builds. A regression of more than 5% on either needs explaining before going on. Commit those numbers in the PR description; they are also the baseline for N3–N5.

### N3: Code folding (L)

**Detection** (`core::fold`, pure functions over a `Document`):

- Indentation width is the visual width of leading whitespace (tabs expand to `tab_width`), from `core::position`.
- A non-blank line is a fold header when the next non-blank line is indented deeper. Blank lines never start or end a region.
- The region runs from the line after the header to the last line before the next non-blank line indented no deeper than the header. Trailing blank lines are left visible. So a closing `}` at the header's indent stays visible, as in VS Code.
- Markdown buffers fold by heading instead. A `#`–`######` heading folds up to the next heading of the same or a higher level. `#` lines inside fenced code blocks are skipped when the fence can be seen on screen; a fence that starts far above may be missed, which is a documented limitation.
- The strategy (`Indent` or `Markdown`) comes from the buffer's language and is chosen by the app.
- No whole-document pass. Whether a line is a header only needs the next non-blank line, so the gutter asks about visible lines only. The run of blank lines it skips is capped (e.g. 1,000 lines). A region's end is scanned only when the region is folded, at O(region) cost.

**State and rules:** `FoldSet` on the view, with the rules in 3.6. Each `SessionTab` gains `folds: Vec<usize>` (header lines, `#[serde(default)]`), restored only where those lines are still headers. A cloned view starts with no folds.

**Actions** (bindable in `keymap.json`, documented in `docs/keymap.md`). Defaults follow Notepad++; check each against its Shortcut Mapper before binding:

| Action | Notepad++ default |
|---|---|
| `ToggleFold` (the caret's innermost region) | none (fold arrow click) |
| `Fold` / `Unfold` (the caret's region) | Ctrl+Alt+F / Ctrl+Alt+Shift+F |
| `FoldAll` / `UnfoldAll` | Alt+0 / Alt+Shift+0 |
| `FoldLevel(n)` / `UnfoldLevel(n)`, n = 1..8 | Alt+1..8 / Alt+Shift+1..8 |

For indentation folding, the level is the nesting depth of the region. For Markdown, it is the heading level.

**UI:**

- A fold margin to the right of the line numbers: a painted chevron on header lines, always shown for folded headers and shown for the rest while the pointer is over the gutter (VS Code). Clicking it toggles the fold. This is a fixed-width column, added to the gutter-width computation.
- A folded header line ends with a small `⋯` pill. Clicking it unfolds.
- Line numbers jump over hidden lines, which follows from the row model.
- View > Fold All / Unfold All / Fold Current / Unfold Current. A setting to hide the fold margin.

**Tests:**

- Core: detection on Rust, Python, YAML, Allman-style C, tab and space indentation, blank-line edges, Markdown with fences.
- `FoldSet` mapping through edits.
- Proptest: random action sequences on a view with random folds keep the invariants (no head hidden, folds sorted and properly nested, all ranges in bounds), and undo/redo identity still holds.
- Kittest: click a chevron to fold; Down skips the fold; Find Next into a fold unfolds it; session round-trip restores folds.
- One snapshot with a folded region.

**Docs:** new `docs/view.md` (folding now; wrap and the map are added in N4 and N5), linked from `docs/README.md` and the README.

### N4: Word wrap (L)

**Breaking** (`core::wrap::breaks(line, width_cols, tab_width) -> row starts`):

- Break after a run of whitespace when there is one before the limit; otherwise break at the last grapheme boundary that fits. Never split a grapheme. A wide glyph that does not fit moves to the next row.
- Tab stops are counted from the start of the document line, not the row, so a line's visual columns are the same wrapped or not. A tab never straddles a break.
- Continuation rows are indented by the line's leading indent, capped at half the width, so a deeply indented line in a narrow pane still makes progress. The minimum wrap width is 10 cells.

**Rendering:**

- A row draws the slice of the line's `LineLayout` between its two breaks, shifted left by the x of its first byte, plus the continuation indent. Add `LineLayout` helpers for the row's x and the inverse, used by paint, hit test and carets alike, so there is still one x computation.
- Cache row breaks in `LayoutCache` beside the layout, keyed by the same revision plus `wrap_cols`.
- With wrap on, there is no horizontal scrolling: the horizontal bar is hidden and `x_offset` is 0. `wrap_cols` = (content width − document map width) / cell width.
- The line number is drawn on a line's first row only.

**Motions:**

- Up/Down/PageUp/PageDown move by row (3.4).
- Home goes to the row's start (after the continuation indent); a second press does the existing smart home on the line. End goes to the row's end; a second press goes to the line end. On a line's first row, Home behaves exactly as it does now. Add these as new `Motion` variants (`RowHome`, `RowEnd`) and keep `Home`/`LineEnd` unchanged, so old macros and keymaps keep their meaning; the default bindings for Home/End switch to the row motions when wrap is on.
- **Caret affinity at a break:** a position exactly at a break is drawn at the start of the next row. Start without affinity. If End on a row that was broken mid-word lands visibly wrong in testing, add a one-bit affinity to `Selection` and check with the user first, since it touches the selection invariants.

**Column selection:** rectangles stay in (document line, visual column of the line). An Alt+drag hit test turns (row, x) into the line's visual column, and painting follows the per-line selections, so this should mostly work as is. Test it, but don't redesign it.

**Settings:** `word_wrap: bool` in `AppSettings` (default off), and View > Word Wrap as a checkable menu item. There is no default shortcut (Notepad++ has none).

**Performance:** only visible lines are wrapped. A pathological single line of many megabytes is as slow as it is today; that stays out of scope.

**Tests:**

- Core: breaks with tabs, CJK, emoji, combining marks, no whitespace, all whitespace, width smaller than the indent.
- Motions across rows and folds together.
- Proptest: positions round-trip through `row_of` / `row_range`.
- Kittest: toggling wrap; resizing reflows and keeps the top line; clicking on a continuation row; Home/End twice.
- A snapshot with wrapped lines.
- Bench with wrap on in release, compared with the N2 baseline.

**Docs:** `docs/view.md`. Remove word wrap from `CLAUDE.md`'s "Not in scope".

### N5: Document map (M)

- A strip inside the editor widget between the text and the vertical scrollbar, default 100 px wide. View > Document Map (`document_map: bool` in settings, default on). It is hidden automatically when the pane is too narrow to fit it beside 40 columns of text.
- **Scale:** each display row is about 2 px tall and each cell about 1 px wide, clipped to the strip. It shows the same rows as the editor: folded lines are hidden and wrapped lines show their rows.
- **Drawing:** for each row, a rectangle per run of non-blank characters, coloured with that span's highlight colour at reduced alpha. Lines without spans yet (the highlighter is catching up or is off) use the weak text colour. Build one `Mesh` for the strip and cache it by (buffer revision, highlight generation, first map row, size, theme), so a still frame redraws nothing. Take spans through the existing `line_spans`, whose `max_catch_up` limit keeps far-away lines cheap.
- **Scrolling:** when the document has more rows than the map can show, the map scrolls in proportion to the editor, as VS Code's does. A translucent rectangle marks the rows visible in the editor.
- **Mouse:** drag the rectangle to scroll; click elsewhere to centre the editor on that row; the wheel over the map scrolls the editor.
- **Stretch goal:** full-width ticks for find matches and the selection, in their usual colours.

**Tests:** kittest for click-to-scroll and dragging the viewport rectangle; a snapshot. Bench with the map on in release, against the N2 baseline: the map must not add more than about 1 ms per frame while scrolling the 50k-line file.

**Docs:** `docs/view.md`.

### N6: Search across open documents (M)

- **Find panel:** add Find All in Current Document and Find All in All Open Documents (Notepad++'s names), and Replace All in All Open Documents in replace mode. "In selection" does not apply to the all-documents commands; the option is disabled while they are chosen.
- **Searching:** clone the open buffers' `Document`s (rope clones are O(1)) and search them on a background thread with the existing `Searcher`, streaming results back, so a 100 MB open file never blocks a frame. Cap the results at 10,000 matches and say so in the panel. Check first that `Document` is cheaply cloneable; if it is not, add that in `core::text`.
- **Results panel:** a resizable bottom panel, "Search Results", above the find panel. Searches stack with the newest first and expanded; older ones are collapsed (keep the last 10, plus a Clear button). Each search is grouped by file and shows the line number and the line's text with the matched spans tinted in the find-match colour. Click or Enter on a result focuses the file's tab and selects the match.
- **Stale results:** each result keeps the buffer's revision from when it was found. If the buffer has changed since, clicking it goes to the same line and selects the nearest match on that line, or just the line if there is none. Results are not re-mapped live.
- **Replace All in All Open Documents:** a confirmation with the match and file counts, then `Action::ReplaceAll` applied through one view of each buffer: one undo step per buffer, buffers become dirty, nothing is saved. Not recorded in macros (it acts outside the active view).
- **Tests:** kittest across three open buffers (one unsaved); a stale result after an edit; replace-all undo per buffer.
- **Docs:** `docs/find-and-replace.md`.

### N7: Folder workspace and file tree (L)

**Opening:**

- File > Open Folder… (rfd folder picker) and File > Close Folder.
- `ochrepad <dir>` on the command line, and dropping a folder on the window.
- File > Recent Folders, kept apart from recent files.
- The root, the expanded folders, the sidebar width and the sidebar's visibility are saved in the session (`#[serde(default)]`).

**Listing** (`ochrepad-fileio`, new module `tree`, the only place that uses `ignore`):

- `list_dir(dir, show_ignored) -> Vec<Entry>` uses `ignore::WalkBuilder` with depth 1 and parent ignore files enabled, and does not follow symlinks.
- Folders come first, then natural, case-insensitive order.
- It runs on a background reader thread with a request/response channel, never on the UI thread. A slow network drive must not freeze the window. (`CLAUDE.md` says "never block a frame on I/O"; opening a file is still synchronous today, and that is not changed here.)
- Folders are listed lazily, when they are expanded.

**Sidebar:**

- `Panel::left` outside the dock, resizable, default 240 px, toggled from View > Sidebar. Choose its shortcut while implementing, avoiding Notepad++'s defaults.
- Rows about 22 px tall, with painted chevrons and file/folder glyphs.
- Keyboard: arrows, Left/Right collapse and expand, Enter opens. Single click selects; double-click or Enter opens.
- The active editor's file is highlighted in the tree. A "Reveal Active File" command expands the tree to show it.
- A toggle in the sidebar header shows ignored files.
- If the root disappears or can't be read, the sidebar says so and offers Close Folder.

**Live refresh:** the existing `notify` watcher (`app/external.rs`) also watches the expanded folders, non-recursively; it already watches open files' folders the same way. An event re-lists that folder, debounced to about 200 ms. Recursive watching is avoided because it is expensive on Linux for trees like `node_modules`.

**File operations** (context menu). All disk changes go through the worker thread, as new job kinds:

- **New File:** created with `create_new` (fails if the name exists), then opened in a tab.
- **New Folder.**
- **Rename:** inline editing in the tree row. It refuses if the target exists. Open buffers under the old path get their new path, the tab titles and the watcher follow, and session paths update.
- **Copy Path / Copy Relative Path.**
- **Reveal in File Manager:** `explorer /select,<path>` on Windows and `xdg-open <parent>` on Linux, through `std::process::Command`.
- **Find in Folder…** is added by N8.

**Tests:**

- fileio tests on temp dirs: sort order, `.gitignore` and nested ignores, the show-ignored toggle, a symlink not followed.
- Kittest: open a folder, expand it, double-click a file to open it, rename an open file and check the tab title and buffer path.
- A real-time watcher test (`wait_for`): a file created outside the app appears in the tree.

**Docs:** new `docs/workspace.md`. Check the Linux CI run afterwards.

### N8: Find in Files (M)

- **Find panel:** a "Find in Files" mode. Directory (defaults to the workspace root, or the folder chosen from the tree's context menu), filters (`*.rs;*.toml`; empty means all), Include Subfolders, Include Ignored Files.
- **Searching** runs on a background thread:
  - walk with `ignore::WalkBuilder`;
  - for files that are open, search the buffer's text, not the disk;
  - otherwise read with `fileio::load`, so encodings and BOMs are handled. Binary and undecodable files are skipped silently, and files over 50 MB are skipped with a count in the summary;
  - search with `Searcher`.
- **Results** stream into the N6 results panel with a progress line ("1,240 files searched") and a Stop button. The same 10,000-match cap applies. Clicking a result opens the file if needed and selects the match. Find only: there is no on-disk replace.
- **Tests:** a temp folder with mixed encodings, an ignored folder, a binary file, and one file open with unsaved changes (the results must show the buffer's text). Stopping mid-search.
- **Docs:** `docs/find-and-replace.md`, `docs/workspace.md`. Check the Linux CI run afterwards.

### N9: Spacing and typography pass, then a UI review (S–M)

- **Spacing tokens** in `theme.rs` (a small named set: 4, 8, 12, 16, 24), used for panel margins, menu item padding, the find panel, the results panel, the status bar and the sidebar, in place of scattered literals.
- **Tab bar:** try 36 px (from 32), with more horizontal padding in tabs and a clearer close-button hover.
- **Status bar:** segments with separators, consistent padding, clickable segments (encoding, line ending, language) that look clickable on hover.
- **Typography:** check UI font sizes and weights (menus, the settings dialog, panel headers in strong text, metadata in weak text).
- **Review:** render the full app at `with_pixels_per_point(1.5)` in dark and light (main editor, sidebar open, results panel, compare view), using the throwaway test from `CLAUDE.md`'s Testing section, and go through them with the user. Decide then whether 0.2.0 or a later version gets icons, an activity bar or a toolbar.
- Regenerate the snapshots and look at every PNG before committing.

### N10: Release 0.2.0 (S)

- **`CLAUDE.md`:**
  - The Layout table: the new core modules (`fold`, `wrap`, display), fileio `tree`, and the app's sidebar and results panel.
  - The architecture rules: `RowPos` and `DisplayMap`, scroll as (line, row in line, fraction), vertical motions through the display map, the folding rules in 3.6.
  - The settled decisions: fold detection, wrap behaviour, the document map, `ignore`.
  - "Not in scope": remove word wrap and code folding.
  - Point to this file as the latest plan.
- **Docs:** `docs/README.md` and the README feature list.
- **Release:** follow `CLAUDE.md`'s Releasing steps: version 0.2.0 in both places in the root `Cargo.toml`, the `CHANGELOG.md` section with its compare link, tag, draft release, `cargo publish --workspace --dry-run`.

## 6. Testing and performance

- The v0.1.0 rules still apply (`CLAUDE.md` Testing). The proptest generator gains the new actions, plus random folds and a random `wrap_cols` on the view, so motions are fuzzed in every display mode.
- **Performance budget:** record the N2 baseline (`--bench`, release, both fixtures). Folding, wrap and the map must each stay within 10% of it for scroll and typing frame times, measured in the same milestone. Find in Files is checked by hand on a large tree (e.g. a checkout of the Rust compiler) for responsiveness while the search runs, not for speed.
- **Linux:** N7 and N8 touch paths, watching and process spawning. Run the Linux clippy target locally, and check CI on the PR.

## 7. Risks

| Risk | Mitigation |
|---|---|
| N2 refactor subtly changes scrolling or hit testing | Unchanged tests and snapshots are the exit criterion; bench before and after. |
| Caret at a wrap break looks wrong | Start without affinity; add a one-bit affinity only if needed, after asking (N4). |
| Indentation folding is wrong for odd files (multi-line strings, misindented code) | It's a known limitation in the docs. Syntax-aware folding is the follow-up if users ask. |
| Map or wrap costs too much in big files | Viewport-only work everywhere; mesh cache for the map; budget in section 6. |
| Rename in the tree with files open loses track of buffers | Rename goes through the worker, and the app updates buffer paths, the watcher and the session in one place; kittest covers it. |
| A slow or hung network folder | All listing and Find in Files reading run on background threads with cancellation. |
