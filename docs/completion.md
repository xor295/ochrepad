# Completion

While you type a word, ochrepad shows a list of ways to finish it: words and
names from your documents and from the open folder, the language's keywords,
and the functions, types and modules the syntax highlighting finds. It needs
nothing installed and works in every language.

## Using it

The list opens by itself once you have typed two characters of a word that
something matches, and **Ctrl+Space** opens it at any time (with nothing
typed yet, it shows everything it has, best first).

| Keys | |
|---|---|
| Up, Down, PageUp, PageDown | Move in the list |
| Enter | Insert the selected item |
| Tab | Insert it, and replace the rest of the word after the caret |
| Esc | Close the list |
| Click | Insert that item |

Typing goes on as usual and narrows the list; moving the caret out of the
word, or typing something that is not part of a word, closes it. Inserting
a completion is one undo step. With several carets, each one's word is
completed the same way.

The list does not open by itself inside comments and strings (Ctrl+Space
still works there), and never in a document that is read-only.

## What matches

You do not have to type the start of a name:

| You type | Matches |
|---|---|
| `get` | `getText`, `get_items`: names that start with it come first |
| `gtl` | `getTextLength`, `get_text_length`, `GTLookup`: the first letters of a name's parts |
| `txtlen` | `get_text_length`: the letters in order, from the start of a part (three letters or more) |

The matched letters are drawn in the accent colour. Matching ignores case;
the same case ranks a little higher.

## Where suggestions come from

| Marker | Source |
|---|---|
| `f`, `T`, `m` | A function, type or module defined in an open document or in the open folder (the detail shows its line or its file) |
| `k` | A keyword of the document's language |
| `w` | A word from the document, another open document or the open folder |

Words near the caret rank higher, then words used often, then words you
picked recently. Words shorter than three characters are not offered.

When a folder is open (**File > Open Folder**), ochrepad reads its files in
the background, respecting `.gitignore`, and the status bar says so while
it does. Files over 2 MB, binary files and files past the first 20,000 are
left out. What it learns is kept in ochrepad's data folder, so the next time
the folder opens it only reads the files that changed. A file you save is
read again at once; files other programs change (a build, `git`, the
terminal) are noticed within 30 seconds, or when ochrepad's window comes
back into focus.

## Settings

**Settings > Preferences > Completion** turns completion on or off, sets
whether the list opens while you type and after how many characters (1 to
5), and whether Enter inserts a suggestion (Tab always does; with Enter off,
Enter always breaks the line).

The key that opens the list is `"ShowCompletions"` in the keymap (see
[Key bindings](keymap.md)).
