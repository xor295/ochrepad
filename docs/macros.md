# Macros

A macro records what you do in the editor and plays it back, so a repetitive
edit only has to be done once.

## Recording and playing

1. **Macro > Start Recording** (Ctrl+Shift+R). The status bar shows
   `Recording macro (N actions)`.
2. Do the edit once, using the keyboard and the find panel.
3. **Macro > Stop Recording** (Ctrl+Shift+R again).
4. **Macro > Playback** (Ctrl+Shift+P) runs it once at the caret.

**Macro > Run a Macro Multiple Times…** runs the recorded macro, or a saved
one, either a set number of times or **until the end of the file**.

A whole playback, however many times it repeats, is one step for undo, so a
single Ctrl+Z takes it all back.

## What is recorded

ochrepad records what you *meant*, not where it happened. "Go to the end of
the line, type a semicolon, go down a line" replays from wherever the caret
is, on whatever line that is.

Recorded:

- typing, deleting, new lines, tabs;
- caret moves and keyboard selections (arrows, Home/End, word and page
  moves, their Shift variants, Select All);
- adding carets with the keyboard, column selections made with
  Alt+Shift+arrows;
- searches from the find panel, with their options: Find Next/Previous,
  Replace, Replace All and Select All. A macro can say "find the next
  `TODO`, go to the start of its line and type `// `";
- the Edit menu's case conversions and line operations;
- paste, with the text that was pasted (playback pastes that same text, not
  whatever is on the clipboard by then).

Not recorded:

- **mouse clicks and drags**: they mean fixed positions, which would not
  carry over. Move with the keyboard while recording;
- **undo and redo**. Undoing while recording takes back the edit in the
  document, but the macro keeps the step, as in Notepad++;
- copying (cut is recorded as a delete, without the copy);
- opening files, switching tabs and other actions outside the text.

## Running until the end of the file

"Until the end of the file" repeats the macro until one of these happens:

- an action fails, typically a Find with no more matches, which is the
  usual way to stop a find-and-edit macro (searches don't wrap round to the
  start while running to the end, whatever the find panel's Wrap around
  says, so the last match ends it);
- a run starts and ends with the caret at the end of the document, so a
  "fix this line, go down a line" macro processes the last line once and
  stops;
- a run changes nothing, so a macro that has nothing left to do doesn't spin.

## When playback stops early

When you run a macro a set number of times, it stops at the first action
that fails, and ochrepad says which step of which run, for example
`The recorded macro stopped at step 3 (find next "TODO") of run 12: no match found.`
What was done up to there stays, and is still one undo step.

## Saving macros

**Macro > Save Current Recorded Macro…** gives the last recording a name and,
optionally, a shortcut such as `Ctrl+Shift+1` (written the same way as in the
[keymap](keymap.md#writing-keys)). Saving under an existing name replaces that
macro.

Saved macros are listed at the bottom of the **Macro** menu and kept in
`macros.json` in the configuration folder. **Delete Saved Macro** removes one.

A macro's shortcut works while you're editing and takes precedence over a
binding in `keymap.json` for the same keys. It can't replace the fixed
application shortcuts (Ctrl+S, Ctrl+F and the others listed in the keymap
page).
