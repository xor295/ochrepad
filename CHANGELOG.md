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
- Each document has its own indentation. Opening a file detects tabs or
  spaces and the indent width from its text, and the status bar flags files
  that mix the two.
- The status bar's indentation, line ending and encoding are menus: indent
  with spaces or tabs and set the width for this document, convert the
  indentation, change the line endings, reopen a file with another encoding,
  or save it in another encoding (characters that encoding cannot store are
  listed, and nothing changes).
- Zoom: Ctrl+plus, Ctrl+minus and Ctrl+0, or Ctrl+mouse wheel, make the
  editors' text larger or smaller (10% to 400%) without changing the menus.
  The status bar shows the zoom while it is not 100%.
- View > Show Symbol, as in Notepad++: spaces and tabs, line endings,
  non-printing characters (zero-width and bidi characters, the BOM, unusual
  spaces), Show All Characters (Ctrl+Shift+8), indent guides and the wrap
  symbol. Hovering a special character's box names it.
- A vertical edge line after a chosen column (Preferences).
- Log tools, for any file ([docs/logs.md](docs/logs.md)):
  - Search > Filter Lines (Alt+Shift+L) shows only the lines that match a
    pattern, or hides them. The gutter keeps the real line numbers, the
    search runs in the background, and edits filter again after a pause.
  - Highlight rules colour whole lines or matches, or dim lines, in files
    named `*.log` and `*.log.*` by default: errors red, warnings amber,
    debug output dimmed. They are edited in Preferences.
  - View > Follow (tail -f) adds what another program appends to the
    file as it grows, without asking and without an undo step; a rotated
    or truncated file is reloaded.
  - The same tools in the viewer for large files: the filter searches the
    whole file in the background and fills the view in as it goes (and
    goes on as the file grows), highlight rules colour the lines on screen,
    and Follow keeps the end in view.
- Undo survives closing a file and restarting ochrepad: each file's undo
  history (up to about 2 MB of the newest changes) is kept and comes back
  when the file is opened again unchanged, and with unsaved work restored
  from a backup. Edit > Clear Undo History forgets it; a preference turns it
  off.
- Edit > Text Tools: Format, Minify and Validate JSON (object key order,
  numbers and strings are kept exactly; invalid JSON is reported with its
  line and column, and the caret moves there), Base64 and URL
  encode/decode, and a dialog with the MD5, SHA-1 and SHA-256 hashes of the
  selection or the document. They work on each selection, or on the whole
  document when nothing is selected, and can be bound to keys and recorded
  in macros.
- Notepad++'s line commands: Delete Line (Ctrl+Shift+L), Transpose Line
  (Ctrl+T), Insert Blank Line Above and Below Current (Ctrl+Alt+Enter,
  Ctrl+Alt+Shift+Enter, indented like the line) and Reverse Line Order.
- Edit > Blank Operations: trim leading, trailing or both, EOL to Space, TAB
  to Space, and Space to TAB (all, or only the indentation).
- Edit > Column Editor (Alt+C): text, or a number sequence (initial value,
  step, repeat, zeros or spaces for padding, decimal, hex, octal or binary),
  inserted at one column down many lines or a column selection.
- The tab menu: Close All to the Left and to the Right, Close All
  Unchanged, Copy Full File Path, Copy File Name, Copy Folder Path, Reveal
  in File Manager and Reload from Disk. The same copies are in Edit > Copy
  to Clipboard.
- View > Full Screen (F11) and Always on Top.
- A read-only viewer for large files: files over 256 MB (a setting) open
  without being read into memory, so a log of several gigabytes shows its
  first screen at once. The line index builds in the background while you
  scroll, select, copy (up to 64 MB), go to a line and search (Find
  Next/Previous, Count and Find All run in the background). Long lines are
  cut after 10,000 bytes with a marker; UTF-8 and legacy encodings are
  shown, UTF-16 is refused. View > Open as Editable opens such a file in an
  editor (up to 1 GB). The session reopens viewers where they were.

### Fixed

- Control characters (NUL, ESC, ...) are drawn as boxes. Before, they were
  drawn with the font's replacement glyph, which could push the rest of the
  line off its columns.
- A line of several megabytes (minified code, a log without line breaks)
  crashed the app when drawn. Only the part on screen is laid out now, a
  long line takes little more memory than its text, and lines over 20,000
  characters are not highlighted (one of a megabyte took seconds).
- A caret beside a bracket in a large file with non-ASCII text crashed the
  app.
- Opening a folder, a device such as `/dev/zero` or a pipe (from the file
  tree, the session or a reload) could hang the app or fill its memory.
  These are refused now, as are files over 1 GB.
- New files saved on Linux were readable only by you; they get the usual
  permissions now. ochrepad's own backups and settings stay private.
- Reveal in File Manager on Windows failed for names with commas.
- If ochrepad hits a bug, it backs up unsaved work and writes the session
  before closing, and says so.

### Security

- The Markdown preview passes only web (`http`, `https`) and mail links to
  the system. Links to files open in ochrepad, relative to the document; any
  other kind (`ms-settings:`, `search-ms:`, custom protocol handlers, network
  paths) is refused with a note. Hovering a link shows where it goes.
- Preview images can no longer crash the app or exhaust its memory: images
  larger than the graphics card's largest texture are shrunk, SVGs are drawn
  at most that large, and image files over 32 MB, devices and pipes are not
  read. Image paths through a symbolic link to a network share are refused
  like network paths.
- Saving keeps Windows' "downloaded from the internet" mark on a file, so
  Windows still warns before running a downloaded script edited here.
- Backup names from a damaged or tampered session file can no longer point
  outside the backup folder.

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
