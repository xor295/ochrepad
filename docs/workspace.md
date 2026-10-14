# Working in a folder

Open a folder to see its files in a sidebar on the left and open them from
there, as in VS Code's Explorer or Notepad++'s Folder as Workspace.

## Opening a folder

- **File > Open Folder…**, or drop a folder onto the window.
- From the command line: `ochrepad path/to/folder` (files and folders can be
  mixed: `ochrepad . notes.txt`).
- **File > Recent Folders** lists the last ten. **File > Close Folder**
  closes it (open tabs stay open).
- The folder, the folders you had expanded, and the sidebar's width come
  back with the session the next time you start ochrepad.

**View > Sidebar** (Ctrl+Shift+E) hides or shows the sidebar.

## The file tree

- Folders come first, then files, in natural order (`file2` before
  `file10`, ignoring case).
- Click a folder to expand or collapse it. Its contents are read in the
  background, so a slow network drive never freezes the window; it shows
  "Loading…" until they arrive.
- **Double-click** a file (or select it and press **Enter**) to open it.
- The active tab's file is marked with an ochre bar. **View > Reveal Active
  File in Sidebar** expands the folders down to it and selects it.
- With the tree focused (click a row), **Up/Down** move the selection,
  **Right/Left** expand and collapse folders (or move into and out of
  them), **F2** renames, and **Esc** goes back to the editor.
- The tree follows changes on disk: files that other programs create,
  rename or delete in the root folder or an expanded folder appear and
  disappear by themselves.

### Ignored files

Files and folders ignored by `.gitignore` (including those in parent
folders, `.git/info/exclude` and git's global excludes) and `.ignore` files
are hidden, and so is the `.git` folder. This works whether or not the
folder is a git repository. Other dot-files, such as `.gitignore` itself,
are shown.

**Folder > Show Ignored Files** (the button at the top of the sidebar)
shows them too, greyed out.

## File operations

Right-click a file or folder, or use the **Folder** menu at the top of the
sidebar (or right-click the empty space below the files):

- **New File… / New Folder…** create one in the folder you clicked (or the
  folder of the file you clicked). A new file opens in a tab straight away.
  An existing name is never overwritten.
- **Rename** (or **F2**) edits the name in place; **Enter** renames,
  **Esc** cancels. Renaming never replaces another file. If the file is
  open, its tab follows the new name; renaming a folder moves the tabs of
  every open file inside it.
- **Copy Path / Copy Relative Path** put the full path, or the path from
  the folder you opened, on the clipboard.
- **Reveal in File Manager** shows the file in Explorer on Windows, or opens
  its folder in your file manager on Linux.
- **Find in Folder…** opens Find in Files on that folder (see
  [Find and replace](find-and-replace.md#find-in-files)).
- **Refresh** reads every listed folder again; **Collapse All** closes them.

Deleting files from the tree is not supported; use your file manager.

Symbolic links are listed but not followed into.
