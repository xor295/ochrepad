# The command palette

**Ctrl+Shift+A** opens a list of every command ochrepad has, in the middle
of the window near the top. Type a few letters of a command's name, press
**Enter**, and it runs. It is the quick way to a command you know exists
but that sits three menus deep or has no shortcut.

The **Commands** button at the right end of the menu bar and **? > Command
Palette…** open it too.

## Finding a command

Commands are named by where they are in the menus:

```
Edit: Line Operations: Sort Lines Ascending
View: Word Wrap
Search: Bookmark: Toggle Bookmark
```

What you type matches anywhere in the name, the way the
[completion list](completion.md) matches:

- part of a word: `wrap` finds **View: Word Wrap**;
- several parts, with or without spaces: `sort asc` finds **Sort Lines
  Ascending**;
- the first letters of the words: `sld` finds **Sort Lines Descending**.

The letters that matched are coloured, and the best match is first.

| Key | Does |
|---|---|
| Up, Down | Move in the list (and around its ends) |
| PageUp, PageDown | Move a page |
| Enter | Run the selected command |
| Escape, or Ctrl+Shift+A again | Close the palette |

A click runs a command as well, and a click anywhere outside closes the
palette.

## What the list shows

- **The shortcut**, at the right of each command that has one. For editing
  commands it is the key your [keymap](keymap.md) gives them now.
- **on** or **off** after a setting, such as **View: Word Wrap**; running
  it switches the setting.
- With nothing typed: every command in the menus' order, after the ones
  you ran most recently (remembered until you close ochrepad).

Besides the menus' commands the list has **Language: …** for every syntax
(as the status bar's language menu), **View: Syntax theme: …**, your saved
macros (**Macro: …**) and **Compare: Compare With: …** for each other open
document.

Only what applies is listed. While a terminal has the focus the editing
commands are left out, and a file open in the read-only viewer has **Edit:
Copy** but not the commands that change text. A command that needs
something that is missing (**File: Rename…** for a document without a
file, the bookmark commands without a bookmark) is left out as well.

## Why Ctrl+Shift+A

Ctrl+Shift+P, which other editors use, plays the recorded macro in
Notepad++, and so it does here. Ctrl+Shift+A is the key for the same kind
of list ("Find Action") in JetBrains' editors. It works in a terminal tab
too, where it is one of the few keys ochrepad keeps for itself.
