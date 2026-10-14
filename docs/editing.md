# Editing lines, blanks and columns

These are Notepad++'s line, blank and column commands. Each one is a single
undo step, can be recorded in a macro, and can be bound to other keys
([Key bindings](keymap.md)).

## Line operations

**Edit > Line Operations** has, besides duplicate, move and join:

| Command | Keys | Does |
|---|---|---|
| Delete Line | Ctrl+Shift+L | Deletes the caret's line with its line break, or every line a selection touches. The caret goes to the start of the line that moves up. |
| Transpose Line | Ctrl+T | Swaps the caret's line with the line above it. The caret goes to the start of its line, which now holds the text from above, so pressing it again swaps them back. |
| Insert Blank Line Above Current | Ctrl+Alt+Enter | Inserts a line above the caret's line, indented like it, and puts the caret there. |
| Insert Blank Line Below Current | Ctrl+Alt+Shift+Enter | The same, below. |
| Reverse Line Order | | Reverses the selected lines, or every line when nothing is selected. |

With several carets, each line is done once. A selection over several
lines inserts above its first line or below its last one.

## Blank operations

**Edit > Blank Operations** works on the selected lines, or on every line
when nothing is selected:

| Command | Does |
|---|---|
| Trim Trailing Space | Removes spaces and tabs at the ends of lines. |
| Trim Leading Space | Removes spaces and tabs at the starts of lines. |
| Trim Leading and Trailing Space | Both. |
| EOL to Space | Joins the lines into one, with a space for each line break. Unlike Join Lines, the spaces around the breaks stay. A final line break stays. |
| TAB to Space | Replaces every tab with the spaces that reach the same tab stop, so nothing moves. |
| Space to TAB (All) | Replaces spaces with tabs wherever that keeps every column: the indentation (tabs, then any spaces narrower than a tab), and after it every run of two or more spaces that reaches a tab stop. A single space before a tab stop stays a space. |
| Space to TAB (Leading) | The indentation only: the same as **Convert Indentation to Tabs** in the status bar's indentation menu, which also makes the document indent with tabs from then on. |

The tab width is the document's (see the status bar).

## Column Editor

**Edit > Column Editor…** (**Alt+C**) inserts the same text, or a sequence
of numbers, at one column on many lines:

- With a **column selection** (Alt+drag), on each of its lines at its left
  edge. Text inside the rectangle is replaced, so a rectangle that is one
  column wide or wider can be overwritten.
- With **several carets**, at each caret, top to bottom.
- With **one caret**, on its line and every line below it to the end of the
  document, at the caret's column (the empty line after a final line break
  is left out).
- With **one selection**, on each of its lines, at the column it starts in.

A line too short to reach the column is padded with spaces first.

For numbers:

| Field | Meaning |
|---|---|
| Initial number | The first number. It can be negative. |
| Increase by | Added for each next number (negative counts down). |
| Repeat | How many lines in a row get the same number. |
| Leading | **None**, **Zeros** (`08`, `09`, `10`) or **Spaces** (right-aligned), so that all the numbers are the same width. |
| Format | Decimal, hexadecimal (upper case), octal or binary. |

For example, with the caret at the start of the first of ten lines, Initial
number 1, Increase by 1 and Leading **Zeros** put `01` to `10` in front of
them.

The fields keep their values for the next time, as in Notepad++.

## Copying names, and the tab menu

**Edit > Copy to Clipboard** copies the current document's full path, file
name or folder. A document that was never saved has only a name ("new 1").

Right-clicking a tab offers, besides Close, Close All but This and Clone to
Other View:

- **Close All to the Left** and **Close All to the Right**: the other tabs on
  that side in the same pane.
- **Close All Unchanged**: every tab without unsaved changes, in every pane.
- **Copy Full File Path**, **Copy File Name** and **Copy Folder Path**.
- **Reveal in File Manager**: the file, selected in Explorer on Windows; its
  folder on Linux.
- **Reload from Disk**: reads the file again. With unsaved changes it asks
  first; Undo brings the changes back.

Closing tabs with unsaved changes asks about each, as closing one does.
A **middle click** on a tab closes it. **File > Restore Recent Closed
File** (**Ctrl+Shift+T**) opens the file closed last again, and the one
before it the next time.

## Multi-select, paste from history, read-only

- **Edit > Multi-select Next** adds the next occurrence of the selected
  text as another selection, so typing then edits them all; with only a
  caret, the first time selects the word at it. **Edit > Multi-select All**
  selects every occurrence at once. Each has Notepad++'s four kinds: Ignore
  Case & Whole Word (neither), Match Case Only, Match Whole Word Only, and
  Match Case & Whole Word. They have no keys of their own; the
  `MultiSelectNext` and `MultiSelectAll` commands can be given some in
  [keymap.json](keymap.md).
- **Edit > Paste from History** offers the last ten texts copied (or cut)
  in ochrepad, newest first, so something copied before the last copy can
  still be pasted. A rectangle pastes as a block.
- **Edit > Read-Only** refuses edits to the current document until it is
  turned off; the tab says "(read-only)". The file on disk is not changed.
- The status bar shows how many characters are selected, and on how many
  lines ("12 selected, 3 lines").

## Bookmarks

A bookmark marks a line, as in Notepad++: its line number is drawn on a
tag in the accent colour. **Ctrl+F2** bookmarks the caret's line (or
removes its bookmark), **F2** goes to the next bookmarked line and
**Shift+F2** to the previous one, round from the last to the first. The
right-click menu adds or removes the bookmark of the line you click, also
on its line number.

**Search > Bookmark** has the rest:

- **Clear All Bookmarks** and **Inverse Bookmark** (bookmark the lines
  without one, and only those).
- **Cut**, **Copy** and **Remove Bookmarked Lines**, and **Remove Unmarked
  Lines**: copying takes each line with its line break, and removing is one
  undo step.
- **Paste to (Replace) Bookmarked Lines** replaces the text of every
  bookmarked line with what is on the clipboard.

The find panel's **Bookmark** button bookmarks every line with a match
(the lines with "ERROR" in a log, say); then F2 steps through them.

Bookmarks move with the text: lines added above push them down, and a
bookmarked line joined to the one before takes its bookmark along. Adding
and removing bookmarks is not an undo step, and undoing an edit doesn't
bring back a bookmark it removed. The session keeps each file's bookmarks.

## The right-click menu

Right-clicking the text moves the caret there, unless you click on the
selection, and opens a menu with Undo and Redo, the clipboard (Cut, Copy,
Paste, Delete, Select All), UPPERCASE and lowercase, the comment toggles,
and the bookmark of the line. Right-clicking a line number does the same
for that line.

## Full screen and always on top

**View > Full Screen** (**F11**) fills the screen with the window; F11 again
goes back. **View > Always on Top** keeps the window above other programs'
windows, and is remembered.

## Closing brackets and quotes

**Settings > Preferences > Brackets and quotes: Close them as you type**
(off by default, as in Notepad++) makes typing an opening `(`, `[`, `{`,
`"` or `'` insert its partner after the caret:

- The partner goes in only where it helps: before a space, the end of the
  line or a closing bracket. Typing `(` in front of a word types just `(`.
- A quote right after a letter or digit is an apostrophe (`don't`), so it
  gets no partner.
- Typing the closing character where it already follows the caret steps
  over it instead of typing a second one.
- **Backspace** between an empty pair, as in `(|)`, deletes both.
- With a selection, typing replaces it as usual. With several carets, each
  gets its pair, or none of them does.
- The pair and what you type inside it undo together, like typing.
