//! Go to Definition (F12; PLAN_completion L2) and Find References
//! (Shift+F12; L3): asks the document's language server where the name at
//! the caret is defined, and opens that file there, or where it is used,
//! listed in the Search Results panel. Only local files open or are read,
//! as with the editor menu's Open File.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use ochrepad_core::position::{ColumnUnits, from_lsp_position};
use ochrepad_core::{
    Action, BufferId, ByteOffset, Document, SearchOptions, SearchQuery, SelectionSet,
};
use ochrepad_lsp::Tag;
use serde_json::{Value, json};

use super::OchrepadApp;
use crate::search_results::{FileHits, Source};

/// A request in flight.
pub(super) struct Pending {
    kind: Kind,
    buffer: BufferId,
    tag: Tag,
    id: i64,
    /// The server's column units, for the positions it answers with.
    units: ColumnUnits,
    /// The name at the caret (references are listed under it).
    name: String,
    server: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Definition,
    References,
}

impl OchrepadApp {
    /// F12: asks where the name at the caret is defined.
    pub(super) fn go_to_definition(&mut self) {
        self.ask_where(Kind::Definition);
    }

    /// Shift+F12: asks where the name at the caret is used.
    pub(super) fn find_references(&mut self) {
        self.ask_where(Kind::References);
    }

    fn ask_where(&mut self, kind: Kind) {
        let buffer = self.buffer_id();
        let (Some(doc), Some(caret)) = (
            self.editor.buffer(buffer).map(|b| b.doc().clone()),
            self.editor
                .view(self.active)
                .map(|v| v.selections.primary().head),
        ) else {
            return;
        };
        let Some((units, server)) = self
            .language_servers
            .session_of(buffer)
            .map(|s| (s.client.units, s.name.clone()))
        else {
            self.note_text("No language server works on this document (View > Language Servers)");
            return;
        };
        if let Some(p) = self.definition.take() {
            self.language_servers.cancel(p.buffer, p.id);
            let _ = self.language_servers.take_answer(p.tag);
        }
        let (capability, method, what) = match kind {
            Kind::Definition => (
                "definitionProvider",
                "textDocument/definition",
                "go to definitions",
            ),
            Kind::References => (
                "referencesProvider",
                "textDocument/references",
                "find references",
            ),
        };
        let sent = self.language_servers.request(
            buffer,
            &doc,
            capability,
            method,
            |c, uri| {
                let mut params = json!({
                    "textDocument": { "uri": uri },
                    "position": c.position(&doc, caret),
                });
                if kind == Kind::References {
                    params["context"] = json!({ "includeDeclaration": true });
                }
                params
            },
            // References in a large project take a while.
            Duration::from_secs(if kind == Kind::References { 60 } else { 10 }),
        );
        let word = ochrepad_core::complete::word_start(&doc, caret, &[]);
        let line_end = doc.line_end(doc.line_of(caret));
        let after: usize = doc
            .slice(caret..line_end)
            .chars()
            .take_while(|&c| ochrepad_core::complete::is_word_char(c, &[]))
            .map(char::len_utf8)
            .sum();
        let end = ByteOffset(caret.0 + after);
        match sent {
            Some((tag, id)) => {
                self.definition = Some(Pending {
                    kind,
                    buffer,
                    tag,
                    id,
                    units,
                    name: doc.slice(word..end),
                    server,
                })
            }
            None => self.note_text(format!("The language server cannot {what} (yet)")),
        }
    }

    /// Once a frame: opens the definition when the answer comes.
    pub(super) fn definition_tick(&mut self) {
        let Some(p) = &self.definition else {
            return;
        };
        let Some(answer) = self.language_servers.take_answer(p.tag) else {
            return;
        };
        let units = p.units;
        let Some(p) = self.definition.take() else {
            return;
        };
        if p.kind == Kind::References {
            match answer {
                Ok(v) => self.list_references(&p, &v),
                Err(e) => self.note_text(format!("Find References: {}", e.message)),
            }
            return;
        }
        let found = match answer {
            Ok(v) => locations(&v),
            Err(e) => {
                self.note_text(format!("Go to Definition: {}", e.message));
                return;
            }
        };
        let Some((path, line, character)) = found.first().cloned() else {
            self.note_text("No definition found");
            return;
        };
        // Only a local file (a link to a network share is not).
        if !crate::safe_images::is_local_path(&path) || !path.is_file() {
            self.note_text(format!(
                "The definition is in {}, not a local file",
                path.display()
            ));
            return;
        }
        self.open_file_at(&path, None);
        let opened = self.buffer().meta.path.as_deref().map(super::canonical);
        if opened == Some(super::canonical(&path)) {
            let at = from_lsp_position(self.buffer().doc(), line, character, units);
            self.apply(Action::SetSelections(SelectionSet::caret(at)));
        }
        if found.len() > 1 {
            self.note_text(format!("{} definitions; this is the first", found.len()));
        }
    }
}

impl OchrepadApp {
    /// A references answer, in the Search Results panel: open documents
    /// as they are, other files read on the search's thread.
    fn list_references(&mut self, p: &Pending, v: &Value) {
        let mut found = ranges(v);
        if found.is_empty() {
            self.note_text(format!("No references to {} found", p.name));
            return;
        }
        found.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
        let open: Vec<(PathBuf, BufferId, Document, u64)> = self
            .editor
            .buffers()
            .filter_map(|b| {
                let path = ochrepad_lsp::uri::plain_path(b.meta.path.as_deref()?);
                Some((path, b.id(), b.doc().clone(), b.revision()))
            })
            .collect();
        let units = p.units;
        let root = self.tree.as_ref().map(|t| t.root().to_path_buf());
        let query = SearchQuery::new(
            p.name.clone(),
            SearchOptions {
                match_case: true,
                whole_word: true,
                ..SearchOptions::default()
            },
        );
        let scope = format!("references ({})", p.server);
        let cancel = Arc::new(AtomicBool::new(false));
        self.results.start_external(query, scope, cancel, move |_| {
            Ok((reference_hits(found, &open, root.as_deref(), units), false))
        });
    }
}

/// On the search's thread: the references' lines, file by file (`found`
/// sorted by file).
fn reference_hits(
    found: Vec<(PathBuf, [(u32, u32); 2])>,
    open: &[(PathBuf, BufferId, Document, u64)],
    root: Option<&std::path::Path>,
    units: ColumnUnits,
) -> Vec<FileHits> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < found.len() {
        let path = found[i].0.clone();
        let n = found[i..].iter().take_while(|f| f.0 == path).count();
        let places = &found[i..i + n];
        i += n;
        let loaded;
        let (doc, source, revision) = match open.iter().find(|o| o.0 == path) {
            Some((_, id, doc, revision)) => (doc, Source::Buffer(*id), *revision),
            None => {
                // Only local files, and not huge ones.
                let readable = crate::safe_images::is_local_path(&path)
                    && std::fs::metadata(&path).is_ok_and(|m| {
                        m.is_file() && m.len() <= crate::search_results::MAX_FILE_BYTES
                    });
                let Some(text) = readable
                    .then(|| ochrepad_fileio::load(&path).ok())
                    .flatten()
                else {
                    continue;
                };
                loaded = Document::new(&text.text);
                (&loaded, Source::Path(path.clone()), 0)
            }
        };
        let mut ranges: Vec<std::ops::Range<ByteOffset>> = places
            .iter()
            .map(|(_, r)| {
                let a = from_lsp_position(doc, r[0].0, r[0].1, units);
                let b = from_lsp_position(doc, r[1].0, r[1].1, units);
                a.min(b)..a.max(b)
            })
            .collect();
        ranges.sort_by_key(|r| (r.start, r.end));
        ranges.dedup();
        let name = root
            .and_then(|r| path.strip_prefix(r).ok())
            .unwrap_or(&path)
            .display()
            .to_string();
        out.push(FileHits {
            hits: crate::search_results::hits(doc, &ranges),
            matches: ranges.len(),
            source,
            name,
            revision,
            expanded: true,
        });
    }
    out
}

/// A references answer's places: each file and range.
fn ranges(v: &Value) -> Vec<(PathBuf, [(u32, u32); 2])> {
    let Some(items) = v.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|l| {
            let path = ochrepad_lsp::uri::file_path(&ochrepad_lsp::uri::normalize(
                l.get("uri")?.as_str()?,
            ))?;
            let range = l.get("range")?;
            Some((
                path,
                [position(range.get("start")?)?, position(range.get("end")?)?],
            ))
        })
        .collect()
}

fn position(p: &Value) -> Option<(u32, u32)> {
    let n = |k: &str| {
        p.get(k)?
            .as_u64()
            .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
    };
    Some((n("line")?, n("character")?))
}

/// A definition answer's places: a location, a list of them, or a list of
/// links (their target's name).
fn locations(v: &Value) -> Vec<(PathBuf, u32, u32)> {
    let one = |l: &Value| {
        let (uri, range) = match (l.get("targetUri"), l.get("uri")) {
            (Some(u), _) => (
                u,
                l.get("targetSelectionRange")
                    .or_else(|| l.get("targetRange"))?,
            ),
            (None, Some(u)) => (u, l.get("range")?),
            (None, None) => return None,
        };
        // Our spelling (rust-analyzer writes `file:///c:/`).
        let path = ochrepad_lsp::uri::file_path(&ochrepad_lsp::uri::normalize(uri.as_str()?))?;
        let start = range.get("start")?;
        let n = |k: &str| {
            start
                .get(k)?
                .as_u64()
                .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
        };
        Some((path, n("line")?, n("character")?))
    };
    match v {
        Value::Array(items) => items.iter().filter_map(one).collect(),
        Value::Object(_) => one(v).into_iter().collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definition_answers() {
        let uri = if cfg!(windows) {
            "file:///C:/p/a.rs"
        } else {
            "file:///p/a.rs"
        };
        let range =
            json!({"start": {"line": 3, "character": 4}, "end": {"line": 3, "character": 8}});
        let one = locations(&json!({"uri": uri, "range": range}));
        assert_eq!(one.len(), 1);
        assert_eq!((one[0].1, one[0].2), (3, 4));
        let links = locations(&json!([{
            "targetUri": uri, "targetRange": range,
            "targetSelectionRange": {"start": {"line": 3, "character": 5}, "end": {"line": 3, "character": 6}},
        }]));
        assert_eq!((links[0].1, links[0].2), (3, 5));
        // Not a local file: left out.
        assert!(locations(&json!({"uri": "file://server/share/a.rs", "range": range})).is_empty());
        assert!(locations(&Value::Null).is_empty());
    }
}
