//! Language servers changing code (PLAN_completion L3): Rename (Shift+F6),
//! Format Document (Alt+Shift+F, and on save when chosen), and applying a
//! server's workspace edit: every edit is checked against its document
//! before any is applied, each document's edits are one undo step, and
//! files that are not open open in tabs, edited and unsaved.

use std::path::PathBuf;
use std::time::Duration;

use egui::{Id, Key, Modal, Ui};
use ochrepad_core::position::{ColumnUnits, from_lsp_position};
use ochrepad_core::{Action, BufferId, ByteOffset, Document, TextEdit};
use ochrepad_lsp::Tag;
use ochrepad_lsp::uri::{file_path, normalize, plain_path};
use serde_json::{Value, json};

use super::OchrepadApp;

/// Where Rename is.
pub(super) enum Rename {
    /// Asking the server whether the name at `caret` can be renamed.
    Preparing {
        buffer: BufferId,
        caret: ByteOffset,
        tag: Tag,
    },
    /// Asking the user for the new name.
    Asking {
        buffer: BufferId,
        caret: ByteOffset,
        old: String,
        new: String,
    },
    /// Waiting for the server's edits.
    Renaming { tag: Tag, units: ColumnUnits },
}

/// A formatting request in flight.
pub(super) struct Formatting {
    buffer: BufferId,
    tag: Tag,
    id: i64,
    units: ColumnUnits,
    /// Save once formatted (format on save).
    then_save: bool,
}

/// A server's edits to one file.
struct FileEdits {
    path: PathBuf,
    /// The document version they were made for, if the server said.
    version: Option<i64>,
    edits: Vec<Value>,
}

impl OchrepadApp {
    /// Shift+F6: rename the name at the caret everywhere.
    pub(super) fn rename_symbol(&mut self) {
        let buffer = self.buffer_id();
        let Some((doc, caret)) = self.doc_and_caret() else {
            return;
        };
        let Some(prepare) = self.language_servers.session_of(buffer).map(|s| {
            s.client
                .capabilities
                .pointer("/renameProvider/prepareProvider")
                .and_then(Value::as_bool)
                == Some(true)
        }) else {
            self.note_text("No language server works on this document (View > Language Servers)");
            return;
        };
        if prepare {
            let sent = self.language_servers.request(
                buffer,
                &doc,
                "renameProvider",
                "textDocument/prepareRename",
                |c, uri| json!({ "textDocument": { "uri": uri }, "position": c.position(&doc, caret) }),
                Duration::from_secs(10),
            );
            if let Some((tag, _)) = sent {
                self.renaming = Some(Rename::Preparing { buffer, caret, tag });
                return;
            }
        }
        if !self
            .language_servers
            .session_of(buffer)
            .is_some_and(|s| s.client.can("renameProvider"))
        {
            self.note_text("The language server cannot rename (yet)");
            return;
        }
        let old = word_at(&doc, caret);
        self.renaming = Some(Rename::Asking {
            buffer,
            caret,
            new: old.clone(),
            old,
        });
    }

    /// Alt+Shift+F: format the document with its language server.
    pub(super) fn format_document(&mut self) {
        let buffer = self.buffer_id();
        if !self.request_formatting(buffer, false) {
            self.note_text("No language server formats this document (View > Language Servers)");
        }
    }

    /// Asks for `buffer`'s formatting; whether it was asked.
    fn request_formatting(&mut self, buffer: BufferId, then_save: bool) -> bool {
        let Some(b) = self.editor.buffer(buffer) else {
            return false;
        };
        let doc = b.doc().clone();
        let indent = b.indent;
        let Some(units) = self
            .language_servers
            .session_of(buffer)
            .map(|s| s.client.units)
        else {
            return false;
        };
        let sent = self.language_servers.request(
            buffer,
            &doc,
            "documentFormattingProvider",
            "textDocument/formatting",
            |_, uri| {
                json!({
                    "textDocument": { "uri": uri },
                    "options": { "tabSize": indent.width, "insertSpaces": indent.use_spaces },
                })
            },
            // Saving waits for it.
            Duration::from_secs(if then_save { 2 } else { 10 }),
        );
        let Some((tag, id)) = sent else {
            return false;
        };
        if let Some(f) = self.formatting.take() {
            self.language_servers.cancel(f.buffer, f.id);
            let _ = self.language_servers.take_answer(f.tag);
        }
        self.formatting = Some(Formatting {
            buffer,
            tag,
            id,
            units,
            then_save,
        });
        true
    }

    /// Ctrl+S with format on save: formats first when a server can, and
    /// saves when it answers. Whether it took the save.
    pub(super) fn format_then_save(&mut self, buffer: BufferId) -> bool {
        self.settings.format_on_save
            && self
                .editor
                .buffer(buffer)
                .is_some_and(|b| b.meta.path.is_some() && !b.read_only)
            && self.request_formatting(buffer, true)
    }

    /// Once a frame: Rename's and formatting's answers.
    pub(super) fn edits_tick(&mut self) {
        self.formatting_tick();
        let answer = match &self.renaming {
            Some(Rename::Preparing { tag, .. } | Rename::Renaming { tag, .. }) => {
                match self.language_servers.take_answer(*tag) {
                    Some(a) => a,
                    None => return,
                }
            }
            _ => return,
        };
        match self.renaming.take() {
            Some(Rename::Preparing { buffer, caret, .. }) => {
                let Some(doc) = self.editor.buffer(buffer).map(|b| b.doc().clone()) else {
                    return;
                };
                let old = match answer {
                    Ok(Value::Null) | Err(_) => {
                        self.note_text("This cannot be renamed");
                        return;
                    }
                    Ok(v) => match v.get("placeholder").and_then(Value::as_str) {
                        Some(p) => p.to_owned(),
                        None => v
                            .get("range")
                            .or(Some(&v).filter(|v| v.get("start").is_some()))
                            .and_then(|r| self.language_servers.range_in(buffer, &doc, r))
                            .map(|r| doc.slice(r))
                            .unwrap_or_else(|| word_at(&doc, caret)),
                    },
                };
                self.renaming = Some(Rename::Asking {
                    buffer,
                    caret,
                    new: old.clone(),
                    old,
                });
            }
            Some(Rename::Renaming { units, .. }) => match answer {
                Ok(Value::Null) => self.note_text("The language server made no changes"),
                Ok(edit) => match self.apply_workspace_edit(&edit, units) {
                    Ok(0) => self.note_text("The language server made no changes"),
                    Ok(1) => self.note_text("Renamed"),
                    Ok(n) => self.note_text(format!("Renamed in {n} files (not saved yet)")),
                    Err(e) => self.message = Some(format!("Rename was not applied:\n{e}")),
                },
                Err(e) => self.message = Some(format!("Rename failed:\n{}", e.message)),
            },
            _ => {}
        }
    }

    fn formatting_tick(&mut self) {
        let Some(f) = &self.formatting else {
            return;
        };
        let Some(answer) = self.language_servers.take_answer(f.tag) else {
            return;
        };
        let Some(f) = self.formatting.take() else {
            return;
        };
        match answer {
            Ok(Value::Array(edits)) => {
                let doc = self.editor.buffer(f.buffer).map(|b| b.doc().clone());
                // Only onto the text the server formatted.
                match doc.filter(|d| self.language_servers.in_sync(f.buffer, d)) {
                    Some(doc) => {
                        if let Err(e) = self.apply_edits_to(f.buffer, &doc, &edits, f.units) {
                            self.note_text(format!("Formatting was not applied: {e}"));
                        }
                    }
                    None => self.note_text("The document changed while it was formatted"),
                }
            }
            Ok(_) => {}
            Err(e) if f.then_save => {
                self.note_text(format!("Saved without formatting: {}", e.message));
            }
            Err(e) => self.note_text(format!("Format Document: {}", e.message)),
        }
        if f.then_save {
            self.save_buffer(f.buffer);
        }
    }

    /// The Rename dialog.
    pub(super) fn rename_symbol_dialog(&mut self, ui: &mut Ui) {
        let Some(Rename::Asking { old, new, .. }) = &mut self.renaming else {
            return;
        };
        let resp = Modal::new(Id::new("ochrepad-rename")).show(ui.ctx(), |ui| {
            ui.label(format!("Rename {old} to:"));
            let field = ui.add(
                egui::TextEdit::singleline(new)
                    .id(Id::new("ochrepad-rename-field"))
                    .desired_width(280.0),
            );
            if !field.has_focus() && !field.lost_focus() {
                field.request_focus();
            }
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Rename").clicked() || enter {
                    Some(true)
                } else if ui.button("Cancel").clicked() {
                    Some(false)
                } else {
                    None
                }
            })
            .inner
        });
        let choice = if resp.should_close() {
            Some(false)
        } else {
            resp.inner
        };
        match choice {
            Some(true) => {
                let Some(Rename::Asking {
                    buffer,
                    caret,
                    old,
                    new,
                }) = self.renaming.take()
                else {
                    return;
                };
                let new = new.trim().to_owned();
                if new.is_empty() || new == old {
                    return;
                }
                let (Some(doc), Some(units)) = (
                    self.editor.buffer(buffer).map(|b| b.doc().clone()),
                    self.language_servers
                        .session_of(buffer)
                        .map(|s| s.client.units),
                ) else {
                    return;
                };
                let sent = self.language_servers.request(
                    buffer,
                    &doc,
                    "renameProvider",
                    "textDocument/rename",
                    |c, uri| {
                        json!({
                            "textDocument": { "uri": uri },
                            "position": c.position(&doc, caret),
                            "newName": new,
                        })
                    },
                    Duration::from_secs(60),
                );
                match sent {
                    Some((tag, _)) => self.renaming = Some(Rename::Renaming { tag, units }),
                    None => self.note_text("The language server stopped"),
                }
                self.view_state_mut().request_focus();
            }
            Some(false) => {
                self.renaming = None;
                self.view_state_mut().request_focus();
            }
            None => {}
        }
    }

    /// Applies a server's workspace edit; the number of files changed.
    /// Nothing is applied unless every edit fits its document.
    pub(super) fn apply_workspace_edit(
        &mut self,
        edit: &Value,
        units: ColumnUnits,
    ) -> Result<usize, String> {
        let files = workspace_edits(edit)?;
        let active = self.active;
        // Every document, opening files that are not open.
        let mut targets = Vec::new();
        for f in &files {
            let buffer = match self.open_buffer_at(&f.path) {
                Some(b) => {
                    if let Some(v) = f.version
                        && self.language_servers.version_of(b) != Some(v)
                    {
                        return Err(format!(
                            "{} changed since the language server looked at it",
                            f.path.display()
                        ));
                    }
                    b
                }
                None => {
                    if !crate::safe_images::is_local_path(&f.path) || !f.path.is_file() {
                        return Err(format!("{} is not a local file", f.path.display()));
                    }
                    self.open_path(&f.path);
                    self.open_buffer_at(&f.path)
                        .ok_or_else(|| format!("{} could not be opened", f.path.display()))?
                }
            };
            let doc = self
                .editor
                .buffer(buffer)
                .map(|b| b.doc().clone())
                .ok_or("a document was closed")?;
            if self.editor.buffer(buffer).is_some_and(|b| b.read_only) {
                return Err(format!("{} is read-only", f.path.display()));
            }
            let edits = text_edits(&doc, &f.edits, units)
                .ok_or_else(|| format!("the edits to {} overlap", f.path.display()))?;
            targets.push((buffer, edits));
        }
        let changed = targets.iter().filter(|(_, e)| !e.is_empty()).count();
        for (buffer, edits) in targets {
            if edits.is_empty() {
                continue;
            }
            let Some(view) = self.editor.views_of(buffer).next().map(|v| v.id()) else {
                continue;
            };
            if let Err(e) = self.editor.apply(view, Action::Edits(edits)) {
                self.note(&e);
            }
        }
        self.activate(active);
        Ok(changed)
    }

    /// Applies a server's edits (`TextEdit`s) to `buffer`, whose text is
    /// `doc`, as one undo step.
    fn apply_edits_to(
        &mut self,
        buffer: BufferId,
        doc: &Document,
        edits: &[Value],
        units: ColumnUnits,
    ) -> Result<(), String> {
        let edits = text_edits(doc, edits, units).ok_or("the edits overlap")?;
        if edits.is_empty() {
            return Ok(());
        }
        let view = self
            .editor
            .views_of(buffer)
            .next()
            .map(|v| v.id())
            .ok_or("the document was closed")?;
        self.editor
            .apply(view, Action::Edits(edits))
            .map_err(|e| format!("{e:?}"))
    }

    /// The open document saved at `path`.
    fn open_buffer_at(&self, path: &std::path::Path) -> Option<BufferId> {
        let path = plain_path(path);
        self.editor
            .buffers()
            .find(|b| b.meta.path.as_deref().map(plain_path).as_deref() == Some(&*path))
            .map(|b| b.id())
    }

    fn doc_and_caret(&self) -> Option<(Document, ByteOffset)> {
        let doc = self.editor.buffer(self.buffer_id())?.doc().clone();
        let caret = self.editor.view(self.active)?.selections.primary().head;
        Some((doc, caret))
    }
}

/// The word at `caret` (letters, digits and `_`).
fn word_at(doc: &Document, caret: ByteOffset) -> String {
    let start = ochrepad_core::complete::word_start(doc, caret, &[]);
    let line_end = doc.line_end(doc.line_of(caret));
    let after: usize = doc
        .slice(caret..line_end)
        .chars()
        .take_while(|&c| ochrepad_core::complete::is_word_char(c, &[]))
        .map(char::len_utf8)
        .sum();
    doc.slice(start..ByteOffset(caret.0 + after))
}

/// A server's `TextEdit`s as byte ranges in `doc`, sorted; `None` if they
/// overlap (or one is not an edit).
fn text_edits(doc: &Document, edits: &[Value], units: ColumnUnits) -> Option<Vec<TextEdit>> {
    let pos = |p: &Value| -> Option<ByteOffset> {
        let n = |k: &str| {
            p.get(k)?
                .as_u64()
                .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
        };
        Some(from_lsp_position(doc, n("line")?, n("character")?, units))
    };
    let mut out = Vec::with_capacity(edits.len());
    for e in edits {
        let range = e.get("range")?;
        let (a, b) = (pos(range.get("start")?)?, pos(range.get("end")?)?);
        out.push(TextEdit {
            start: a.min(b),
            end: a.max(b),
            text: e.get("newText")?.as_str()?.to_owned(),
        });
    }
    // Inserts at one place keep the server's order.
    out.sort_by_key(|e| (e.start, e.end));
    out.windows(2)
        .all(|w| w[0].end <= w[1].start)
        .then_some(out)
}

/// A `WorkspaceEdit`'s edits by file: `documentChanges` (text edits only;
/// creating, renaming and deleting files is refused) or `changes`. Only
/// local files.
fn workspace_edits(v: &Value) -> Result<Vec<FileEdits>, String> {
    let path_of =
        |uri: &str| file_path(&normalize(uri)).ok_or_else(|| format!("{uri} is not a local file"));
    let mut out: Vec<FileEdits> = Vec::new();
    if let Some(changes) = v.get("documentChanges").and_then(Value::as_array) {
        for c in changes {
            if c.get("kind").is_some() {
                return Err(
                    "the language server wanted to create, rename or delete files, \
                            which ochrepad does not do for it"
                        .into(),
                );
            }
            let doc = c.get("textDocument").ok_or("a change names no document")?;
            let uri = doc
                .get("uri")
                .and_then(Value::as_str)
                .ok_or("a change names no document")?;
            out.push(FileEdits {
                path: path_of(uri)?,
                version: doc.get("version").and_then(Value::as_i64),
                edits: c
                    .get("edits")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            });
        }
    } else if let Some(changes) = v.get("changes").and_then(Value::as_object) {
        for (uri, edits) in changes {
            out.push(FileEdits {
                path: path_of(uri)?,
                version: None,
                edits: edits.as_array().cloned().unwrap_or_default(),
            });
        }
    }
    // One entry per file.
    out.sort_by(|a, b| a.path.cmp(&b.path));
    let mut merged: Vec<FileEdits> = Vec::new();
    for f in out {
        match merged.last_mut().filter(|m| m.path == f.path) {
            Some(m) => m.edits.extend(f.edits),
            None => merged.push(f),
        }
    }
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(name: &str) -> String {
        if cfg!(windows) {
            format!("file:///C:/p/{name}")
        } else {
            format!("file:///p/{name}")
        }
    }

    #[test]
    fn edits_in_server_positions() {
        let doc = Document::new("let é = 1;\nlet b = é;\n");
        let edits = text_edits(
            &doc,
            &[
                json!({"range": {"start": {"line": 1, "character": 8}, "end": {"line": 1, "character": 9}}, "newText": "x"}),
                json!({"range": {"start": {"line": 0, "character": 4}, "end": {"line": 0, "character": 5}}, "newText": "x"}),
            ],
            ColumnUnits::Utf16,
        )
        .unwrap();
        assert_eq!(
            edits
                .iter()
                .map(|e| (e.start.0, e.end.0))
                .collect::<Vec<_>>(),
            [(4, 6), (20, 22)]
        );
        let overlapping = [
            json!({"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 5}}, "newText": ""}),
            json!({"range": {"start": {"line": 0, "character": 4}, "end": {"line": 0, "character": 6}}, "newText": ""}),
        ];
        assert!(text_edits(&doc, &overlapping, ColumnUnits::Utf16).is_none());
    }

    #[test]
    fn workspace_edits_by_file() {
        let e = json!({"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}}, "newText": "x"});
        let files = workspace_edits(
            &json!({"changes": {uri("a.rs"): [e.clone()], uri("b.rs"): [e.clone(), e.clone()]}}),
        )
        .unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[1].edits.len(), 2);
        let files = workspace_edits(&json!({"documentChanges": [
            {"textDocument": {"uri": uri("a.rs"), "version": 3}, "edits": [e.clone()]},
            {"textDocument": {"uri": uri("a.rs"), "version": 3}, "edits": [e.clone()]},
        ]}))
        .unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].version, Some(3));
        assert_eq!(files[0].edits.len(), 2);
        assert!(
            workspace_edits(&json!({"documentChanges": [{"kind": "create", "uri": uri("c.rs")}]}))
                .is_err()
        );
        assert!(workspace_edits(&json!({"changes": {"file://server/share/a.rs": [e]}})).is_err());
    }
}
