# Changelog

All notable changes to ochrepad. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/) (while below 1.0, a minor version
adds features and a patch version fixes bugs).

Each release's section becomes its GitHub release notes, so write it for
people downloading the release.

## [Unreleased]

### Added

- Go to Line (Ctrl+G): a line, or `line:column`.
- Tab indents the selected lines when a selection spans lines; Shift+Tab
  unindents the selected lines or the caret's line. Edit > Indent and
  Unindent do the same.
- Toggle Comment (Ctrl+/) and Toggle Block Comment (Ctrl+Shift+/), with the
  comment markers of about 100 languages.
- Matching brackets: the bracket beside the caret and its match are
  outlined (in red when there is no match). Ctrl+B jumps to the match and
  Ctrl+Alt+B selects up to it.

## [0.2.0] - 2026-10-13

### Added

- Code folding. Blocks fold by indentation, and Markdown files fold by
  heading. Click the chevron beside a line number, or use Notepad++'s
  shortcuts: Ctrl+Alt+F and Ctrl+Alt+Shift+F for the block at the caret,
  Alt+0 and Alt+Shift+0 for everything, Alt+1 to Alt+8 (with Shift to
  unfold) for one level. The same commands are in the View menu. Folds are
  kept with the session. See docs/view.md.
- Word wrap (View > Word Wrap). Long lines wrap at the edge of the pane,
  after spaces where possible, and their continuation rows are indented
  like the line. Up and Down move by row; Home and End go to the start or
  end of the row first, then of the line.
- Document map: a zoomed-out view of the document on the right of each
  editor. Drag its marker to scroll, or click to jump there. View >
  Document Map turns it off.
- Folders: File > Open Folder (or `ochrepad <folder>`, or dropping a folder
  on the window) shows the folder's files in a sidebar (View > Sidebar,
  Ctrl+Shift+E). Files ignored by `.gitignore` are hidden. The tree follows
  changes on disk, and its menus create files and folders, rename (F2),
  copy paths and reveal files in the file manager. Renaming an open file
  keeps its tab. The folder comes back with the session; File > Recent
  Folders lists the last ten.
- Find All in Current Document and Find All in All Open Documents list every
  match in a new Search Results panel; click a result to go to it. Replace
  All in All Open Documents replaces in every open document after asking,
  one undo step per document.
- Find in Files (Search > Find in Files…, Ctrl+Shift+F): searches the files
  in a folder, with filters such as `*.rs; *.toml`, subfolders, and
  `.gitignore`d files left out unless asked for. Files in any encoding are
  found; binary files and files over 50 MB are skipped. Open files are
  searched with their unsaved changes. Results stream into the Search
  Results panel, and Stop ends a long search.
- Edit > Line Operations: Remove Empty Lines, Remove Empty Lines (Containing
  Blank characters), Remove Duplicate Lines and Remove Consecutive Duplicate
  Lines. They work on the selected lines, or on the whole document when
  nothing is selected.
- Smart highlighting: selecting text highlights its other occurrences, as in
  Notepad++.

### Changed

- The compare view is editable: each side is a full editor (typing, undo,
  multiple carets, syntax highlighting), the two sides scroll together, and
  the differences update a moment after you stop typing. A side showing the
  saved file stays read-only.
- Ctrl+F and Ctrl+H select the search text, so you can type a new search
  straight away even when the find panel is already open.
- The find panel is more compact: Find All is one menu, Replace and In
  Files are toggles, and × closes it. The tab bar is a little taller.
- The gutter is one column wider, for the fold margin.

### Fixed

- Symbols such as ✓ and ✗ showed as boxes. A system symbol font (Segoe UI
  Symbol on Windows, DejaVu Sans Mono on Linux) is now used for characters
  the editor font lacks.

## [0.1.0] - 2026-10-10

The first release, for Windows and Linux.

### Editing

- A fast custom editor that stays smooth in files of 100 MB, with a line
  number gutter, current-line highlight, smart Home, auto-indent, and undo
  that takes back a run of typing in one step.
- Column selection (Alt+drag, Alt+Shift+arrows) and multiple carets
  (Ctrl+click, Ctrl+Alt+Up/Down), with block copy and paste.
- Find and replace with regular expressions, whole word, match case, in
  selection, `$1` replacements, Count, Select All and Replace All.
- Macros: record, play once, N times or to the end of the file; save them
  with a name and a shortcut.
- Case conversion, line sorting (including by a column), duplicate, move,
  join lines and trim trailing whitespace.
- Clicking a line number selects the line.

### Files and windows

- Tabs and split screen, with several views of one file kept in sync.
- Syntax highlighting for over 180 languages, incremental, with a language
  picker in the status bar.
- Encodings (UTF-8, UTF-16, legacy code pages), BOMs and line endings are
  detected and preserved; saves are atomic; binary files are refused.
- Backups of unsaved work every few seconds and session restore; optional
  automatic saving.
- Notices when another program changes or deletes an open file.
- Markdown preview with images, loaded only from this machine.
- Side-by-side compare with aligned lines and changed words highlighted.

### Settings

- Preferences for fonts, tabs or spaces, theme, backups and the
  highlighting size limit; an editable keymap; recent files; drag and drop.
- A warm light and dark theme with ochre accents.

[Unreleased]: https://github.com/xor295/ochrepad/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/xor295/ochrepad/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/xor295/ochrepad/releases/tag/v0.1.0
