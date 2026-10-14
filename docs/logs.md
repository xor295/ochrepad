# Working with logs

ochrepad has three tools for log files, and they work on any file: a filter
that shows only the lines you want, highlight rules that colour lines by
level, and follow mode, which shows what a program appends to a file as it
writes it.

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
