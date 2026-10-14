# Folding and the editor view

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
count to the next tab stop, using the tab width from the preferences.

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
