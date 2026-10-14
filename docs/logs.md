# Logs and large files

ochrepad has three tools for log files, and they work on any file: a filter
that shows only the lines you want, highlight rules that colour lines by
level, and follow mode, which shows what a program appends to a file as it
writes it. Files too large to edit open in a read-only
[viewer](#large-files-the-viewer) instead.

## Filter lines

**Search > Filter Lines** (**Alt+Shift+L**) opens the filter bar under the
editor. Type some text, and after a moment only the lines that contain it
are shown:

- **Show matching** shows only the lines with a match; **Hide matching**
  hides them and shows the rest.
- **Match case**, **Whole word** and **Regular expression** work as in the
  find panel (the regular-expression syntax is described in
  [Find and replace](find-and-replace.md)). A match that spans lines shows
  all of them.
- The bar says how many lines are shown, out of how many. The line numbers
  in the gutter are the real ones, so you can tell where a line is.
- Up and Down move between the shown lines, and Page Up/Down and the
  scrollbar go through them too.
- **Escape** in the field, or **×**, removes the filter.

The search runs in the background, so a large file stays responsive while
it is filtered; the old filter stays until the new one is ready.

**Editing while filtered.** You can edit a filtered document. Lines you
change, add or paste stay visible while you work, and a moment after you
stop typing the filter is applied again. Going to a hidden line (Go to
Line, Find Next) shows that line.

Each tab has its own filter, and the bar shows the one of the tab you are
in. Filters are not kept when you close ochrepad. Folding works as usual on
a filtered document.

## Highlight rules

Files whose name matches **\*.log** or **\*.log.\*** (`server.log`,
`server.log.1`) are coloured by these rules:

| Lines with | Look |
|---|---|
| `ERROR`, `FATAL`, `CRITICAL` or `SEVERE` | red |
| `WARN` or `WARNING` | amber |
| `DEBUG`, `TRACE` or `VERBOSE` | dimmed |

The rules are in **Settings > Preferences > Log highlighting**, where you
can change which files they apply to (patterns with `*` and `?`, separated
by `;`), and add, change or remove rules. Each rule has:

- a **pattern**: plain text, or a regular expression with **Regex** on;
  **Match case** as in the find panel;
- a **colour**;
- a **style**: **Whole line** tints the line (the first whole-line rule that
  matches wins), **Match only** tints just the matching text, and **Dim the
  line** draws its text faintly.

**Restore Defaults** brings back the rules above. A pattern that is not a
valid regular expression is shown in red and ignored.

## Follow (tail -f)

**View > Follow (tail -f)** follows the current file as another program
writes to it, like `tail -f` or Notepad++'s monitoring mode:

- When the file grows, the new text is added to the end of the document a
  moment later, without asking. The tab stays clean, and the added text is
  not an undo step.
- If the caret is at the end of the document, it stays at the end and the
  view scrolls along. Put the caret anywhere else to read in peace.
- A line written in several pieces appears as it is written; a Windows line
  ending split between two writes is joined.
- If the file gets shorter or changes before its end (a log that is
  rotated or truncated), it is reloaded instead.
- If you edit the document, following pauses: changes to the file then ask
  what to do, as for any file.
- The status bar shows **Following** while it is on; click it to stop.

Follow mode works on UTF-8 files (plain ASCII included). Files in other
encodings are reloaded whole on each change instead.

## Large files: the viewer

Files larger than **256 MB** open read-only in the **viewer for large
files** instead of an editor. The viewer doesn't read the file into memory:
it shows the first screen at once and reads only the lines you look at, so
multi-gigabyte logs open as fast as small ones and never fill the memory.
The size is **Settings > Preferences > Open read-only above … MB** (1 to
1024 MB; files over 1 GB can only be viewed).

While the file is open, ochrepad builds an index of where its lines start
in the background. The status bar shows how far it has got ("Indexing…
45%, 3,100,000 lines so far"), then the number of lines. You can scroll,
select and search before it is done; the scrollbar and the line count grow
as the index does.

What the viewer does:

- **Scrolling and selecting** with the mouse and the keyboard, as in the
  editor: arrows, Home and End, Page Up and Page Down, Ctrl+Home and
  Ctrl+End, Ctrl+arrows by word, Shift to select, double-click for a word,
  triple-click or a click on the line number for a line, Ctrl+A for
  everything, Ctrl+mouse wheel to zoom.
- **Copy** (Ctrl+C) copies the selection, at most 64 MB of it, with line
  breaks as LF like the editor. A larger selection is cut, with a note in
  the status bar.
- **Go to Line** (Ctrl+G). A line the index hasn't reached yet is gone to as
  soon as it has; the status bar says so meanwhile.
- **Find** (Ctrl+F, F3 and Shift+F3, Count, Find All in the current
  document): the search runs in the background over the whole file, and the
  find panel says "Searching…" until it is done. Find All lists the first
  10,000 matches in the Search Results panel; click one to go there.
  Matches on screen are highlighted. The regular-expression syntax is the
  same as in the editor, and `^`, `$` and `.` treat CR LF and CR as line
  breaks too.
- **Long lines** show their first 10,000 bytes, then a marker saying how
  much is left out. Copying a selection that includes such a line copies
  all of it.
- **Encodings**: UTF-8 (with or without a byte order mark) and the
  single-byte and other legacy encodings ochrepad detects (Windows-1252,
  Shift_JIS, …) are shown; invalid bytes show as `�`. UTF-16 files can't be
  shown in the viewer (their line breaks are two bytes), and are refused
  with a message. In a legacy-encoded file, a search finds ASCII text; text
  with other characters is matched as UTF-8 and so is not found.
- Line endings are shown in the status bar (the most common kind) and
  never changed: nothing is ever written to the file.

**View > Open as Editable…** (also on the tab's right-click menu) opens the
file in a normal editor tab instead, after warning that it is held in
memory several times over. Files over 1 GB can't be opened for editing.

The viewer isn't a document in the usual sense: it isn't part of Find All
in all open documents, backups or comparisons. The session remembers it
(and where you were) and opens it again in a viewer.

### When the file changes

If another program adds to the file (a log being written), the viewer
indexes the new part and the line count grows. If the file gets shorter (a
log that was truncated or rewritten), it is read again from the start, with
a note in the status bar. On Windows, a file open in the viewer can't be
truncated by another program (Windows refuses while the file is mapped into
memory); it can still be appended to, renamed or deleted.
