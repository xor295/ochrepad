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
  blue, hints dotted. A dot in the gutter marks a line with an error or
  warning, and the status bar counts them. Resting the pointer on one shows
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
