//! Search > Go to Symbol in Folder (Ctrl+Alt+T; PLAN_completion L3): a
//! dialog searching the language server's symbols for the project
//! (`workspace/symbol`) as you type, and opening the one chosen. Only local
//! files open.

use std::path::PathBuf;
use std::time::Duration;

use egui::{Id, Key, Modal, Modifiers, RichText, ScrollArea, Ui};
use ochrepad_core::position::{ColumnUnits, from_lsp_position};
use ochrepad_core::{Action, BufferId, SelectionSet};
use ochrepad_lsp::Tag;
use serde_json::{Value, json};

use super::OchrepadApp;

/// Quiet time after typing before the server is asked.
const PAUSE_SECS: f64 = 0.25;
/// Results shown.
const MAX_SHOWN: usize = 200;

/// The dialog.
pub(super) struct SymbolSearch {
    /// The document whose server is asked.
    buffer: BufferId,
    units: ColumnUnits,
    query: String,
    /// When the query last changed.
    changed: f64,
    /// The query asked about, and its tag.
    asked: Option<(String, Tag)>,
    /// The query answered, and what was found.
    answered: Option<String>,
    found: Vec<Found>,
    selected: usize,
    /// The server's error, if it failed.
    error: Option<String>,
}

struct Found {
    name: String,
    /// A letter for its kind.
    kind: &'static str,
    container: Option<String>,
    path: PathBuf,
    line: u32,
    character: u32,
}

impl OchrepadApp {
    /// Ctrl+Alt+T: opens the dialog.
    pub(super) fn go_to_symbol(&mut self) {
        let buffer = self.buffer_id();
        let Some(units) = self
            .language_servers
            .session_of(buffer)
            .filter(|s| s.client.can("workspaceSymbolProvider"))
            .map(|s| s.client.units)
        else {
            self.note_text(
                "Go to Symbol needs a language server that searches the folder; \
                 open a file one works on (View > Language Servers)",
            );
            return;
        };
        self.symbol_search = Some(SymbolSearch {
            buffer,
            units,
            query: String::new(),
            changed: self.now,
            asked: None,
            answered: None,
            found: Vec::new(),
            selected: 0,
            error: None,
        });
    }

    /// The dialog, once a frame while it is open.
    pub(super) fn symbol_search_dialog(&mut self, ui: &mut Ui) {
        let now = self.now;
        let root = self.tree.as_ref().map(|t| t.root().to_path_buf());
        let Some(s) = &mut self.symbol_search else {
            return;
        };
        // The answer.
        if let Some((query, tag)) = &s.asked
            && let Some(answer) = self.language_servers.take_answer(*tag)
        {
            s.answered = Some(query.clone());
            s.asked = None;
            match answer {
                Ok(v) => {
                    s.found = found(&v);
                    s.error = None;
                }
                Err(e) => {
                    s.found.clear();
                    s.error = Some(e.message);
                }
            }
            s.selected = 0;
        }
        // Ask once typing pauses.
        let stale = s.answered.as_deref() != Some(s.query.as_str())
            && s.asked.as_ref().is_none_or(|(q, _)| *q != s.query);
        if stale && !s.query.trim().is_empty() {
            if now - s.changed >= PAUSE_SECS {
                let doc = self.editor.buffer(s.buffer).map(|b| b.doc().clone());
                let query = s.query.clone();
                if let Some(doc) = doc
                    && let Some((tag, _)) = self.language_servers.request(
                        s.buffer,
                        &doc,
                        "workspaceSymbolProvider",
                        "workspace/symbol",
                        |_, _| json!({ "query": query }),
                        Duration::from_secs(10),
                    )
                {
                    s.asked = Some((query, tag));
                }
            } else {
                ui.ctx()
                    .request_repaint_after(Duration::from_secs_f64(PAUSE_SECS));
            }
        }
        let (down, up) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
            )
        });
        let n = s.found.len().min(MAX_SHOWN);
        if n > 0 && down {
            s.selected = (s.selected + 1) % n;
        }
        if n > 0 && up {
            s.selected = (s.selected + n - 1) % n;
        }
        let mut chosen = None;
        let resp = Modal::new(Id::new("ochrepad-go-to-symbol")).show(ui.ctx(), |ui| {
            ui.set_width(560.0);
            ui.label("Go to Symbol in Folder");
            let field = ui.add(
                egui::TextEdit::singleline(&mut s.query)
                    .id(Id::new("ochrepad-go-to-symbol-field"))
                    .hint_text("A name, or part of one")
                    .desired_width(f32::INFINITY),
            );
            if field.changed() {
                s.changed = now;
            }
            if !field.has_focus() && !field.lost_focus() {
                field.request_focus();
            }
            if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                chosen = Some(s.selected);
            }
            ui.add_space(4.0);
            if let Some(e) = &s.error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            } else if s.asked.is_some() && s.found.is_empty() {
                ui.weak("Searching…");
            } else if s.answered.is_some() && s.found.is_empty() {
                ui.weak("Nothing found");
            }
            ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                for (i, f) in s.found.iter().take(MAX_SHOWN).enumerate() {
                    let file = root
                        .as_deref()
                        .and_then(|r| f.path.strip_prefix(r).ok())
                        .unwrap_or(&f.path)
                        .display()
                        .to_string();
                    let mut text = format!("{}  {}", f.kind, f.name);
                    if let Some(c) = &f.container {
                        text.push_str(&format!("  ({c})"));
                    }
                    let r = ui
                        .selectable_label(i == s.selected, RichText::new(text).monospace())
                        .on_hover_text(format!("{file}:{}", f.line + 1));
                    ui.weak(format!("    {file}:{}", f.line + 1));
                    if i == s.selected && (down || up) {
                        r.scroll_to_me(None);
                    }
                    if r.clicked() {
                        chosen = Some(i);
                    }
                }
            });
        });
        if resp.should_close() {
            self.symbol_search = None;
            self.view_state_mut().request_focus();
            return;
        }
        let Some(i) = chosen else {
            return;
        };
        let Some(s) = self.symbol_search.take() else {
            return;
        };
        let Some(f) = s.found.get(i) else {
            return;
        };
        if !crate::safe_images::is_local_path(&f.path) || !f.path.is_file() {
            self.note_text(format!("{} is not a local file", f.path.display()));
            return;
        }
        self.open_file_at(&f.path, None);
        let opened = self.buffer().meta.path.as_deref().map(super::canonical);
        if opened == Some(super::canonical(&f.path)) {
            let at = from_lsp_position(self.buffer().doc(), f.line, f.character, s.units);
            self.apply(Action::SetSelections(SelectionSet::caret(at)));
        }
    }
}

/// A `workspace/symbol` answer: `SymbolInformation`s or `WorkspaceSymbol`s
/// (with a location's range), in local files.
fn found(v: &Value) -> Vec<Found> {
    let Some(items) = v.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|it| {
            let name = it.get("name")?.as_str()?;
            let location = it.get("location")?;
            let uri = location.get("uri")?.as_str()?;
            let path = ochrepad_lsp::uri::file_path(&ochrepad_lsp::uri::normalize(uri))?;
            let start = location.pointer("/range/start");
            let n = |k: &str| {
                start
                    .and_then(|s| s.get(k))
                    .and_then(Value::as_u64)
                    .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX))
            };
            Some(Found {
                name: name.lines().next().unwrap_or(name).to_owned(),
                kind: kind_letter(it.get("kind").and_then(Value::as_u64)),
                container: it
                    .get("containerName")
                    .and_then(Value::as_str)
                    .filter(|c| !c.is_empty())
                    .map(str::to_owned),
                path,
                line: n("line"),
                character: n("character"),
            })
        })
        .collect()
}

/// The letter the Function List and completion use for a symbol kind.
fn kind_letter(kind: Option<u64>) -> &'static str {
    match kind {
        Some(2..=4) => "M",
        Some(5 | 10 | 11 | 19 | 23 | 26) => "T",
        Some(6 | 9 | 12) => "ƒ",
        Some(14 | 22) => "c",
        Some(7 | 8 | 13) => "v",
        _ => "·",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_from_json() {
        let uri = if cfg!(windows) {
            "file:///C:/p/a.rs"
        } else {
            "file:///p/a.rs"
        };
        let v = json!([
            {"name": "Point", "kind": 23, "containerName": "geometry",
             "location": {"uri": uri, "range": {"start": {"line": 4, "character": 7}, "end": {"line": 4, "character": 12}}}},
            {"name": "remote", "kind": 12, "location": {"uri": "file://server/share/b.rs"}},
            {"name": "no_range", "kind": 12, "location": {"uri": uri}},
        ]);
        let f = found(&v);
        assert_eq!(f.len(), 2, "not the network file");
        assert_eq!((f[0].kind, f[0].line, f[0].character), ("T", 4, 7));
        assert_eq!(f[0].container.as_deref(), Some("geometry"));
        assert_eq!((f[1].line, f[1].character), (0, 0));
    }
}
