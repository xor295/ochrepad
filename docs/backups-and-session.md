# Backups, sessions and files changed elsewhere

Like Notepad++, ochrepad's "autosave" is a backup system: your unsaved work is
copied somewhere safe every few seconds, but your files are only changed when
you save them, unless you turn on automatic saving.

Both options are in the **Settings** menu and in **Settings > Preferences**.

## Backups (on by default)

Every 7 seconds (**Autosave every … seconds** in Preferences), each tab with
unsaved changes is copied to the backup folder:

| | |
|---|---|
| Windows | `%APPDATA%\ochrepad\data\backup` |
| Linux | `~/.local/share/ochrepad/backup` |

A backup is two files: the text (`<id>.txt`, UTF-8) and its details
(`<id>.json`: the file's path, encoding, line endings and language). Only tabs
that changed since their last backup are written, and writing happens in the
background, so typing never waits for the disk.

A backup is deleted when it is no longer needed: when you save the file,
undo back to the saved version, or close the tab and choose **Don't Save**.

### Closing ochrepad with backups on

Exiting does **not** ask about unsaved files. ochrepad backs up every
unsaved tab, records the open tabs, and closes. Next time it starts, the same
tabs come back in the same layout, with the carets and scroll positions you
left, and the unsaved ones still marked unsaved (`*`), with your changes.
New tabs that were never saved come back too, with their names (`new 3`).

Closing a single tab still asks whether to save it.

If a file from the last session can't be opened any more (deleted, moved, or
on a drive that isn't there), ochrepad says which and opens the rest.

If a file changed on disk while ochrepad was closed:

- a tab that had no unsaved changes opens the new version;
- a tab that had unsaved changes comes back with **your** version, without a
  warning. Save it to keep it, or close it with **Don't Save** and reopen the
  file to get the version on disk.

### With backups off

Nothing is copied while you work. Exiting asks about each unsaved file
(**Save**, **Don't Save** or **Cancel**, where Cancel stops the exit). The
open tabs are still remembered, but only files that have been saved come back.

## Saving files automatically (off by default)

**Save files automatically** also saves every tab that has a file name, every
few seconds and whenever the ochrepad window loses focus (when you switch to
another program). Tabs that have never been saved are left alone; backups
cover those.

Saving works the same way as **Save**: the file keeps its encoding and line
endings, and is replaced in one step (see [File handling](file-handling.md)).

## Undo after closing

Each file's undo history is kept when you close it or ochrepad, so **Undo**
still works after you open it again: back through the edits you made in
earlier sessions, and **Redo** forward again. Unsaved work restored from a
backup comes back with its history too.

- A history is used only if the file is exactly as it was when the history
  was kept. If another program changed it in between, the file opens with
  no history (nothing is undone into text that is not there).
- Up to about 2 MB of the newest changes are kept per file, and the histories
  of the 300 most recently edited files. A single change bigger than that
  (replacing a whole large file, say) ends the kept history there.
- Undoing back to the text on disk makes the tab clean again, as usual.
- **Edit > Clear Undo History** forgets the current document's history,
  also the copy on disk. **Settings > Preferences > Undo history** turns
  keeping it off.
- The histories are in the data folder, under `history`
  (`%APPDATA%\ochrepad\data\history` on Windows,
  `~/.local/share/ochrepad/history` on Linux).

## If ochrepad crashes

If ochrepad hits a bug and has to close, it first backs up every unsaved tab
and writes the session, as on exit, and then says so in a message. The next
start brings the work back. (With backups turned off there is nothing to
bring back, and the message says that too.)

## The session file

The list of open tabs and the layout are kept in `session.json` in the
configuration folder (`%APPDATA%\ochrepad\config` on Windows,
`~/.config/ochrepad` on Linux). It is updated on the backup timer and when ochrepad exits.
Markdown previews and comparisons are not part of the session; reopen them
from the menus.

## One ochrepad at a time

Only one ochrepad runs at a time, so two windows never overwrite each
other's session. Opening a file while ochrepad is running (from **Open
with**, a double-click on a file, or `ochrepad notes.txt` in a terminal)
opens it in a new tab of the running window and brings that window
forward; starting ochrepad again without a file just brings the window
forward. Folders open in its sidebar, and relative paths are taken from the
folder the command was run in.

(Windows can refuse to let one program bring another's window forward; the
taskbar button flashes instead.)

## Session files

Besides the session ochrepad keeps by itself, you can save sets of files to
open together later, as in Notepad++:

- **File > Save Session…** saves the open files to a file you name
  (`session.json` is suggested): each file's path, in tab order, with its
  caret and the first line on screen, which one is active, and the open
  folder. Tabs that were never saved have no file to list, so they are left
  out, and the status bar says how many.
- **File > Load Session…** opens the files a session file lists, beside the
  tabs already open (none are closed). Each opens as with **File > Open**: a
  file that is already open is switched to, and a large one opens in the
  viewer. The files come back with their carets and scroll positions, the
  folder opens in the sidebar, and the session's active file becomes the
  active tab. Files that are missing or can't be opened are listed in one
  message afterwards; the others still open.

A session file is JSON, so you can also write or change one by hand:

```json
{
  "version": 1,
  "files": [
    { "path": "C:\\Users\\me\\project\\src\\main.rs", "caret": 1520, "first_line": 30 },
    { "path": "notes.md" }
  ],
  "active": 0,
  "folder": "C:\\Users\\me\\project"
}
```

- `version` is required, and is 1. A file from a later version of
  ochrepad, with a higher number, is refused.
- A relative `path`, such as `notes.md` here, is taken from the session
  file's folder. (Save Session writes full paths.)
- `caret` is where the caret goes, in bytes from the start of the text
  (UTF-8, with each line break counting as one); past the end it goes to the
  end. `first_line` is the first line on screen, counting from 0.
- `active` is the active file's place in the list, counting from 0.
- Everything but `version` and each file's `path` can be left out.

A session file may come from someone else, so ochrepad reads one only if it
is no larger than 1 MB, opens at most 200 of its files, and skips (and
lists) an entry it can't read. On
Windows it also opens only files on this computer's drives: a network path
such as `\\server\share\notes.txt` is skipped as "not on this computer",
because opening it would send your Windows credentials to that server.

## Files changed by other programs

ochrepad watches the folders of the files you have open. When another program
changes or deletes one of them, ochrepad asks what to do, a moment after the
change (so a program that writes a file in several steps is only noticed
once):

| What happened | You can |
|---|---|
| A file with no unsaved changes was changed | **Reload** it, or **Keep this version** |
| A file with unsaved changes was changed | **Reload** it and lose your changes, or **Keep my changes** |
| A file was deleted or moved away | **Keep open** (the tab becomes unsaved, so saving writes it back) or **Close** |

Reloading can be undone with Ctrl+Z, which brings back the text you had.
Saving from ochrepad itself never triggers these questions.
