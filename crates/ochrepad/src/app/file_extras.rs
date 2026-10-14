//! More of Notepad++'s File menu: Save All, Save a Copy As…, Reload from
//! Disk and Rename…. Writes and renames go through the worker, as every
//! write to a user file does.

use std::path::{Path, PathBuf};

use egui::{Id, Key, Modal, TextEdit, Ui};
use ochrepad_core::BufferId;
use ochrepad_fileio::worker::Job;

use super::OchrepadApp;
use super::autosave::PendingJob;

/// The File > Rename… prompt.
pub(super) struct RenameDialog {
    buffer: BufferId,
    from: PathBuf,
    name: String,
    error: Option<String>,
}

impl OchrepadApp {
    /// File > Save All: every document with a file and unsaved changes.
    /// Documents never saved are left for Save As.
    pub fn save_all(&mut self) {
        let dirty: Vec<(BufferId, bool)> = self
            .editor
            .buffers()
            .filter(|b| b.is_dirty() && !b.read_only)
            .map(|b| (b.id(), b.meta.path.is_some()))
            .collect();
        let mut saved = 0;
        for (buffer, named) in &dirty {
            if *named && self.save_buffer(*buffer) {
                saved += 1;
            }
        }
        let unnamed = dirty.iter().filter(|(_, named)| !named).count();
        let mut note = match saved {
            0 => "Nothing to save".to_owned(),
            1 => "Saving 1 document".to_owned(),
            n => format!("Saving {n} documents"),
        };
        if unnamed > 0 {
            note.push_str(&format!(
                "; {unnamed} new {} without a name (File > Save As)",
                if unnamed == 1 {
                    "document"
                } else {
                    "documents"
                }
            ));
        }
        self.note_text(note);
    }

    /// File > Save a Copy As…: writes the document to another file, which
    /// it does not become (its tab keeps its own file).
    pub fn save_copy_as(&mut self) {
        let buffer = self.buffer_id();
        let name = self.buffer_name(buffer);
        let Some(path) = rfd::FileDialog::new().set_file_name(name).save_file() else {
            return;
        };
        self.save_copy_to(buffer, &path);
    }

    /// Writes `buffer`'s text to `path`, in its encoding and line endings,
    /// without changing the buffer.
    pub fn save_copy_to(&mut self, buffer: BufferId, path: &Path) {
        let Some(b) = self.editor.buffer(buffer) else {
            return;
        };
        let tag = self.persist.worker.submit(Job::Save {
            path: path.to_path_buf(),
            doc: b.doc().clone(),
            meta: b.meta.clone(),
        });
        self.persist
            .pending
            .insert(tag, PendingJob::Copy(path.to_path_buf()));
    }

    /// The worker finished Save a Copy As.
    pub(super) fn copy_saved(&mut self, path: &Path, result: Result<(), String>) {
        match result {
            Ok(()) => self.note_text(format!("Saved a copy as {}", path.display())),
            Err(e) => {
                self.message = Some(format!("Could not save a copy as {}:\n{e}", path.display()));
            }
        }
    }

    /// File > Rename…: asks for the active document's new name.
    pub fn open_rename(&mut self) {
        let buffer = self.buffer_id();
        let Some(from) = self.buffer().meta.path.clone() else {
            self.note_text("Save the document first (File > Save As)");
            return;
        };
        let name = from
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.rename = Some(RenameDialog {
            buffer,
            from,
            name,
            error: None,
        });
    }

    pub(super) fn rename_dialog(&mut self, ui: &mut Ui) {
        let Some(d) = &mut self.rename else {
            return;
        };
        let mut answer = None;
        Modal::new(Id::new("ochrepad-rename")).show(ui.ctx(), |ui| {
            ui.label(format!("Rename {} to:", d.from.display()));
            let field = ui.add(
                TextEdit::singleline(&mut d.name)
                    .id(Id::new("ochrepad-rename-field"))
                    .desired_width(320.0),
            );
            if !field.has_focus() && !field.lost_focus() {
                field.request_focus();
            }
            if field.changed() {
                d.error = None;
            }
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if let Some(e) = &d.error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Rename").clicked() || enter {
                    answer = Some(true);
                } else if ui.button("Cancel").clicked() {
                    answer = Some(false);
                }
            });
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                answer = Some(false);
            }
        });
        match answer {
            Some(false) => {
                self.rename = None;
                self.view_state_mut().request_focus();
            }
            Some(true) => {
                let name = d.name.trim();
                if !crate::file_tree::is_plain_name(name) {
                    d.error = Some("Give a file name, without a folder or a drive".to_owned());
                    return;
                }
                let to = d.from.with_file_name(name);
                if to == d.from {
                    self.rename = None;
                    return;
                }
                let (buffer, from) = (d.buffer, d.from.clone());
                self.rename = None;
                if self.editor.buffer(buffer).is_none() {
                    return;
                }
                // As the sidebar's rename: the worker refuses to replace a
                // file, and open documents take the new path when it is done.
                let tag = self.persist.worker.submit(Job::Rename {
                    from: from.clone(),
                    to: to.clone(),
                });
                self.persist.pending.insert(
                    tag,
                    PendingJob::Tree(super::workspace::TreeJob::Rename { from, to }),
                );
                self.view_state_mut().request_focus();
            }
            None => {}
        }
    }
}
