# Key bindings

The editor's key bindings live in `keymap.json` in your configuration folder
(see [Configuration](../README.md#configuration)). **Settings > Edit Keymap**
opens it in a tab. The first time, it is filled with every default binding,
so you can see and change them all. Saving the file applies it straight away.

If the file has a mistake, ochrepad says what and where (for example
`keymap.json entry 3: "Ctrl+Shft+K" is not a key combination`) and keeps the
bindings it had.

## The file

The file is a list of entries. Each entry names a key combination and the
command it runs:

```json
[
  { "keys": "Ctrl+D", "command": { "Action": "SelectLine" } },
  { "keys": "Ctrl+Alt+L", "command": { "Action": "TrimTrailingWhitespace" } },
  { "keys": "Ctrl+J", "command": null }
]
```

- An entry **replaces** the default binding of its keys. Keys you don't
  mention keep their defaults, so a file with only your changes is enough.
- `"command": null` **removes** a binding, leaving the keys unbound.
- If the same keys appear twice, the later entry wins.

### Writing keys

Modifiers come first, joined with `+`: `Ctrl`, `Alt` and `Shift`, in any
order, any case (`Cmd` and `Command` are also accepted for `Ctrl`). Then
one key:

| | Names |
|---|---|
| Letters and digits | `A` … `Z`, `0` … `9` |
| Function keys | `F1` … `F35` |
| Arrows | `Up`, `Down`, `Left`, `Right` (or `ArrowUp`, …) |
| Editing | `Enter`, `Tab`, `Backspace`, `Delete`, `Insert`, `Escape` (or `Esc`), `Space` |
| Navigation | `Home`, `End`, `PageUp`, `PageDown` |
| Punctuation | `Comma`, `Period`, `Semicolon`, `Colon`, `Quote`, `Backtick`, `Minus`, `Plus`, `Equals`, `Slash`, `Backslash`, `Pipe`, `Questionmark`, `Exclamationmark`, `OpenBracket`, `CloseBracket`, `OpenCurlyBracket`, `CloseCurlyBracket` |

For example `Ctrl+Shift+Up`, `alt+f5`, `Ctrl+OpenBracket`.

### What the keymap does not cover

- **Application shortcuts** are fixed and are handled before the keymap,
  so binding them here has no effect: Ctrl+N, Ctrl+O, Ctrl+S, Ctrl+Shift+S,
  Ctrl+W, Ctrl+Tab, Ctrl+Shift+Tab, Ctrl+F, Ctrl+Shift+F, Ctrl+H, Ctrl+G, F3,
  Shift+F3, Ctrl+F3, Ctrl+Shift+F3, Ctrl+Shift+T, Ctrl+PageUp,
  Ctrl+PageDown, Ctrl+Shift+PageUp, Ctrl+Shift+PageDown, Ctrl+Shift+R,
  Ctrl+Shift+P, Ctrl+Shift+M, Ctrl+Shift+E, F8,
  Shift+F8, Ctrl+plus, Ctrl+=, Ctrl+minus, Ctrl+0, Ctrl+Shift+8,
  Alt+Shift+L, Alt+C (Column Editor), F11 (Full Screen), Ctrl+\` (Terminal)
  and Ctrl+Shift+\` (New Terminal).
- **Copy, cut and paste** (Ctrl+C, Ctrl+X, Ctrl+V) come from the system
  clipboard, not from key bindings.
- **A saved macro's shortcut** takes precedence over a keymap binding for the
  same keys (see [Macros](macros.md)).
- The keymap applies while the editor has the keyboard. In the find panel or
  a dialog, keys go to that field, and in a terminal to the program running
  there (see [The terminal](terminal.md#keys)).

## Commands

A command is `{ "Action": … }` for nearly everything. An action without
options is written as a string, `{ "Action": "SelectAll" }`. An action with
options is an object, `{ "Action": { "ConvertCase": "Upper" } }`.

### Moving and selecting

| Command | Does |
|---|---|
| `{"Action": {"MoveCursor": {"motion": "Left", "extend": false}}}` | Moves every caret. With `"extend": true` it extends the selections instead (the Shift versions). The motions are listed below. |
| `{"PageUp": {"extend": false}}` | Moves up by one screenful. |
| `{"PageDown": {"extend": false}}` | Moves down by one screenful. |
| `{"Action": "SelectAll"}` | Selects the whole document. |
| `"BeginEndSelect"` | Edit > Begin/End Select: the first time marks the caret as where a selection begins, the second selects from there to the caret. No key by default. |
| `"BeginEndSelectColumn"` | The same, ending in a column selection. |
| `{"Action": "SelectWord"}` | Selects the word at each caret. |
| `{"Action": "SelectLine"}` | Selects each caret's whole line, including its line break. |
| `{"Action": "AddCursorAbove"}` | Adds a caret on the line above, in the same column. |
| `{"Action": "AddCursorBelow"}` | Adds a caret on the line below. |
| `{"Action": "CollapseSelections"}` | Back to a single caret (Esc). |
| `{"Action": {"ColumnExtend": "Down"}}` | Grows or shrinks a column selection by moving its corner: `"Up"`, `"Down"`, `"Left"` or `"Right"`. Starts one at the caret if there is none. |

Motions for `MoveCursor`:

| Motion | Moves to |
|---|---|
| `"Left"`, `"Right"` | the previous or next character (a whole accented letter or emoji counts as one) |
| `"WordLeft"`, `"WordRight"` | the previous or next word boundary |
| `"Up"`, `"Down"` | the line above or below (the row, with word wrap), keeping the column you started from; folded lines are skipped |
| `"Home"` | the first non-blank character of the line, or the line start if already there |
| `"RowHome"` | with word wrap, the start of the caret's row, then `"Home"` on a second press; without wrap, the same as `"Home"` (the Home key) |
| `"RowEnd"` | with word wrap, the end of the caret's row, then the end of the line; without wrap, the line end (the End key) |
| `"LineStart"`, `"LineEnd"` | the very start or end of the line |
| `"DocStart"`, `"DocEnd"` | the start or end of the document |
| `{"PageUp": 20}`, `{"PageDown": 20}` | the given number of rows up or down |

### Typing and deleting

| Command | Does |
|---|---|
| `{"Action": "InsertNewline"}` | A line break, keeping the current line's indentation. |
| `{"Action": "InsertTab"}` | A tab, or spaces up to the next tab stop when **Indent with: Spaces** is set. |
| `{"Action": {"InsertText": "→"}}` | Types the given text at every caret. |
| `{"Action": {"Paste": "TODO: "}}` | Inserts fixed text like paste does: with several carets and as many lines as carets, one line goes to each. |
| `{"Action": {"PasteBlock": "a\nb\nc"}}` | Inserts the lines as a column block, one per line from the caret down. |
| `{"Action": {"Complete": {"before": 0, "after": 0, "text": "println!"}}}` | What accepting a completion does: at every caret, replaces the `before` characters before it and the `after` characters after it (within the line) with the text, as one undo step. |
| `{"Action": "DeleteBackward"}` | Backspace. |
| `{"Action": "DeleteForward"}` | Delete. |
| `{"Action": "DeleteWordBackward"}` | Deletes to the previous word boundary. |
| `{"Action": "DeleteWordForward"}` | Deletes to the next word boundary. |
| `{"Action": "Undo"}` | Undo. |
| `{"Action": "Redo"}` | Redo. |
| `"ToggleOvertype"` | Switches between inserting and typing over the text (Insert), as the status bar's **OVR** shows. |

### Lines and case

| Command | Does |
|---|---|
| `{"Action": "DuplicateLine"}` | Duplicates the caret's line, or each selection. |
| `{"Action": "MoveLinesUp"}` | Moves the selected lines up one line. |
| `{"Action": "MoveLinesDown"}` | Moves the selected lines down one line. |
| `{"Action": "JoinLines"}` | Joins the selected lines (or the caret's line and the next). |
| `"SplitLines"` | Split Lines (Ctrl+I): splits the selected lines, or the caret's line, at the blanks between words to fit the vertical edge, or the pane's width without one ([more](editing.md#line-operations)). |
| `{"Action": {"SplitLines": {"width": 72}}}` | The same, at a fixed number of columns. |
| `{"Action": "TrimTrailingWhitespace"}` | Removes trailing spaces and tabs from the selected lines, or from every line when nothing is selected. |
| `{"Action": {"RemoveEmptyLines": {"include_whitespace": false}}}` | Removes empty lines from the selected lines, or from the whole document when nothing is selected. With `include_whitespace`, lines of only spaces and tabs go too. |
| `{"Action": "DeleteLine"}` | Deletes the caret's line, or every line a selection touches, with its line break (Ctrl+Shift+L). |
| `{"Action": "TransposeLines"}` | Swaps the caret's line with the line above it; the caret goes to the start of its line (Ctrl+T). |
| `{"Action": "InsertLineAbove"}` | Inserts a line above the caret's line, indented like it, and moves the caret there (Ctrl+Alt+Enter). |
| `{"Action": "InsertLineBelow"}` | Inserts a line below the caret's line, indented like it, and moves the caret there (Ctrl+Alt+Shift+Enter). |
| `{"Action": "ReverseLines"}` | Reverses the order of the selected lines, or of every line when nothing is selected. |
| `{"Action": {"RemoveDuplicateLines": {"consecutive_only": false}}}` | Removes lines equal to an earlier line, keeping the first one and the order, in the selected lines or the whole document. Lines compare exactly. With `consecutive_only`, only a line equal to the one right before it is removed. |
| `{"Action": "TrimLeadingWhitespace"}` | Removes leading spaces and tabs from the selected lines, or from every line when nothing is selected. |
| `{"Action": "TrimLeadingAndTrailingWhitespace"}` | Removes leading and trailing spaces and tabs from the selected lines, or from every line. |
| `{"Action": "EolToSpace"}` | Replaces the line breaks between the selected lines (every line when nothing is selected) with spaces, keeping the blanks around them. |
| `{"Action": "TabsToSpaces"}` | Replaces every tab on the selected lines (every line) with the spaces that reach the same tab stop. |
| `{"Action": "SpacesToTabs"}` | Replaces spaces with tabs wherever the columns stay the same: the indentation, and runs of two or more spaces reaching a tab stop. |
| `{"Action": {"ColumnInsert": {"Text": "> "}}}` | The Column Editor: inserts the text at the caret's column on its line and every line below, on each line of a column selection, or at each of several carets ([more](editing.md#column-editor)). For numbers, give `{"Numbers": {"initial": 1, "increase": 1, "repeat": 1, "leading": "Zeros", "format": "Dec"}}` instead of `{"Text": …}`: `leading` is `"None"`, `"Zeros"` or `"Spaces"`, and `format` is `"Dec"`, `"Hex"`, `"Oct"` or `"Bin"`. Fields left out take those values (with `"None"`). |
| `{"Action": {"ConvertCase": "Upper"}}` | Converts the selections (or the caret's line) to `"Upper"`, `"Lower"`, `"Title"`, `"TitleBlend"`, `"Sentence"`, `"SentenceBlend"` or `"Invert"` case ([more](editing.md#convert-case)). `{"Random": {"seed": 1}}` gives random case; the same seed gives the same result each time. |
| `{"Action": {"SortLines": {"descending": false, "case_insensitive": true, "numeric": false, "remove_duplicates": false}}}` | Sorts the selected lines, or the whole document when nothing is selected. These four options must be given. `numeric` compares the number at the start of each line; `remove_duplicates` drops repeated lines; an optional `"by_length": true` sorts by length instead. With a column selection, lines are sorted by the text inside the column. |
| `{"Action": {"ShuffleLines": {"seed": 42}}}` | Puts the same lines in a random order (Randomize Line Order); the seed decides which, so the same seed gives the same order. |

### Indenting, comments and brackets

| Command | Does |
|---|---|
| `{"Action": "Indent"}` | Indents the selected lines, or the caret's line, by one level (a tab, or spaces when the editor indents with spaces). Tab does this when a selection spans lines. |
| `{"Action": "Unindent"}` | Removes one level of indentation from the selected lines, or the caret's line (Shift+Tab). |
| `{"Action": {"ConvertIndentation": {"to_spaces": true}}}` | Rewrites every line's leading whitespace as spaces (or with `false`, tabs), keeping its width, and indents the document that way from now on. One undo step. Also in the status bar's indentation menu. |
| `{"Action": "ToggleLineComment"}` | Comments the selected lines, or the caret's line, at their common indent, or uncomments them if they all are (Ctrl+/, or Notepad++'s Ctrl+Q). For a language with only block comments, wraps the lines in one. |
| `{"Action": "CommentLines"}` | Notepad++'s Single Line Comment (Ctrl+K): comments the lines even if they already are, adding a level. For a language with only block comments, wraps the lines unless they already are wrapped. |
| `{"Action": "UncommentLines"}` | Notepad++'s Single Line Uncomment (Ctrl+Shift+K): takes one line comment off each line that has one and leaves the others, or unwraps the lines' block comment. |
| `{"Action": "ToggleBlockComment"}` | Wraps each selection, or the caret's line, in a block comment, or unwraps it (Ctrl+Shift+/ or Ctrl+Shift+Q). |
| `{"Action": "GoToMatchingBracket"}` | Moves the caret to the bracket matching the one beside it (Ctrl+B). |
| `{"Action": "SelectToMatchingBracket"}` | Selects from the bracket beside the caret to its match, both included (Ctrl+Alt+B). |
| `{"Action": {"GoToLine": {"line": 10, "column": 1}}}` | Moves the caret to a line and column, both counted from 1 as in the status bar. Ctrl+G asks for them. |

### Text tools

These work on every selection, or on the whole document when nothing is
selected; see [Text tools](text-tools.md).

| Command | Does |
|---|---|
| `{"Action": {"FormatJson": {"indent": null}}}` | Formats JSON with one member or element per line, indented like the document. Give an indent to use another: `{"FormatJson": {"indent": {"use_spaces": true, "width": 2}}}`. |
| `{"Action": "MinifyJson"}` | Removes the blank space between JSON tokens. |
| `{"Action": {"Encode": "Base64"}}` | Encodes as `"Base64"`, as URL percent-encoding (`"Url"`) or as hex digits (`"Hex"`). |
| `{"Action": {"Decode": "Url"}}` | Decodes `"Base64"`, `"Url"` or `"Hex"`; refused when the text is not valid or does not decode to UTF-8 text. |

### Folding

Fold commands change only what the editor shows: they are not undo steps and
are not recorded in macros. See [Folding](view.md#folding).

| Command | Does |
|---|---|
| `{"Action": "Fold"}` | Folds the caret's line if a region starts there, else the innermost region around the caret (Ctrl+Alt+F). |
| `{"Action": "Unfold"}` | Unfolds the caret's line, or every fold inside the region around the caret (Ctrl+Alt+Shift+F). |
| `{"Action": "ToggleFold"}` | Unfolds the caret's line if it is folded, else folds as `Fold` does. No default keys. |
| `{"Action": "FoldAll"}` | Folds every region (Alt+0). |
| `{"Action": "UnfoldAll"}` | Unfolds everything (Alt+Shift+0). |
| `{"Action": {"FoldLevel": 1}}` | Folds every region of a level: 1 is outermost; in Markdown the level is the heading level (Alt+1 … Alt+8). |
| `{"Action": {"UnfoldLevel": 1}}` | Unfolds every region of a level (Alt+Shift+1 … Alt+Shift+8). |

### Bookmarks

Bookmarks belong to the document, and adding or removing them is not an
undo step; the commands that delete or replace lines are. Macros record
them (a macro can find a line and bookmark it, then run to the end of the
file); a bookmark set from the right-click menu is not recorded, as no
mouse action is. See [Bookmarks](editing.md#bookmarks).

| Command | Does |
|---|---|
| `{"Action": "ToggleBookmark"}` | Bookmarks the caret's line (each caret's line), or removes the bookmarks if they all have one (Ctrl+F2). |
| `{"Action": "NextBookmark"}` | Moves the caret to the start of the next bookmarked line, from the last one to the first (F2). |
| `{"Action": "PrevBookmark"}` | Moves the caret to the start of the previous bookmarked line (Shift+F2). |
| `{"Action": "ClearBookmarks"}` | Removes every bookmark. |
| `{"Action": "InverseBookmarks"}` | Bookmarks every line without a bookmark, and removes the others. |
| `{"Action": "RemoveBookmarkedLines"}` | Deletes the bookmarked lines, as one undo step. |
| `{"Action": "RemoveUnbookmarkedLines"}` | Deletes every line without a bookmark, as one undo step. |
| `{"Action": {"ReplaceBookmarkedLines": "TODO"}}` | Replaces the text of each bookmarked line with the given text, keeping the line breaks. (Search > Bookmark > Paste to (Replace) Bookmarked Lines does this with the clipboard.) |

### Searching

These run a search without opening the find panel. The `query` always has
all five options:

| Command | Does |
|---|---|
| `{"Action": {"Find": {"query": {"pattern": "TODO", "options": {"regex": false, "match_case": false, "whole_word": true, "wrap": true, "in_selection": false}}, "backward": false}}}` | Selects the next match (or the previous one with `"backward": true`). |
| `{"Action": {"Replace": {"query": {"pattern": "\\s+$", "options": {"regex": true, "match_case": false, "whole_word": false, "wrap": true, "in_selection": false}}, "replacement": ""}}}` | Replaces the selected match and selects the next one. |
| `{"Action": {"ReplaceAll": {"query": {"pattern": "colour", "options": {"regex": false, "match_case": true, "whole_word": true, "wrap": true, "in_selection": false}}, "replacement": "color"}}}` | Replaces every match, as one undo step. |
| `{"Action": {"SelectAllMatches": {"pattern": "foo", "options": {"regex": false, "match_case": true, "whole_word": true, "wrap": true, "in_selection": false}}}}` | Puts a selection on every match. |
| `{"Action": {"MultiSelectNext": {"match_case": true, "whole_word": true}}}` | Adds the next occurrence of the selected text as another selection (Edit > Multi-select Next). With only a caret, selects the word at it first. No default keys. |
| `{"Action": {"MultiSelectAll": {"match_case": false, "whole_word": true}}}` | Selects every occurrence of the selected text, or of the word at the caret (Edit > Multi-select All). |
| `{"Action": {"BookmarkMatches": {"pattern": "ERROR", "options": {"regex": false, "match_case": true, "whole_word": true, "wrap": true, "in_selection": false}}}}` | Bookmarks every line where a match starts, keeping the bookmarks already there. |

The pattern syntax is described in [Find and replace](find-and-replace.md).
In JSON, a backslash is written twice (`"\\s+$"`).

### Not for key bindings

`SetSelections` and `ColumnSelect` are how mouse clicks and drags select
text, and `Edits` is how a language server's changes (formatting, a rename)
are applied. They take absolute positions, so they make no sense on a key.
