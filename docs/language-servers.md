# Language servers

A language server is a program that understands one language well: it
knows the types in your project, finds mistakes as you type, and knows
where each name is defined. Editors such as VS Code use them; ochrepad can
too, if one is installed. Without one, ochrepad's own
[completion](completion.md) still works.

## Getting one

ochrepad looks for these on `PATH` (the first one found for a language is
used):

| Language | Server | Install |
|---|---|---|
| Rust | `rust-analyzer` | `rustup component add rust-analyzer` |
| Python | `basedpyright`, `pyright`, `pylsp`, `jedi-language-server` or `ty` | for example `pip install basedpyright` |
| Go | `gopls` | `go install golang.org/x/tools/gopls@latest` |
| C, C++, Objective-C | `clangd` | from LLVM, or your package manager |
| JavaScript, TypeScript | `typescript-language-server` | `npm install -g typescript-language-server typescript` |
| Markdown | `marksman` | from its releases page |

Others (dbt's, for example) can be added in `lsp.json`, below.

## Trusting a folder

A language server reads your project and can run some of its code:
rust-analyzer runs build scripts and procedural macros, for example. So the
first time a server would start for a folder, ochrepad asks:

- **Start in this folder**: servers may run there from now on.
- **Not in this folder**: they never do (until you say otherwise).
- **Never start** *server*: that server never runs anywhere.

The folder is the open folder (**File > Open Folder**) when the file is in
it; otherwise the nearest folder above the file with a project file
(`Cargo.toml`, `pyproject.toml`, `package.json`, `go.mod`, `.git`, …).
**View > Language Servers** lists the folders you trusted, with **Forget**.

ochrepad only runs a server it found itself in a `PATH` folder outside the
project, so a program planted in a repository is never run.

## What it shows

- **Completion**: the server's suggestions come first in the
  [completion](completion.md) list, above ochrepad's own, with the selected
  one's documentation beside the list. Some insert more than the name: a
  suggestion for a name not imported yet also adds the import (one undo
  step for both). Placeholders in a server's templates are inserted as
  plain text.
- **Problems** are underlined: errors red, warnings yellow, information
  blue, hints dotted. A dot in the gutter and a mark on the scrollbar show
  where errors and warnings are, and the status bar counts them. Resting the pointer on one shows
  its message. **Alt+F8** and **Alt+Shift+F8** move to the next and
  previous problem. **View > Problems** lists every file's problems; a click
  opens the file there.
- **Hover**: resting the pointer on a name shows what the server knows
  about it (its type, its documentation).
- **Go to Definition** (**F12**, **Search > Go to Definition**, or the
  right-click menu): opens the file where the name at the caret is defined,
  at the definition, even in a library's source. Only files on this
  computer open.
- **Find References** (**Shift+F12**, **Search > Find References**, or the
  right-click menu): lists everywhere the name at the caret is used, in the
  Search Results panel, like Find in Files. A click goes there.
- **Go to Symbol in Folder** (**Ctrl+Alt+T**, **Search > Go to Symbol in
  Folder…**): type part of a name to search the whole project's functions,
  types and constants; Up, Down and Enter (or a click) open one.
- The **Function List** (View > Function List) lists what the server
  finds in the document.

## Changing code

- **Rename** (**Shift+F6**, **Edit > Rename…**, or the right-click menu):
  renames the name at the caret everywhere the server knows it is used.
  Each document's changes are one undo step. Files that were not open
  open in tabs with the changes, not saved yet, so you can look before
  saving (File > Save All). Nothing changes unless every change fits: if
  a document changed after the server looked, nothing is applied and you
  can try again.
- **Format Document** (**Alt+Shift+F**, **Edit > Format Document**, or
  the right-click menu): the server's formatter (rustfmt through
  rust-analyzer, for example), as one undo step. **Format on save** in
  View > Language Servers makes Ctrl+S format first; a server that takes
  more than two seconds is not waited for (the file is saved as it is).

- **Code Actions** (**Ctrl+.**, **Edit > Code Actions…**, or the
  right-click menu): the server's fixes and refactorings for the selection
  or the caret (add a missing import, inline a variable, extract a
  function), in a menu at the caret. The server's preferred fix is
  selected; Up, Down and Enter choose, Esc closes. Greyed items say why
  they cannot be used here. Their changes apply like Rename's. Some
  actions are commands only an editor extension can run (a test runner, a
  debugger); ochrepad says so instead.

A server can ask to create, rename or delete files as part of a change;
ochrepad refuses that, and applies a server's changes only when you asked
for them (a code action's command may ask for edits while it runs; at any
other time they are refused).
- **Signature help**: typing `(` or `,` in a call shows the function's
  signature above the caret (below it when there is no room), with the
  argument you are typing picked out and its documentation. It follows the
  caret while you stay in the call; **Esc** closes it and
  **Ctrl+Shift+Space** shows it again.

## View > Language Servers

Each running server, its folder and state (starting, indexing, running,
stopped), with **Restart**, **Stop** and its log (what it printed, and what
ochrepad told it). A server that crashes is started again, up to three
times. **Use language servers** turns them all off.

## lsp.json

**Edit lsp.json…** in that window opens `lsp.json` in ochrepad's
configuration folder, starting from an example. Saving it applies it.

```json
{
  "servers": [
    {
      "name": "dbt",
      "command": ["dbt-language-server", "--stdio"],
      "languages": ["SQL"],
      "settings": {},
      "initializationOptions": null
    }
  ],
  "disabled": ["pylsp"]
}
```

- `name`: a server's name; one named like a built-in one replaces it.
- `command`: the program and its arguments. A bare name is looked for on
  `PATH`; anything else must be an absolute path.
- `languages`: ochrepad's names for them, as the status bar shows them.
- `settings`: what the server is told when it asks for its settings, by
  section (`{"rust-analyzer": {"check": {"command": "clippy"}}}`).
- `initializationOptions`: sent when it starts.
- `disabled`: servers never started.

Files larger than **Highlight files up to** (Preferences) are not given to
servers.
