# The editor view: zoom, symbols, document map, word wrap and folding

## Zoom

**View > Zoom** makes the text of every editor (and both sides of a
comparison) larger or smaller, without changing the menus and panels:

- **Ctrl+plus** (or **Ctrl+=**) and **Ctrl+minus** step through 10% to 400%;
  **Ctrl+0** goes back to 100%.
- **Ctrl+mouse wheel** over an editor does the same, one step per notch.
- The status bar shows the zoom while it is not 100%; click it to reset.

The zoom is remembered. **Settings > Preferences > Font size** is the size at
100%.

## Show Symbol

**View > Show Symbol** draws what is normally invisible, as in Notepad++:

- **Show Space and Tab**: a dot in each space, an arrow across each tab.
- **Show End of Line**: a small `CRLF`, `LF` or `CR` after each line, the
  ending the document is saved with (the status bar's line ending menu
  changes it). A file opened with mixed endings shows the one it will be
  saved with on every line.
- **Show Non-Printing Characters**: zero-width characters (such as U+200B
  ZERO WIDTH SPACE and the BOM), bidi controls (such as U+202E
  RIGHT-TO-LEFT OVERRIDE) and spaces other than the plain one (such as
  U+00A0 NO-BREAK SPACE) as tinted boxes.
- **Show All Characters** (**Ctrl+Shift+8**) turns the three above on or off
  together.
- **Show Indent Guide**: dotted lines at each indent level inside a line's
  indentation, from the second level on, continuing through blank lines
  inside a block.
- **Show Wrap Symbol**: with word wrap on, a hooked arrow where a line
  continues on the next row.

**Control characters** (NUL, ESC and the rest of U+0000–U+001F, DEL and
U+0080–U+009F) are always drawn as boxes, as Notepad++ does, so text after
them stays in its columns. Each special character takes one cell, the same
whether it is shown or not, so showing them never moves the text. **Hover
over a box** for the character's code and name; zoomed in far enough, the
box also shows its abbreviation (`NUL`, `ESC`, `ZWSP`, `NBSP`, …).

**Vertical edge**: Settings > Preferences > **Vertical edge after column**
draws a thin line after that column (80 when first turned on), as a guide
for line length.

## Document map

The strip on the right of each editor shows the whole document zoomed out:
each line is a thin row, and each run of text a block in its highlight
colour, so you can see the shape of the file and jump around in it.

- The lighter band marks the part of the document on screen. **Drag it** to
  scroll; it moves like a scrollbar.
- **Click** anywhere else in the map to bring that part of the document to
  the middle of the editor.
- The mouse wheel over the map scrolls the editor.
- When the document is longer than the map, the map scrolls along with the
  editor, keeping the band in view.
- Folded lines are left out and wrapped lines show as their rows, as in the
  editor.
- **View > Document Map** turns it off for every editor. It also hides
  itself while a pane is too narrow to show 40 columns of text beside it.

## Change history

The gutter marks the lines that differ from the file as it was when you
opened or last saved it, in the compare view's colours: a **blue** bar
beside a changed line, a **green** bar beside an added one, and a **red**
wedge between two lines where lines were deleted. The marks update a moment
after you stop typing. Undoing back to the saved text clears them, and
saving starts afresh from the saved text.

A document that came back from a backup with unsaved changes is compared
with its file on disk. New documents that were never saved, and files over
the highlighting size limit (Settings > Preferences), have no marks.
**View > Change History** turns the marks off.

## Word wrap

**View > Word Wrap** wraps long lines at the edge of the pane instead of
scrolling sideways. It applies to every tab and is remembered.

- Lines break after a space or tab where possible; a word longer than the
  pane breaks where it reaches the edge. Wide characters and emoji are never
  split.
- The rows after a line's first are indented like the line itself (up to
  half the pane's width), so wrapped code keeps its shape.
- The line number shows on a line's first row only.
- **Up and Down** move by row. **Home** goes to the start of the row, and a
  second press goes to the start of the line (the first non-blank
  character, as without wrap). **End** goes to the end of the row, then to
  the end of the line.
- At the end of a row that is not the line's last, the caret stops before
  the last character of the row (usually the space the line breaks after):
  the position after it is the start of the next row.
- Resizing the window or a pane rewraps the text and keeps the same line at
  the top.
- The scrollbar counts lines, not rows, so in a file with many long lines
  its thumb is a little off. Rewrapping a whole large file to measure it
  exactly would make every resize slow.
- Column selection (Alt+drag) still selects the same columns of each line;
  on a wrapped line the columns may be on different rows.

## Folding

Folding hides a block of lines behind its first line, so you can see the
shape of a long file.

### Where the fold points are

A line can be folded when the next non-blank line is **indented deeper**.
Folding it hides everything up to the next line that is indented no deeper,
so for code like this:

```rust
fn main() {
    if ready {
        go();
    }
}
```

folding `fn main() {` hides the three lines inside, and the closing `}`
stays visible. Blank lines at the end of a block also stay visible. Tabs
count to the next tab stop, using the document's tab width (see
[indentation](file-handling.md#the-status-bar-menus)).

Because it only looks at indentation, folding works the same way in every
language, including Python, YAML and plain text. It is fooled by code that is
not indented to match its structure, such as a multi-line string whose lines
start at the left margin.

**Markdown** files fold by heading instead: a heading folds everything up to
the next heading of the same or a higher level, so `## Install` folds its
`###` subsections with it. A `#` line inside a fenced code block is not a
heading. ochrepad looks up to 200 lines back for the fence that opens it, so
in a very long code block a `#` line far from the opening fence can still get
a fold arrow.

### Folding and unfolding

- Click the **chevron** in the margin next to the line numbers. Chevrons for
  folded lines always show; the others show while the pointer is over the
  gutter. **View > Show Fold Margin** hides the margin.
- A folded line ends with a **⋯** marker; click it to unfold.
- From the keyboard or the **View** menu:

| | |
|---|---|
| Fold the block around the caret | Ctrl+Alt+F |
| Unfold it | Ctrl+Alt+Shift+F |
| Fold everything / unfold everything | Alt+0 / Alt+Shift+0 |
| Fold / unfold every block of a level | Alt+1 … Alt+8 / Alt+Shift+1 … Alt+Shift+8 |

Levels count from the outside: level 1 is the outermost blocks, level 2 the
blocks inside those, and so on. In Markdown the level is the heading level.
These are the Notepad++ shortcuts; they can be changed in
[the keymap](keymap.md#folding).

### How folded text behaves

- **The caret never hides.** Up and Down skip folded lines. Anything that
  puts a caret inside a fold (Find Next, a click on a search result, Undo,
  Right arrow at the end of a folded line, a macro) unfolds it.
- **Folding never changes the file.** Folds are not undo steps, the tab
  does not become unsaved, and macros do not record them.
- **Editing inside a fold unfolds it.** A Replace All that changes hidden
  text, for example, opens that fold. Typing at the end of the folded line
  keeps it folded.
- **Selections include hidden lines.** Selecting across a folded line and
  copying or deleting takes the hidden lines too.
- **Each view folds separately.** A second view of the same file (View >
  Clone to Other View) starts unfolded.
- **Folds are saved with the session** and come back the next time you start
  ochrepad, as long as the same lines still start a block.
