# Column selection and multiple cursors

ochrepad can edit in many places at once. Everything you type, delete or
paste happens at every caret, and it is all one step for undo.

## Making carets and selections

| | |
|---|---|
| **Alt+drag** | Selects a rectangle (a column selection). |
| **Alt+Shift+arrow keys** | Starts a rectangle at the caret, or grows and shrinks the current one. |
| **Ctrl+click** | Adds a caret where you click. The new caret becomes the main one. |
| **Ctrl+Alt+Up / Down** | Adds a caret on the line above or below, in the same column. |
| **Select All** in the find panel | Puts a selection on every match. |
| **Esc** | Back to a single caret (the main one). |

The status bar shows what you have: `Column selection, 12 lines` or
`5 selections`.

## How a column selection works

A column selection is a rectangle measured in screen columns, so tabs and
wide characters (such as Chinese or Japanese) line up the way you see them.
It becomes one selection per line:

- **Lines shorter than the rectangle** get an empty selection at their end.
  ochrepad has no "virtual space" past the end of a line, so typing there
  adds text right after the line's last character rather than padding it out
  to the column.
- **A tab or wide character that straddles an edge** of the rectangle is left
  out of that line's selection, so a selection never covers half a
  character.

Typing replaces the selected text on every line. With an empty rectangle
(zero columns wide, which is what Alt+drag straight down makes), typing
inserts the same text on every line, which is handy for adding a prefix or a
comment marker to a block of lines.

## Copy and paste

**Copying several selections** puts them on the clipboard one per line, in
document order. Empty selections are skipped, except in a column selection,
where every line counts (so a rectangle that crosses short lines keeps its
shape).

**Pasting** depends on what you paste into:

- **One caret, and the text came from a column copy in ochrepad**: it is
  pasted as a block. Line 1 goes at the caret, line 2 at the same column on
  the next line, and so on. Short lines are padded with spaces to reach the
  column, and lines are added at the end of the document if needed.
- **Several carets, and the text has exactly as many lines as there are
  carets**: each caret gets one line. Copying from five carets and pasting
  into five carets puts every piece back on its own line. This also works
  with text from other programs, for example pasting a list of five names
  into a five-line column.
- **Otherwise**: the whole text is pasted at every caret.

ochrepad remembers its last column copy to recognise it when you paste. Text
copied from other programs is never pasted as a block.

## Sorting by a column

With a column selection active, **Edit > Line Operations > Sort Lines** sorts
the whole lines, but compares them by the text inside the rectangle, as
Notepad++ does. For example, select the date column of a log and sort to put
the lines in date order.
