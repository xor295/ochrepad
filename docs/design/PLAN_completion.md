# Plan: code completion and language servers

Written 2026-10-25, after the terminal. `CLAUDE.md` is the current guide;
read its architecture rules before starting. This plan adds completion as
you type that needs nothing installed, then a client for language servers
(LSP) for the languages where one is installed, and, last and only if it
passes a strict review, completion from a small local language model. Where
it changes a rule in `CLAUDE.md`, the step that makes the change updates it.

## 1. Goal and scope

1. **Completion with nothing installed** (the default, for every language):
   a popup at the caret with words and names from the document, the other
   open documents and the open folder, the language's keywords, and the
   definitions the syntax highlighting finds (what the Function List shows),
   ranked and matched loosely (`gTL` finds `getTextLength`). No popup in
   comments; file paths inside strings.
2. **Language servers** where the user has one installed (`rust-analyzer`,
   `pyright`, `gopls`, `clangd`, …): their completions in the same popup,
   errors and warnings in the text, the gutter and a Problems panel, hover
   information, go to definition, signature help; then references, rename,
   formatting and code actions.
3. **Local model completion (maybe never).** A gate, not a feature: section
   6 lists what must hold before anything of it is built, and it is dropped
   at the first sign of trouble.

### Not in this plan

- Downloading or installing language servers (ochrepad finds them on `PATH`
  and says what to install; it never fetches one).
- Reading a project's editor settings (`.vscode/`, `.idea/`) for server
  commands or anything else: a folder never makes ochrepad run a program.
- Debugging (DAP), semantic highlighting, inlay hints, call hierarchies.
- Tree-sitter or any parser per language: the built-in completion works from
  syntect's scopes and plain text. (Revisit only if section 4's results are
  clearly not good enough.)
- Cloud models or any network service for completion.

## 2. Decisions

| Question | Decision |
|---|---|
| Order | Built-in completion, then the LSP client, then (maybe) the local model gate. Each step is useful alone. |
| When the popup opens | By itself once 2 characters of a word are typed and something matches beyond them; Ctrl+Space at any time. A preference: after 1 or 2 or 3 characters, or only on Ctrl+Space. |
| Accepting | Tab or Enter (a preference can make Enter only insert a line break). Esc closes. |
| Starting language servers | Asked once per folder, before a server first runs there, and remembered: servers run project code (rust-analyzer runs build scripts and procedural macros). The question names the program and the folder. |
| Finding servers | A built-in table of the usual servers per language, looked for on `PATH`; `lsp.json` in the config folder overrides or adds entries (command, arguments, languages, settings to send). Never a path from the project. |
| Resolving a command | ochrepad resolves the program to an absolute path itself, from `PATH` entries that are absolute and outside the open folder, so a `rust-analyzer.exe` planted in a repository is never run (Windows' process search would otherwise look in the current folder). |
| Dependencies | `lsp-types =0.97.0` (approved): the protocol's types; pure Rust, brings `fluent-uri`, `serde_repr` and an older `bitflags` 1.x. JSON-RPC framing and the process handling are ours (std only). Nothing else. |
| Where code lives | Matching, ranking, the word index and LSP position conversions in `ochrepad-core` (no file access, no egui). Keyword tables and the scope at a position in `ochrepad-syntax`. Walking the folder in `ochrepad-fileio`. A new crate `ochrepad-lsp` (no egui) for the client: processes, framing, requests, document sync. The popup, the Problems panel and wiring in the app. |
| Edits from completion and servers | Through `Editor::apply` as Actions, as every edit: one undo step each, mapped to every caret where that makes sense. A server can never write a file: edits to files that are not open (a rename) open them in tabs, unsaved, for the user to review and save. |
| Keys | Ctrl+Space complete; F12 go to definition (Ctrl+click already adds a caret); Shift+F12 find references; Shift+F6 rename (F2 is Notepad++'s next bookmark); Ctrl+. code actions; Ctrl+Shift+Space signature help; Alt+Shift+F format document. All free today; all go through the keymap. |
| Positions | The rule that byte offsets are canonical stays. LSP positions are (line, column in UTF-16 units) unless the server accepts UTF-8 (LSP 3.17 `positionEncodings`; rust-analyzer and clangd do). Conversions in `core::position`, tested both ways like the others there. |

## 3. Shared foundation (step C1)

### The popup

- Drawn by the editor widget, anchored below the caret (above it near the
  bottom of the pane), using `LineLayout` for the caret's x as everything
  else does. At most ~10 rows, scrolling; each row has a kind marker
  (keyword, function, type, module, variable, word, path, snippet), the
  label with the matched characters in the accent colour, and a dim detail
  (the file a name was found in, or the server's detail).
- While it is open it takes Up, Down, PageUp, PageDown, Tab, Enter and Esc
  before the keymap; everything else goes on as usual and updates the list.
  It closes when the caret leaves the word, on a click elsewhere, on focus
  loss, and when nothing matches.
- With several carets it lists for the primary caret, and accepting changes
  the same text before every caret that has it (VS Code's behaviour).
- Not in the viewer, not in read-only documents, not in a column selection.

### The Action

- `Action::Complete { before: usize, after: usize, text: String }`: replace
  `before` characters before each caret and `after` characters after it
  with `text`, as one undo step, recorded in macros like typing.
- `Action::Edits(Vec<TextEdit>)` for servers' edits (additional edits of a
  completion, rename, formatting, code actions): absolute byte ranges,
  checked against the document (each on char boundaries, sorted, not
  overlapping) before anything changes; refused as a whole otherwise. Not
  recorded in macros (they name absolute places).
- Both go through `resolve`; proptest covers them in `props.rs` with the
  other actions.

### Matching and ranking (`core::complete`)

- A candidate matches the typed prefix as: exact prefix, then case-insensitive
  prefix, then word-start initials (`gTL`, `get_t_l`), then any subsequence
  (only for 3+ typed characters). Each gives a score; ties break on the
  source (server > definitions > keywords > words), how near the caret a
  word was seen, how often it is used, and how recently the user accepted
  it (kept per folder in the data folder, last 500).
- Candidates are deduplicated by label; a candidate equal to the typed word
  is dropped.
- Budget: a list in under 2 ms for 50,000 candidates on this machine in a
  release build; an example bench (`complete_bench`) measures it.

## 4. Built-in completion (steps C2–C4)

### C2: the document, open documents and keywords

- Words: maximal runs of letters, digits and `_` (and `-` in CSS, Lisp and
  other languages where names have it, from a small table), at least 3
  characters, not all digits. From the current document (weighted by
  distance from the caret, for documents up to the highlight size limit) and
  the other open documents. Kept per buffer, rebuilt on a thread after a
  pause in typing, like the change history.
- Keywords: `ochrepad-syntax/src/keywords.rs`, a table per language for the
  languages people use most (Rust, Python, JavaScript/TypeScript, Go, C,
  C++, C#, Java, Kotlin, Swift, PHP, Ruby, Lua, SQL, shell, PowerShell, HTML,
  CSS, YAML keys are words), with a test that every entry names a real
  language (as `comments.rs` has).
- Context from the syntax: `BufferHighlighter::scope_at(doc, offset)`
  re-parses the caret's line from the nearest checkpoint (at most 128 lines
  back) and says whether the caret is in a comment, a string, or code. No
  popup in comments; in strings, only paths (C4).

### C3: the open folder

- An index of the open folder, built on a thread when a folder is opened:
  every file `walk_files` gives (`.gitignore` respected), up to 2 MB a file
  and 20,000 files, binary files skipped. Per file: its words with counts,
  and its definitions from `Registry::symbols` (name, kind, line).
- Words first (fast: plain text, ~100 MB/s), definitions after them at low
  priority (syntect is slower), so completion improves over the first
  seconds or minute on a large folder. A status-bar note while it runs.
- Kept up to date: a file saved in ochrepad is re-read at once; files
  changed by others (the terminal, git) are found by a cheap stat walk every
  30 seconds while ochrepad is idle and when the window gains focus.
- Cached in the data folder (`index/<sha256 of the folder>.json`, by path,
  size and modified time), so reopening a folder is quick; pruned like the
  undo histories.
- Memory: words are interned; a 5,000-file project should take tens of MB
  at most. The step measures it on ochrepad's own folder and on a large
  repository, and the limits move if needed.
- Definitions from the folder rank above plain words, with the file they
  are in as the detail; accepting one never opens anything.

### C4: context

- After `.`, `::`, `->` or `?.`: names the folder's text has after the same
  separator, then everything else. (A cheap stand-in for member completion;
  a server does it properly.)
- In a string that looks like a path (`"./`, `"../`, `"src/`, a drive or a
  `/`): the names in that folder, relative to the document's folder, then
  the open folder. Local paths only, never a network path (the
  `safe_images::is_local_path` rule).
- Preferences: completion on or off; after how many characters; sources
  (document, open documents, open folder); Enter accepts.

## 5. Language servers (steps L1–L3)

### L1: the client (`ochrepad-lsp`)

- One process per (server, root folder). The root is the open folder, else
  the nearest folder above the file with a marker (`Cargo.toml`,
  `pyproject.toml`, `setup.py`, `package.json`, `go.mod`, `CMakeLists.txt`,
  `compile_commands.json`, `dbt_project.yml`, `.git`).
- A reader thread and a writer thread per process; messages are framed
  (`Content-Length`), at most 64 MB each, and a malformed one ends that
  server's session (with a status-bar note and the log), never the app. The
  app polls a channel each frame; requests have ids, timeouts, and are
  cancelled (`$/cancelRequest`) when typing moves on.
- Document sync: `didOpen`, incremental `didChange` from each transaction's
  `ChangeSet` (positions converted in core), `didSave`, `didClose`.
  Documents over the highlight size limit are not sent.
- Server lifecycle: started on the first document of its languages in a
  trusted folder; `shutdown`/`exit` on closing the last such document or
  the folder, and on exit (killed after 2 seconds). A crash restarts it up
  to 3 times, then the status bar says it stopped. stderr goes to a log kept
  in memory (View > Language Servers shows each server, its state and log,
  with Restart and Stop).
- The trust question, once per folder, stored in the settings; "Don't ask
  again for this server" per server.
- The server table: rust-analyzer; basedpyright, pyright, pylsp,
  jedi-language-server and ty for Python (the first one found); gopls;
  clangd; typescript-language-server; marksman; and whatever `lsp.json`
  adds (dbt's, others).

### L2: what it shows

- Completion: the server's items in the popup, above the built-in ones, with
  `completionItem/resolve` for the selected item's documentation (a side
  panel). Snippet syntax (`$1`, `${1:name}`) is inserted as plain text with
  the caret at the first placeholder; tab stops are not in this plan.
- Diagnostics: underlines (errors red, warnings yellow, never the ochre
  accent, hints dotted), marks in the gutter and on the scrollbar overview, a count in
  the status bar, and a Problems panel (bottom, like Search Results) for the
  open folder; F8/Shift+F8 already go through changes, so next/previous
  problem gets its own keys (Alt+F8/Alt+Shift+F8, free today).
- Hover: after a pause over a name, a tooltip with the server's Markdown,
  rendered with the preview's renderer and its link rules (only http,
  https, mailto; files local only).
- Go to definition (F12), signature help (on `(` and `,`, and
  Ctrl+Shift+Space).

### L3: changing code

- Find references (Shift+F12) into the Search Results panel.
- Rename (Shift+F6): `prepareRename`, a field at the name, then the
  workspace edit applied with `Action::Edits` per document; documents that
  are not open open in tabs, unsaved.
- Format document (Alt+Shift+F) and format on save (a preference, off).
- Code actions (Ctrl+.): a menu at the caret; only edits are applied, and a
  server's `workspace/executeCommand` runs only on the server.
- The Function List uses the server's document symbols when there is one.
- Go to Symbol in Folder (a dialog searching `workspace/symbol`).

### What a server's messages may not do

- `window/showDocument` and file links: local files only, opened like the
  editor's Open File; never a network path or a non-`file:` URI.
- `workspace/applyEdit`: only while a user's rename or code action is in
  progress; at other times refused.
- Settings sent to servers come from `lsp.json` only.
- No `window/showMessageRequest` button runs anything; they are shown as
  notes.

## 6. The local model: a gate (step X, maybe never)

The aim would be JetBrains' kind of full-line completion: grey text after
the caret, Tab to take it. It is not started until everything else in this
plan is done, and it is **dropped**, without discussion, if any of these
fails. Dropping it is a fine outcome; nothing else in this plan depends on
it.

**Before writing any code**, a short written review for the user covering:

- **Dependencies**: every crate the inference needs (likely `candle-core`,
  `candle-transformers`, `tokenizers`, an HTTP client), with its maintainers,
  release history, the number of new transitive crates, C or C++ code
  built, `unsafe` use, and open advisories (`cargo audit`/`cargo deny`).
  Rejected if it pulls in a C/C++ build, a large new tree, anything
  unmaintained, or anything with a history of serious advisories.
- **No Python, no external process, no network service** at any point.
- **The model file**: GGUF or safetensors only (data, never code: no
  pickled PyTorch files); a license that allows it (e.g. Apache-2.0);
  downloaded only when the user asks, from a fixed URL, checked against a
  SHA-256 in the source, stored in the data folder. Rejected if no model
  meets this.
- **Size**: the release binary grows by no more than ~5 MB; the model stays
  outside it.

**Then a spike** on a branch that is not merged, measuring on this machine:
latency from a keystroke to a suggestion (target: median under 300 ms,
95th percentile under 800 ms, with a 512–1024 token context and the cache
reused between keystrokes), memory (under 1 GB), CPU use while typing, and
whether the suggestions are worth having on ochrepad's own code. Rejected if
it misses any target or the suggestions are not clearly useful.

Only after the user accepts both would it be built: behind a Cargo feature,
off by default in settings, with the model's download confirmed by the
user.

## 7. Steps

| Step | Content | Done when |
|---|---|---|
| C0 | `cargo deny` in CI (advisories, licenses, sources, duplicate versions as warnings), with a `deny.toml`. Not an app dependency. | CI runs it on Windows and Linux; the current tree passes or each exception is written down. |
| C1 | The popup, `Action::Complete` and `Action::Edits`, `core::complete` (matching, ranking), Ctrl+Space. | kittest: typing shows the popup, arrows and Tab accept, Esc closes, undo is one step, multi-caret; proptests for the Actions; the bench under budget. |
| C2 | Document and open-document words, keyword tables, the scope at the caret (no popup in comments). | Tests per source; keyword table test; no popup in a comment or string. |
| C3 | The folder index (words, then definitions), its cache and updates. | Index of ochrepad's own folder: time and memory written in the PR; a file changed in the terminal shows up within 30 s; tests for the cache and limits. |
| C4 | Member-separator context, paths in strings, preferences, `docs/completion.md`. | Tests for both contexts; docs. |
| L1 | `ochrepad-lsp`: framing, processes, requests, document sync, lifecycle, trust question, server table, `lsp.json`, View > Language Servers. | A fake server (a test binary speaking scripted JSON-RPC) drives tests on both platforms: start, sync (contents match after random edits), crash and restart, malformed messages, trust. |
| L2 | Completion from servers, diagnostics (underlines, gutter, Problems panel), hover, go to definition, signature help. | Fake-server tests for each; an ignored test against a real rust-analyzer when installed; screenshots reviewed. |
| L3 | References, rename, formatting, code actions, document symbols, folder symbols, `docs/language-servers.md`. | Fake-server tests; a rename across two files leaves the other one open and unsaved. |
| X | The local-model gate (section 6). | The review, then (only if accepted) the spike's numbers; the user decides. |

## 8. Risks

- **Ranking quality** decides whether built-in completion feels smart or
  noisy. C1–C4 are tested on ochrepad's own code by using it; weights are
  tuned, not guessed.
- **Keys while the popup is open** must not break the keymap, macros or the
  find panel; the popup takes only the keys listed in section 3.
- **Servers vary**: position encodings, incremental sync bugs, slow startup
  (rust-analyzer indexing for a minute). The status bar shows "indexing"
  from `$/progress`; requests made before a server is ready wait, then time
  out quietly.
- **Large folders**: the index's limits keep memory and time bounded; a
  folder over them gets document and open-document completion only, with a
  note.
- **Supply chain**: one new crate in this plan (`lsp-types`). C0's `cargo
  deny` watches it and everything else from now on.
