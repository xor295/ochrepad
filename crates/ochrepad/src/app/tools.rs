//! Edit > Text Tools (PLAN_v0.3.0 P4): JSON, Base64 and URL encoding, and
//! the hash dialog. The edits are core actions; Validate JSON and the
//! hashes only read the document.

use egui::{Id, Modal, RichText, Ui};
use ochrepad_core::text_tools::{self, Hashes};
use ochrepad_core::{Action, ActionError, Codec, LineEnding, Selection, SelectionSet};

use super::OchrepadApp;

/// The hash dialog while it is open.
pub(super) struct HashDialog {
    /// "the selection" or "the document".
    of: &'static str,
    ending: LineEnding,
    hashes: Hashes,
}

impl OchrepadApp {
    /// Edit > Text Tools.
    pub(super) fn text_tools_menu(&mut self, ui: &mut Ui) {
        if ui.button("Format JSON").clicked() {
            self.apply(Action::FormatJson { indent: None });
        }
        if ui.button("Minify JSON").clicked() {
            self.apply(Action::MinifyJson);
        }
        if ui.button("Validate JSON").clicked() {
            self.validate_json();
        }
        ui.separator();
        for (label, action) in [
            ("Base64 Encode", Action::Encode(Codec::Base64)),
            ("Base64 Decode", Action::Decode(Codec::Base64)),
            ("URL Encode", Action::Encode(Codec::Url)),
            ("URL Decode", Action::Decode(Codec::Url)),
        ] {
            if ui.button(label).clicked() {
                self.apply(action);
            }
        }
        ui.separator();
        if ui.button("Hashes…").clicked() {
            self.show_hashes();
        }
    }

    /// Text Tools > Validate JSON: a note when the selection (or the
    /// document) is valid JSON; otherwise the caret goes to the problem and
    /// a message says what it is.
    pub fn validate_json(&mut self) {
        let Some(view) = self.editor.view(self.active) else {
            return;
        };
        let whole = view.selections.ranges().iter().all(Selection::is_empty);
        let b = self.buffer();
        match text_tools::validate_json_in(b.doc(), &view.selections, b.indent.width) {
            Ok(()) => {
                let of = if whole {
                    "The document"
                } else {
                    "The selection"
                };
                self.note_text(format!("{of} is valid JSON"));
            }
            Err(e) => {
                self.json_error(&e);
            }
        }
    }

    /// Moves the caret to where JSON stops being valid and says why. False
    /// for any other error.
    pub(super) fn json_error(&mut self, error: &ActionError) -> bool {
        let ActionError::InvalidJson(e) = error else {
            return false;
        };
        let _ = self.editor.apply(
            self.active,
            Action::SetSelections(SelectionSet::caret(e.at)),
        );
        self.view_state_mut().reveal_caret();
        self.message = Some(format!(
            "Invalid JSON on line {}, column {}:\n{}.",
            e.line, e.column, e.message
        ));
        true
    }

    /// Text Tools > Hashes…: MD5, SHA-1 and SHA-256 of the selection, or of
    /// the document.
    pub fn show_hashes(&mut self) {
        let Some(view) = self.editor.view(self.active) else {
            return;
        };
        let b = self.buffer();
        let ending = b.meta.line_ending;
        let (hashes, whole) = text_tools::hash_selections(b.doc(), &view.selections, ending);
        self.hashes = Some(HashDialog {
            of: if whole {
                "the document"
            } else {
                "the selection"
            },
            ending,
            hashes,
        });
    }

    pub(super) fn hash_dialog(&mut self, ui: &mut Ui) {
        let Some(d) = &self.hashes else {
            return;
        };
        let mut close = false;
        let resp = Modal::new(Id::new("ochrepad-hashes")).show(ui.ctx(), |ui| {
            ui.label(RichText::new("Hashes").strong());
            ui.label(describe(d));
            ui.add_space(8.0);
            egui::Grid::new("ochrepad-hashes-grid")
                .num_columns(3)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    for (name, value) in [
                        ("MD5", &d.hashes.md5),
                        ("SHA-1", &d.hashes.sha1),
                        ("SHA-256", &d.hashes.sha256),
                    ] {
                        ui.label(name);
                        ui.label(RichText::new(value.as_str()).monospace());
                        if ui
                            .button("Copy")
                            .on_hover_text(format!("Copy the {name} hash"))
                            .clicked()
                        {
                            ui.ctx().copy_text(value.clone());
                        }
                        ui.end_row();
                    }
                });
            ui.add_space(8.0);
            let button = ui.button("Close");
            // Keep the keyboard here, so typing does not reach the editor.
            if ui.memory(|m| m.focused().is_none()) {
                button.request_focus();
            }
            close = button.clicked();
        });
        if close || resp.should_close() {
            self.hashes = None;
            self.view_state_mut().request_focus();
        }
    }
}

/// "Of the selection: 3 bytes of UTF-8."
fn describe(d: &HashDialog) -> String {
    let bytes = match d.hashes.bytes {
        1 => "1 byte".to_owned(),
        n => format!("{n} bytes"),
    };
    let breaks = if d.hashes.line_breaks == 0 {
        ""
    } else {
        match d.ending {
            LineEnding::CrLf => ", line breaks as CR LF",
            LineEnding::Lf => ", line breaks as LF",
            LineEnding::Cr => ", line breaks as CR",
        }
    };
    format!("Of {}: {bytes} of UTF-8{breaks}.", d.of)
}
