# Changelog

All notable changes to ochrepad. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/) (while below 1.0, a minor version
adds features and a patch version fixes bugs).

Each release's section becomes its GitHub release notes, so write it for
people downloading the release.

## [Unreleased]

### Added

- Smart highlighting: selecting text highlights its other occurrences, as in
  Notepad++.

### Changed

- Ctrl+F and Ctrl+H select the search text, so you can type a new search
  straight away even when the find panel is already open.

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

[Unreleased]: https://github.com/xor295/ochrepad/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/xor295/ochrepad/releases/tag/v0.1.0
