# Logs and large files

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
