//! Per-document settings in the status bar (PLAN_v0.3.0 P2): indentation
//! (detected on open, chosen per document, converted), line endings and
//! encoding (Reopen with Encoding, Save with Encoding).

use egui::Ui;
use ochrepad_core::{Action, BufferId, LineEnding};
use ochrepad_fileio as fileio;

use super::OchrepadApp;

impl OchrepadApp {
    /// Sets `buffer`'s indentation from what its text uses (or leaves the
    /// preferences' default when nothing is indented).
    pub(super) fn detect_indent(&mut self, buffer: BufferId) {
        let default = self.editor.settings.indent();
        if let Some(b) = self.editor.buffer_mut(buffer) {
            let found = ochrepad_core::indent::detect(b.doc());
            b.indent = found.apply(default);
            b.indent_set = found.use_spaces.is_some();
            b.indent_mixed = found.mixed;
        }
    }

    /// The indentation menu: "Spaces: 4", "Tabs: 4", or a mixed warning.
    pub(super) fn indent_menu(&mut self, ui: &mut Ui) {
        let id = self.buffer_id();
        let b = self.buffer();
        let (indent, mixed) = (b.indent, b.indent_mixed);
        let label = format!(
            "{}{}: {}",
            if mixed { "Mixed, " } else { "" },
            if indent.use_spaces { "Spaces" } else { "Tabs" },
            indent.width
        );
        let resp = ui.menu_button(label, |ui| {
            let set = |app: &mut Self, f: &dyn Fn(&mut ochrepad_core::Indent)| {
                if let Some(b) = app.editor.buffer_mut(id) {
                    f(&mut b.indent);
                    b.indent_set = true;
                }
            };
            if ui.radio(indent.use_spaces, "Indent Using Spaces").clicked() {
                set(self, &|i| i.use_spaces = true);
            }
            if ui.radio(!indent.use_spaces, "Indent Using Tabs").clicked() {
                set(self, &|i| i.use_spaces = false);
            }
            ui.separator();
            for w in [2, 4, 8] {
                if ui.radio(indent.width == w, format!("Width {w}")).clicked() {
                    set(self, &|i| i.width = w);
                }
            }
            ui.separator();
            if ui.button("Convert Indentation to Spaces").clicked() {
                self.apply(Action::ConvertIndentation { to_spaces: true });
            }
            if ui.button("Convert Indentation to Tabs").clicked() {
                self.apply(Action::ConvertIndentation { to_spaces: false });
            }
        });
        resp.response.on_hover_text(if mixed {
            "Some lines are indented with tabs and others with spaces. \
             Convert Indentation makes them all the same."
        } else {
            "Indentation for this document"
        });
    }

    /// The line ending menu: choosing one changes how the document is saved.
    pub(super) fn line_ending_menu(&mut self, ui: &mut Ui) {
        let id = self.buffer_id();
        let current = self.buffer().meta.line_ending;
        let name = |e: LineEnding| match e {
            LineEnding::CrLf => "Windows (CR LF)",
            LineEnding::Lf => "Unix (LF)",
            LineEnding::Cr => "Macintosh (CR)",
        };
        ui.menu_button(name(current), |ui| {
            for e in [LineEnding::CrLf, LineEnding::Lf, LineEnding::Cr] {
                if ui.radio(current == e, name(e)).clicked()
                    && let Some(b) = self.editor.buffer_mut(id)
                {
                    if b.meta.line_ending != e || b.meta.mixed_line_endings {
                        b.meta.line_ending = e;
                        b.meta.mixed_line_endings = false;
                        b.mark_modified();
                    }
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("Line endings the document is saved with");
    }

    /// The encoding menu: reopen the file decoded another way, or save it
    /// in another encoding.
    pub(super) fn encoding_menu(&mut self, ui: &mut Ui) {
        let meta = self.buffer().meta.clone();
        let label = if meta.bom {
            format!("{} BOM", meta.encoding)
        } else {
            meta.encoding.clone()
        };
        ui.menu_button(label, |ui| {
            let can_reopen = meta.path.is_some() && !self.buffer().is_dirty();
            ui.add_enabled_ui(can_reopen, |ui| {
                ui.menu_button("Reopen with Encoding", |ui| {
                    for (name, shown) in fileio::ENCODINGS {
                        if ui.button(*shown).clicked() {
                            self.reopen_with_encoding(name);
                            ui.close();
                        }
                    }
                })
            })
            .response
            .on_disabled_hover_text(
                "Save or undo the changes first: reopening reads the file again",
            );
            ui.menu_button("Save with Encoding", |ui| {
                if ui.button("UTF-8 with BOM").clicked() {
                    self.save_with_encoding("UTF-8", true);
                    ui.close();
                }
                for (name, shown) in fileio::ENCODINGS {
                    if ui.button(*shown).clicked() {
                        self.save_with_encoding(name, name.starts_with("UTF-16"));
                        ui.close();
                    }
                }
            });
        })
        .response
        .on_hover_text("Encoding");
    }

    /// Reads the active document's file again, decoded as `encoding`.
    pub fn reopen_with_encoding(&mut self, encoding: &str) {
        let id = self.buffer_id();
        let Some(path) = self.buffer().meta.path.clone() else {
            return;
        };
        if self.buffer().is_dirty() {
            self.message =
                Some("Save or undo the changes first: reopening reads the file again.".to_owned());
            return;
        }
        match fileio::load_as(&path, encoding) {
            Ok(d) => match self.editor.reload_buffer(id, &d.text, d.meta) {
                Ok(()) => {
                    self.record_disk_stamp(id);
                    self.detect_indent(id);
                }
                Err(e) => self.message = Some(format!("Could not reopen {}:\n{e}", path.display())),
            },
            Err(e) => {
                self.message = Some(format!(
                    "{} cannot be read as {encoding}:\n{e}",
                    path.display()
                ));
            }
        }
    }

    /// From now on the active document is saved as `encoding` (UTF-16 and
    /// "UTF-8 with BOM" with a byte order mark). Refused, listing the
    /// characters it cannot hold, if the text does not fit the encoding.
    pub fn save_with_encoding(&mut self, encoding: &str, bom: bool) {
        const SHOWN: usize = 10;
        let id = self.buffer_id();
        let b = self.buffer();
        let mut meta = b.meta.clone();
        meta.encoding = encoding.to_owned();
        meta.bom = bom;
        let text = b.doc().to_string();
        let bad = fileio::unencodable(&text, encoding, SHOWN + 1);
        if !bad.is_empty() {
            let mut msg = format!("These characters cannot be saved as {encoding}:\n");
            for (ch, line) in bad.iter().take(SHOWN) {
                msg += &format!("\n{ch:?} (U+{:04X}) on line {line}", u32::from(*ch));
            }
            if bad.len() > SHOWN {
                msg += "\n…and more";
            }
            msg += "\n\nThe encoding is unchanged.";
            self.message = Some(msg);
            return;
        }
        if let Err(e) = fileio::encode(&text, &meta) {
            self.message = Some(e.to_string());
            return;
        }
        if let Some(b) = self.editor.buffer_mut(id)
            && (b.meta.encoding != meta.encoding || b.meta.bom != meta.bom)
        {
            b.meta = meta;
            b.mark_modified();
        }
    }
}
