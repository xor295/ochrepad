# Text tools

**Edit > Text Tools** formats JSON, encodes and decodes Base64, URL
encoding and hex, and shows hashes of text.

## What the tools work on

With a selection, each tool works on every selected piece of text, each on
its own (with several carets or a column selection, every selection is
converted separately). Afterwards each selection covers its new text, so you
can run the opposite tool straight away.

With nothing selected, the tools work on the whole document, leaving out
blank space (spaces, tabs and line breaks) at its start and end. A final
line break stays where it is, and a pasted Base64 string decodes without the
line break you typed after it.

Every run is one undo step, and the tools can be recorded in macros and
bound to keys (see [Key bindings](keymap.md#text-tools)). If a tool cannot
run (invalid JSON, or text that does not decode), nothing changes.

## JSON

- **Format JSON** puts each object member and array element on its own line,
  with a space after each `:`. It indents the way the document does: with
  tabs, or with spaces of the document's indent width, as the status bar
  shows (`Tabs: 4`, `Spaces: 2`). Choose another there before formatting to
  get a different indent. A selection in the middle of a line is indented to
  fit that line: its later lines start with the line's indentation. JSON
  nested more than 1,000 levels deep is not formatted (each level indents
  every line inside it, so the result would be enormous); minifying and
  validating take any depth.
- **Minify JSON** removes every space and line break between the tokens.
- **Validate JSON** checks the JSON without changing it. The status bar says
  when it is valid.

ochrepad uses its own JSON reader, so formatting never changes the data:
object members keep their order (duplicate keys stay as they are), numbers
keep their exact text (`1.50`, `1e10` and `-0` are not rewritten), and
strings are copied byte for byte, escapes included. Only the blank space
between tokens changes. Any JSON value can be formatted, not only objects
and arrays, and there is no limit on how deeply it nests.

The JSON must be strict JSON (RFC 8259): comments, trailing commas, single
quotes and unquoted keys are errors. When the JSON is not valid, Format,
Minify and Validate all move the caret to the first problem and show a
message with its line and column and what is wrong there, for example
`expected ',' or '}' but found '"'`. The column is counted as in the status
bar.

## Base64, URL and hex encoding

- **Base64 Encode** and **Decode** use the standard alphabet (`A`–`Z`,
  `a`–`z`, `0`–`9`, `+`, `/`) with `=` padding. Decoding also accepts the
  URL-safe alphabet (`-` and `_`, as in JSON Web Tokens), missing padding,
  and spaces or line breaks anywhere in the text, so wrapped Base64 decodes
  as it is.
- **URL Encode** and **Decode** use percent-encoding (RFC 3986): every byte
  except the letters, digits, `-`, `.`, `_` and `~` is written as `%XX`, so a
  space becomes `%20`. Decoding turns each `%XX` back into its byte and
  leaves everything else, including `+`, as it is.
- **Hex Encode** and **Decode** (Notepad++'s Converter: ASCII to hex and
  back) write each byte as two hex digits, so `Hi` becomes `4869`.
  Decoding takes upper or lower case, and spaces or line breaks anywhere
  (`48 69` decodes too).

Text is encoded as UTF-8. Line breaks are encoded as the document's line
ending (shown in the status bar), as they are in the file. When decoding,
line breaks of any kind become the editor's.

Decoding is refused, with a note in the status bar, when the text is not
valid Base64, URL encoding or hex, or when the decoded bytes are not UTF-8 text
(for example, an encoded image). ochrepad never inserts replacement
characters for bytes it cannot show.

## Hashes

**Hashes…** shows the MD5, SHA-1 and SHA-256 of the selection, or of the
whole document when nothing is selected, each with a **Copy** button. With
several selections, they are hashed joined by line breaks, as Copy would put
them on the clipboard.

The text is hashed as UTF-8 with the document's line endings, and the dialog
says how many bytes that is. So the hash of a whole document matches a
checksum of its file when the file is UTF-8 without a BOM. To hash it with
other line endings, change them in the status bar first.

MD5 and SHA-1 are no longer safe for security uses; they are here for
comparing with checksums people publish.
