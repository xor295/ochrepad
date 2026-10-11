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

## The session file

The list of open tabs and the layout are kept in `session.json` in the
configuration folder (`%APPDATA%\ochrepad\config` on Windows,
`~/.config/ochrepad` on Linux). It is updated on the backup timer and when ochrepad exits.
Markdown previews and comparisons are not part of the session; reopen them
from the menus.

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
