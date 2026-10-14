# ochrepad documentation

The [main README](../README.md) covers building, the keyboard shortcuts and
where settings are kept. These pages go into the parts that need more than a
menu item to explain:

- [Find and replace](find-and-replace.md): the panel's options and how the
  regular-expression syntax differs from Notepad++.
- [Column selection and multiple cursors](column-selection.md): making
  rectangles and extra carets, and how copy, paste and sorting work with them.
- [The editor view](view.md): zoom, Show Symbol (spaces, tabs, line
  endings, invisible characters, indent guides), the document map, word
  wrap, and folding (where blocks can be folded, the shortcuts, how folded
  text behaves).
- [Working with logs](logs.md): filtering lines, highlight rules for log
  levels, and following a file as it grows (tail -f).
- [Working in a folder](workspace.md): opening a folder, the file tree in
  the sidebar, ignored files, and creating and renaming files.
- [Macros](macros.md): what gets recorded, running a macro to the end of the
  file, saving macros with shortcuts.
- [Text tools](text-tools.md): formatting and checking JSON, Base64 and URL
  encoding, and hashes.
- [Editing lines, blanks and columns](editing.md): Notepad++'s line and
  blank operations, the Column Editor, the tab menu, and full screen.
- [Key bindings](keymap.md): the `keymap.json` format and every command you
  can bind.
- [Backups, sessions and files changed elsewhere](backups-and-session.md):
  how unsaved work is kept, what happens on exit, automatic saving, and
  reloading files other programs change.
- [How ochrepad reads and writes files](file-handling.md): encodings, line
  endings, which files are refused, and atomic saves.
- [Logs and large files](logs.md): the read-only viewer that opens files of
  any size at once.
