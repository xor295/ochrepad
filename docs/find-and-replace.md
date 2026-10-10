# Find and replace

Open the panel with **Ctrl+F** (find) or **Ctrl+H** (find and replace). A one-line
selection becomes the search text. **Enter** / **F3** finds the next match,
**Shift+Enter** / **Shift+F3** the previous one, **Esc** closes the panel.

Options:

- **Regular expression**: see the dialect notes below. Off means the text is
  matched literally.
- **Match case**: off by default.
- **Whole word**: in literal mode a word boundary is only required on a side
  where the text starts or ends with a word character, so `foo(` still matches
  as a whole word.
- **Wrap around**: on by default, as in Notepad++.
- **In selection**: Count, Select All and Replace All only look inside the
  selection(s). Find Next/Previous always search the whole document.

**Select All** turns every match into a selection, so typing edits them all.
**Replace All** is a single undo step.

## Regular expression dialect

ochrepad uses Rust's `regex` syntax (through `regex-cursor`), not the Boost
syntax Notepad++ uses. The main differences:

| | ochrepad | Notepad++ |
|---|---|---|
| Lookahead / lookbehind `(?=…)`, `(?<=…)` | not supported | supported |
| Backreferences in the pattern `\1` | not supported | supported |
| `^` and `$` | start and end of each line | same |
| `.` matches a newline | no | optional |
| Named groups | `(?<name>…)` or `(?P<name>…)` | `(?<name>…)` |
| Groups in the replacement | `$1`, `${1}`, `${name}`, `$0` for the whole match, `$$` for `$` | `$1`, `\1` |
| Escapes in the replacement | `\n`, `\t`, `\\` | same, plus more |

Patterns using unsupported syntax are reported as invalid while you type.

Literal (non-regex) replacements are inserted as-is: `$1` and `\n` are not
special.
