# ochrepad documentation

The [main README](../README.md) covers building, the keyboard shortcuts and
where settings are kept. These pages go into the parts that need more than a
menu item to explain:

- [Find and replace](find-and-replace.md): the panel's options and how the
  regular-expression syntax differs from Notepad++.
- [Column selection and multiple cursors](column-selection.md): making
  rectangles and extra carets, and how copy, paste and sorting work with them.
- [The editor view](view.md): zoom, Show Symbol (spaces, tabs, line
  endings, invisible characters, indent guides), matching brackets and
  tags, the document map, the Document List and Function List, the change
  history, word wrap, and folding (where blocks can be folded, the
  shortcuts, how folded text behaves).
- [Logs and large files](logs.md): filtering lines, highlight rules for log
  levels, following a file as it grows (tail -f), and the read-only viewer
  that opens files of any size at once.
- [Working in a folder](workspace.md): opening a folder, the file tree in
  the sidebar, ignored files, and creating and renaming files.
- [Macros](macros.md): what gets recorded, running a macro to the end of the
  file, saving macros with shortcuts.
- [Text tools](text-tools.md): formatting and checking JSON, Base64, URL
  and hex encoding, and hashes.
- [Editing lines, blanks and columns](editing.md): Notepad++'s line and
  blank operations, the Column Editor, the tab menu and reopening closed
  files, bookmarks, the right-click menu, the summary, multi-select, paste
  from history, read-only documents, full screen, closing brackets as you
  type, and smart indentation.
- [Completion](completion.md): the suggestion list, how typed letters
  match names, where suggestions come from, and indexing the open folder.
- [Language servers](language-servers.md): using rust-analyzer, pyright
  and others for problems and hover information, trusting folders, and
  `lsp.json`.
- [The terminal](terminal.md): terminal tabs, the shell, the keys a
  terminal takes, copy and paste, Ctrl+clicking links and compiler errors.
- [Key bindings](keymap.md): the `keymap.json` format and every command you
  can bind.
- [Backups, sessions and files changed elsewhere](backups-and-session.md):
  how unsaved work is kept, what happens on exit, automatic saving, saving
  and loading session files, and reloading files other programs change.
- [How ochrepad reads and writes files](file-handling.md): encodings, line
  endings, which files are refused, atomic saves, and exporting a document
  as a web page.
