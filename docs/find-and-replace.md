# Find and replace

Open the panel with **Ctrl+F** (find) or **Ctrl+H** (find and replace). A one-line
selection becomes the search text. The search text is selected each time, so
pressing **Ctrl+F** and typing always starts a new search, even if the panel
was already open. **Enter** / **F3** finds the next match,
**Shift+Enter** / **Shift+F3** the previous one, **Esc** closes the panel.

**Ctrl+F3** (Search > Select and Find Next) searches for the selected text,
or the word at the caret, without opening the panel, and **Ctrl+Shift+F3**
backwards; F3 and Shift+F3 then go on with it. The panel's options apply.

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

**Bookmark** bookmarks every line with a match, so F2 and Shift+F2 can go
from one to the next ([Bookmarks](editing.md#bookmarks)).

## Find All and the Search Results panel

**Find All in Current Document** and **Find All in All Open Documents** list
every match in the **Search Results** panel above the find panel, grouped by
document, one row per line with the matches tinted. Click a row (or focus it
and press Enter) to go to the match; its tab comes to the front.

- The search runs in the background, so a very large document does not
  freeze the window; the panel says "searching…" until it is done.
- A search lists at most 10,000 matches and says so when there were more.
- **In selection** applies to Find All in Current Document only.
- Each search is added at the top of the panel and the older ones are
  folded away (click a heading to open or close it). The panel keeps the
  last 10 searches; **Clear** removes them and **Close** hides the panel.
- Results are not updated as you edit. If a document has changed since the
  search, clicking a result goes to the same line number and selects the
  match nearest to where it was on that line, or the start of the line if
  there is none.
- Unsaved documents are searched as they are in the editor.

## Find in Files

**Search > Find in Files…** (Ctrl+Shift+F), or **In Files…** in the find
panel, adds a row for searching the files in a folder on disk:

- **Folder**: where to search. It starts as the open folder (see
  [Working in a folder](workspace.md)), or the active file's folder; right-click
  a folder in the sidebar and choose **Find in Folder…** to search just that
  one. **Browse…** picks one.
- **Filters**: which files, as globs separated by `;` (or spaces or
  commas), such as `*.rs; *.toml`. Empty means every file.
- **Subfolders**: search the folders inside too (on by default).
- **Ignored files**: also search files that `.gitignore` and `.ignore` leave
  out. The `.git` folder is never searched.

Press **Find in Files**. Matches appear in the Search Results panel as files
are searched, with the number of files searched so far; **Stop** ends the
search and keeps what it found. Clicking a result in a file that is not open
opens it.

- Files are read with the same encoding detection as File > Open, so
  UTF-16 and legacy encodings are found too. Binary files and files that
  cannot be decoded are skipped without a message. Files over 50 MB are
  skipped and counted in the results heading.
- A file that is open in a tab is searched as it is in the editor, unsaved
  changes included, not as it is on disk.
- **In selection** does not apply. Find in Files only finds; there is no
  replace for files that are not open.

**Replace All in All Open Documents** (in replace mode) first counts the
matches and asks before replacing. Each document gets its own undo step,
the documents become unsaved, and nothing is written to disk until you save.
It is not recorded in a macro.

## Styling tokens

**Search > Style All Occurrences of Token** highlights the selected text,
or the word at the caret, everywhere in the document in one of five colours
(sky, violet, yellow, rose and green), as in Notepad++. The right-click
menu has it too. Styling a token again changes its colour. Case matters, and
a word only matches whole words.

- **Search > Clear Style** removes one style's highlights, or all of them.
- **Search > Jump Down** and **Jump Up** select the next or previous
  occurrence of a style's tokens, going round the document.

Unlike Notepad++, an occurrence you type later is highlighted too. Styles
belong to the document and last until you clear them or close it.

## Smart highlighting

Selecting text on one line highlights every other place the same text appears,
as in Notepad++, so you can scan for it without opening the panel. The match is
exact: case-sensitive, and not limited to whole words. Blank selections and
selections over several lines are not highlighted.

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
