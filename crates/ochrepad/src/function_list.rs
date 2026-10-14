//! View > Function List, as in Notepad++: a panel on the right listing what
//! the active document defines (functions, types, modules, the headings of
//! a Markdown file), from its syntax's scopes (`Registry::symbols`), found
//! on a thread once the text has been quiet for a moment. The field filters
//! by name, the entry the caret is in is marked, and a click goes to it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};

use egui::{Align, Layout, RichText, ScrollArea, TextEdit, Ui};
use ochrepad_core::{BufferId, Document};
use ochrepad_syntax::{Registry, Symbol, SymbolKind};

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
    pub fn tick(&mut self, target: &Target<'_>, registry: &Arc<Registry>, now: f64) -> bool {
        let key: Key = (target.buffer, target.revision, target.language.to_owned());
        if self.seen.as_ref().is_none_or(|(k, _)| *k != key) {
            // Another document shows at once; an edit waits for quiet.
            let other = self.seen.as_ref().is_none_or(|(k, _)| k.0 != key.0);
            self.seen = Some((key.clone(), if other { now - PAUSE_SECS } else { now }));
        }
        if let Some(job) = &self.job
            && let Ok(entries) = job.rx.try_recv()
        {
            let job = self.job.take().expect("checked above");
            self.entries = Some((job.key.clone(), Arc::new(entries)));
        }
        if self.entries.as_ref().is_some_and(|(k, _)| *k == key) {
            return false;
        }
        if self.job.as_ref().is_some_and(|j| j.key != key) {
            self.job = None;
        }
        let quiet = self
            .seen
            .as_ref()
            .is_some_and(|(_, t)| now - t >= PAUSE_SECS);
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
