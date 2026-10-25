//! View > Function List, as in Notepad++: a panel on the right listing what
//! the active document defines (functions, types, modules, the headings of
//! a Markdown file), from its language server's document symbols when one
//! works on it (PLAN_completion L3), else from its syntax's scopes
//! (`Registry::symbols`) on a thread, once the text has been quiet for a
//! moment. The field filters by name, the entry the caret is in is marked,
//! and a click goes to it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};

use egui::{Align, Layout, RichText, ScrollArea, TextEdit, Ui};
use ochrepad_core::{BufferId, Document};
use ochrepad_syntax::{Registry, Symbol, SymbolKind};
use serde_json::{Value, json};

use crate::lsp::LanguageServers;

/// Quiet time after an edit before the list is read again.
const PAUSE_SECS: f64 = 0.3;

/// A symbol, with how deep it is nested (from its line's indentation, or a
/// heading's level).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub symbol: Symbol,
    pub depth: usize,
}

/// What the list was made from: buffer, revision, language.
type Key = (BufferId, u64, String);

struct Job {
    key: Key,
    cancel: Arc<AtomicBool>,
    rx: Receiver<Vec<Entry>>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[derive(Default)]
pub struct FunctionList {
    /// Shows only entries whose name contains this (ignoring case).
    pub filter: String,
    entries: Option<(Key, Arc<Vec<Entry>>)>,
    job: Option<Job>,
    /// The language server's symbols asked for: for what, tag and id.
    asked: Option<(Key, ochrepad_lsp::Tag, i64)>,
    /// The server had no list for this (the syntax's is read instead).
    server_failed: Option<Key>,
    /// The key last seen, and since when.
    seen: Option<(Key, f64)>,
}

/// What to show for the active tab.
pub struct Target<'a> {
    pub buffer: BufferId,
    pub revision: u64,
    pub language: &'a str,
    pub doc: &'a Document,
    pub indent_width: usize,
    pub caret_line: usize,
}

impl FunctionList {
    /// The entries for the document they were last read from.
    pub fn entries(&self, buffer: BufferId) -> Option<&[Entry]> {
        self.entries
            .as_ref()
            .filter(|(key, _)| key.0 == buffer)
            .map(|(_, e)| e.as_slice())
    }

    /// Per frame while the panel shows: reads the active document's symbols
    /// again when it changed and went quiet. Returns whether that is
    /// pending.
    pub fn tick(
        &mut self,
        target: &Target<'_>,
        registry: &Arc<Registry>,
        now: f64,
        servers: &mut LanguageServers,
    ) -> bool {
        let key: Key = (target.buffer, target.revision, target.language.to_owned());
        if self.seen.as_ref().is_none_or(|(k, _)| *k != key) {
            // Another document shows at once; an edit waits for quiet.
            let other = self.seen.as_ref().is_none_or(|(k, _)| k.0 != key.0);
            self.seen = Some((key.clone(), if other { now - PAUSE_SECS } else { now }));
        }
        if let Some(job) = &self.job {
            let done = match job.rx.try_recv() {
                Ok(entries) => Some(entries),
                // The thread failed: an empty list rather than "Reading…".
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Vec::new()),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            };
            if let Some(entries) = done {
                let job = self.job.take().expect("checked above");
                self.entries = Some((job.key.clone(), Arc::new(entries)));
            }
        }
        if let Some((k, tag, id)) = self.asked.take() {
            match servers.take_answer(tag) {
                Some(Ok(v)) => match server_entries(&v, target.language == "Markdown") {
                    Some(entries) => self.entries = Some((k, Arc::new(entries))),
                    None => self.server_failed = Some(k),
                },
                Some(Err(_)) => self.server_failed = Some(k),
                None if k != key => servers.cancel(k.0, id),
                None => self.asked = Some((k, tag, id)),
            }
        }
        if self.entries.as_ref().is_some_and(|(k, _)| *k == key) {
            return false;
        }
        if self.job.as_ref().is_some_and(|j| j.key != key) {
            self.job = None;
        }
        if self.asked.is_some() {
            return true;
        }
        let quiet = self
            .seen
            .as_ref()
            .is_some_and(|(_, t)| now - t >= PAUSE_SECS);
        // The language server's list, when it has one.
        if self.job.is_none() && quiet && self.server_failed.as_ref() != Some(&key) {
            let doc = target.doc;
            let sent = servers.request(
                target.buffer,
                doc,
                "documentSymbolProvider",
                "textDocument/documentSymbol",
                |_, uri| json!({ "textDocument": { "uri": uri } }),
                std::time::Duration::from_secs(10),
            );
            if let Some((tag, id)) = sent {
                self.asked = Some((key, tag, id));
                return true;
            }
        }
        if self.job.is_none() && quiet {
            let (tx, rx) = channel();
            let cancel = Arc::new(AtomicBool::new(false));
            let stop = cancel.clone();
            let registry = registry.clone();
            let doc = target.doc.clone();
            let language = target.language.to_owned();
            let width = target.indent_width.max(1);
            let spawned = std::thread::Builder::new()
                .name("ochrepad-symbols".into())
                .spawn(move || {
                    let found = registry.symbols(&language, &doc, &|| stop.load(Ordering::Relaxed));
                    if let Some(symbols) = found {
                        let entries = symbols
                            .into_iter()
                            .map(|symbol| {
                                let depth = depth(&doc, &symbol, width);
                                Entry { symbol, depth }
                            })
                            .collect();
                        let _ = tx.send(entries);
                    }
                });
            if spawned.is_ok() {
                self.job = Some(Job { key, cancel, rx });
            }
        }
        true
    }

    /// The panel's contents. Returns the line of an entry clicked, and
    /// whether the panel asked to close.
    pub fn ui(&mut self, ui: &mut Ui, target: Option<&Target<'_>>) -> (Option<usize>, bool) {
        let mut close = false;
        let mut go = None;
        ui.horizontal(|ui| {
            ui.strong("Function List");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .small_button("×")
                    .on_hover_text("Close the Function List")
                    .clicked()
                {
                    close = true;
                }
            });
        });
        ui.add(
            TextEdit::singleline(&mut self.filter)
                .id(egui::Id::new("ochrepad-function-filter"))
                .hint_text("Filter")
                .desired_width(f32::INFINITY),
        );
        ui.add_space(crate::theme::space::XS);
        let Some(target) = target else {
            ui.weak("Nothing to list for this tab.");
            return (None, close);
        };
        let Some(entries) = self.entries(target.buffer) else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak("Reading…");
            });
            return (None, close);
        };
        if entries.is_empty() {
            ui.weak(format!("Nothing found in this {} file.", target.language));
            return (None, close);
        }
        let filter = self.filter.to_lowercase();
        let shown: Vec<&Entry> = entries
            .iter()
            .filter(|e| filter.is_empty() || e.symbol.name.to_lowercase().contains(&filter))
            .collect();
        // The entry the caret is in: the last one starting at or above it.
        let current = entries
            .iter()
            .rposition(|e| e.symbol.line <= target.caret_line)
            .map(|i| &entries[i]);
        let weak = ui.visuals().weak_text_color();
        let row_h = ui.spacing().interact_size.y;
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, row_h, shown.len(), |ui, rows| {
                for e in &shown[rows] {
                    ui.horizontal(|ui| {
                        ui.add_space(e.depth.min(8) as f32 * 12.0);
                        let (glyph, hint) = match e.symbol.kind {
                            SymbolKind::Function => ("ƒ", "Function"),
                            SymbolKind::Type => ("T", "Type"),
                            SymbolKind::Module => ("M", "Module"),
                            SymbolKind::Heading => ("#", "Heading"),
                        };
                        ui.label(RichText::new(glyph).monospace().color(weak))
                            .on_hover_text(hint);
                        let selected = current.is_some_and(|c| std::ptr::eq(c, *e));
                        let label = ui
                            .selectable_label(selected, &e.symbol.name)
                            .on_hover_text(format!("Line {}", e.symbol.line + 1));
                        if label.clicked() {
                            go = Some(e.symbol.line);
                        }
                    });
                }
            });
        (go, close)
    }
}

/// How deep `symbol` is nested: a heading's level, else its line's
/// indentation in indent levels.
/// A `documentSymbol` answer as entries: nested `DocumentSymbol`s (their
/// depth from the nesting) or flat `SymbolInformation`s. Functions, types
/// and modules, as the syntax's list has; a Markdown document's symbols
/// are its headings. `None` when it is not a list.
fn server_entries(v: &Value, markdown: bool) -> Option<Vec<Entry>> {
    fn walk(items: &[Value], depth: usize, markdown: bool, out: &mut Vec<Entry>) {
        for it in items {
            let line = it
                .pointer("/selectionRange/start/line")
                .or_else(|| it.pointer("/location/range/start/line"))
                .or_else(|| it.pointer("/range/start/line"))
                .and_then(Value::as_u64);
            let name = it.get("name").and_then(Value::as_str);
            let kind = if markdown {
                Some(SymbolKind::Heading)
            } else {
                match it.get("kind").and_then(Value::as_u64) {
                    Some(2..=4) => Some(SymbolKind::Module),
                    Some(5 | 10 | 11 | 19 | 23 | 26) => Some(SymbolKind::Type),
                    Some(6 | 9 | 12) => Some(SymbolKind::Function),
                    _ => None,
                }
            };
            let mut inner = depth;
            if let (Some(line), Some(name), Some(kind)) = (line, name, kind) {
                out.push(Entry {
                    symbol: Symbol {
                        line: usize::try_from(line).unwrap_or(usize::MAX),
                        name: name.lines().next().unwrap_or(name).to_owned(),
                        kind,
                    },
                    depth,
                });
                inner += 1;
            }
            if let Some(children) = it.get("children").and_then(Value::as_array) {
                walk(children, inner, markdown, out);
            }
        }
    }
    let items = v.as_array()?;
    let mut out = Vec::new();
    walk(items, 0, markdown, &mut out);
    Some(out)
}

fn depth(doc: &Document, symbol: &Symbol, indent_width: usize) -> usize {
    let text = doc.line_text(symbol.line);
    if symbol.kind == SymbolKind::Heading {
        let hashes = text.chars().take_while(|&c| c == '#').count();
        return hashes.saturating_sub(1);
    }
    let mut cols = 0;
    for c in text.chars() {
        match c {
            ' ' => cols += 1,
            '\t' => cols += indent_width - cols % indent_width,
            _ => break,
        }
    }
    cols / indent_width
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_symbols_nest_and_keep_definitions() {
        let at = |line: u64| json!({"start": {"line": line, "character": 0}, "end": {"line": line, "character": 1}});
        let v = json!([
            {"name": "Point", "kind": 23, "range": at(0), "selectionRange": at(0), "children": [
                {"name": "x", "kind": 8, "range": at(1), "selectionRange": at(1)},
            ]},
            {"name": "impl Point", "kind": 19, "range": at(3), "selectionRange": at(3), "children": [
                {"name": "new", "kind": 6, "range": at(4), "selectionRange": at(4)},
            ]},
            {"name": "LIMIT", "kind": 14, "range": at(7), "selectionRange": at(7)},
        ]);
        let entries = server_entries(&v, false).unwrap();
        let got: Vec<(&str, usize, usize)> = entries
            .iter()
            .map(|e| (e.symbol.name.as_str(), e.symbol.line, e.depth))
            .collect();
        assert_eq!(got, [("Point", 0, 0), ("impl Point", 3, 0), ("new", 4, 1)]);
        // Flat symbols, and a Markdown document's headings.
        let flat = json!([{"name": "Intro", "kind": 15, "location": {"uri": "file:///a.md", "range": at(2)}}]);
        let headings = server_entries(&flat, true).unwrap();
        assert_eq!(headings[0].symbol.kind, SymbolKind::Heading);
        assert_eq!(headings[0].symbol.line, 2);
        assert!(server_entries(&Value::Null, false).is_none());
    }
}
