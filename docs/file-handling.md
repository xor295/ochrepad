# How ochrepad reads and writes files

The rule: a file you open and save without editing comes out byte for byte
the same. ochrepad never silently changes characters, encodings or line
endings, and never leaves a half-written file behind.

## Opening

**Encoding.** ochrepad works out how the text is stored, in this order:

1. A byte order mark (BOM) at the start says it outright: UTF-8, UTF-16 LE or
   UTF-16 BE.
2. Otherwise, if the file is valid UTF-8, it is UTF-8.
3. Otherwise ochrepad guesses the legacy encoding from the bytes (for example
   Windows-1252, Shift_JIS or GBK), the way web browsers do.

The status bar shows the result, for example `UTF-8`, `UTF-8 BOM`,
`UTF-16LE BOM` or `windows-1252`.

**Files ochrepad refuses to open**, with a message saying why:

- **Binary files**: anything with a NUL byte in its first 8 KB (unless it
  has a UTF-16 BOM). Opening and saving a binary file in a text editor would
  corrupt it.
- **Text that isn't valid in its encoding**, for example a file with a UTF-16
  BOM but an odd number of bytes. ochrepad won't replace the bad bytes with
  `�`, because saving would then lose them for good.

UTF-16 files without a BOM contain NUL bytes, so they are treated as binary.

**Line endings.** Windows (CR LF), Unix (LF) and old Macintosh (CR) endings
are all recognised. The status bar shows the file's ending. If a file mixes
them, ochrepad uses the most common one and says
`Mixed line endings (saved as shown)` in the status bar: saving converts every
line to the ending shown.

## New files

A new tab is UTF-8 without a BOM, with Windows line endings on Windows and
Unix line endings elsewhere.

## Saving

**Save** writes the file back with the same encoding, BOM and line endings it
was opened with. **Save As** keeps them too, under the new name.

If the text now contains a character the file's encoding can't store (say,
an emoji in a Windows-1252 file), saving stops with a message naming the
character and its line, and the file on disk is left untouched. ochrepad
doesn't yet have a command to change a file's encoding or line endings.

**Saving is atomic.** ochrepad writes the new contents to a temporary file in
the same folder, flushes it to disk, then swaps it in place of the old file
in one step. If the computer crashes or the disk fills up in the middle, you
have either the old file or the new one, never a mix. The file keeps its
permissions, and if the file is a symbolic link, the file it points to is
updated and the link stays.

Saving happens in the background, so typing never waits for a slow disk. A
tab's `*` goes away when the write has finished. If you close a tab or exit
while its save is still running, ochrepad waits for it, and if the save
fails, the close or exit is cancelled so you can deal with it.

## Pasted text

Text pasted from the clipboard has its line endings converted to the
document's internally, so pasting Windows text into a Unix file doesn't mix
endings.

## Large files

Above 10 MB (**Highlight files up to … MB** in Preferences) syntax
highlighting is turned off, and the status bar says so. Scrolling, typing and
search stay fast in files of 100 MB, the largest size tested. Bigger files
should still open and save correctly, but more slowly.

## Backups and other programs

See [Backups, sessions and files changed elsewhere](backups-and-session.md)
for what happens to unsaved work, and when another program changes a file
you have open.
