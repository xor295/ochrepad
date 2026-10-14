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
  Ctrl+W, Ctrl+Tab, Ctrl+Shift+Tab, Ctrl+F, Ctrl+H, F3, Shift+F3,
  Ctrl+Shift+R, Ctrl+Shift+P, Ctrl+Shift+M, F8 and Shift+F8.
- **Copy, cut and paste** (Ctrl+C, Ctrl+X, Ctrl+V) come from the system
  clipboard, not from key bindings.
- **A saved macro's shortcut** takes precedence over a keymap binding for the
  same keys (see [Macros](macros.md)).
- The keymap applies while the editor has the keyboard. In the find panel or
  a dialog, keys go to that field.

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
| `"Up"`, `"Down"` | the line above or below, keeping the column you started from |
| `"Home"` | the first non-blank character of the line, or the line start if already there |
| `"LineStart"`, `"LineEnd"` | the very start or end of the line |
| `"DocStart"`, `"DocEnd"` | the start or end of the document |
| `{"PageUp": 20}`, `{"PageDown": 20}` | the given number of lines up or down |

### Typing and deleting

| Command | Does |
|---|---|
| `{"Action": "InsertNewline"}` | A line break, keeping the current line's indentation. |
| `{"Action": "InsertTab"}` | A tab, or spaces up to the next tab stop when **Indent with: Spaces** is set. |
| `{"Action": {"InsertText": "→"}}` | Types the given text at every caret. |
| `{"Action": {"Paste": "TODO: "}}` | Inserts fixed text like paste does: with several carets and as many lines as carets, one line goes to each. |
| `{"Action": {"PasteBlock": "a\nb\nc"}}` | Inserts the lines as a column block, one per line from the caret down. |
| `{"Action": "DeleteBackward"}` | Backspace. |
| `{"Action": "DeleteForward"}` | Delete. |
| `{"Action": "DeleteWordBackward"}` | Deletes to the previous word boundary. |
| `{"Action": "DeleteWordForward"}` | Deletes to the next word boundary. |
| `{"Action": "Undo"}` | Undo. |
| `{"Action": "Redo"}` | Redo. |

### Lines and case

| Command | Does |
|---|---|
| `{"Action": "DuplicateLine"}` | Duplicates the caret's line, or each selection. |
| `{"Action": "MoveLinesUp"}` | Moves the selected lines up one line. |
| `{"Action": "MoveLinesDown"}` | Moves the selected lines down one line. |
| `{"Action": "JoinLines"}` | Joins the selected lines (or the caret's line and the next). |
| `{"Action": "TrimTrailingWhitespace"}` | Removes trailing spaces and tabs from the selected lines, or from every line when nothing is selected. |
| `{"Action": {"RemoveEmptyLines": {"include_whitespace": false}}}` | Removes empty lines from the selected lines, or from the whole document when nothing is selected. With `include_whitespace`, lines of only spaces and tabs go too. |
| `{"Action": {"RemoveDuplicateLines": {"consecutive_only": false}}}` | Removes lines equal to an earlier line, keeping the first one and the order, in the selected lines or the whole document. Lines compare exactly. With `consecutive_only`, only a line equal to the one right before it is removed. |
| `{"Action": {"ConvertCase": "Upper"}}` | Converts the selections (or the caret's line) to `"Upper"`, `"Lower"`, `"Title"` or `"Invert"` case. |
| `{"Action": {"SortLines": {"descending": false, "case_insensitive": true, "numeric": false, "remove_duplicates": false}}}` | Sorts the selected lines, or the whole document when nothing is selected. All four options must be given. `numeric` compares the number at the start of each line; `remove_duplicates` drops repeated lines. With a column selection, lines are sorted by the text inside the column. |

### Searching

These run a search without opening the find panel. The `query` always has
all five options:

| Command | Does |
|---|---|
| `{"Action": {"Find": {"query": {"pattern": "TODO", "options": {"regex": false, "match_case": false, "whole_word": true, "wrap": true, "in_selection": false}}, "backward": false}}}` | Selects the next match (or the previous one with `"backward": true`). |
| `{"Action": {"Replace": {"query": {"pattern": "\\s+$", "options": {"regex": true, "match_case": false, "whole_word": false, "wrap": true, "in_selection": false}}, "replacement": ""}}}` | Replaces the selected match and selects the next one. |
| `{"Action": {"ReplaceAll": {"query": {"pattern": "colour", "options": {"regex": false, "match_case": true, "whole_word": true, "wrap": true, "in_selection": false}}, "replacement": "color"}}}` | Replaces every match, as one undo step. |
| `{"Action": {"SelectAllMatches": {"pattern": "foo", "options": {"regex": false, "match_case": true, "whole_word": true, "wrap": true, "in_selection": false}}}}` | Puts a selection on every match. |

The pattern syntax is described in [Find and replace](find-and-replace.md).
In JSON, a backslash is written twice (`"\\s+$"`).

### Not for key bindings

`SetSelections` and `ColumnSelect` are how mouse clicks and drags select
text. They take absolute positions, so they make no sense on a key.
