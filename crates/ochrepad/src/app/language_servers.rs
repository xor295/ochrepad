//! Language servers in the app (PLAN_completion L1): the per-frame tick,
//! the question before a server first runs in a folder, View > Language
//! Servers (each server's state and log, Restart and Stop, `lsp.json`),
//! and the status bar's note.

use std::path::Path;

use egui::{Id, Modal, RichText, Ui};
use ochrepad_core::{Action, FileMeta};
use ochrepad_fileio as fileio;

use super::OchrepadApp;
use crate::lsp::{Notice, Trust};

/// What `lsp.json` starts as.
const LSP_TEMPLATE: &str = r#"{
  "servers": [
    {
      "name": "example",
      "command": ["example-language-server", "--stdio"],
      "languages": ["SQL"],
      "settings": {},
      "initializationOptions": null
    }
  ],
  "disabled": []
}
"#;

impl OchrepadApp {
    /// Once a frame, after the documents' edits.
    pub(super) fn language_servers_tick(&mut self, ui: &Ui) {
        let folder = self.tree.as_ref().map(|t| t.root().to_path_buf());
        let s = &self.settings;
        let trust = Trust {
            enabled: s.language_servers,
            trusted: &s.lsp_trusted,
            declined: &s.lsp_declined,
            never: &s.lsp_never,
        };
        self.language_servers.max_bytes = s.highlight_limit_mb << 20;
        let notices =
            self.language_servers
                .tick(&self.editor, folder.as_deref(), &trust, self.now, ui.ctx());
        for n in notices {
            match n {
                Notice::Note(text) => self.note_text(text),
                Notice::Show { path, line } => {
                    // Only a local file, as Open File in the editor's menu.
                    if crate::safe_images::is_local_path(&path) && path.is_file() {
                        self.open_file_at(&path, line.map(|l| (l as usize + 1, 1)));
                    }
                }
            }
        }
    }

    /// The question before a server first runs in a folder.
    pub(super) fn language_server_question(&mut self, ui: &mut Ui) {
        let Some(q) = self.language_servers.question.clone() else {
            return;
        };
        enum Answer {
            Start,
            NotHere,
            Never,
        }
        let mut answer = None;
        Modal::new(Id::new("ochrepad-lsp-trust")).show(ui.ctx(), |ui| {
            ui.set_max_width(520.0);
            ui.label(RichText::new(format!("Start {} for this folder?", q.server)).heading());
            ui.add_space(8.0);
            ui.label(format!("Folder: {}", q.root.display()));
            ui.label(format!("Program: {}", q.program));
            ui.add_space(8.0);
            ui.label(
                "A language server reads the folder's code and can run some of it \
                 (rust-analyzer runs build scripts and procedural macros, for example). \
                 Start it only in a folder you trust.",
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let start = ui.button("Start in this folder");
                if ui.memory(|m| m.focused().is_none()) {
                    start.request_focus();
                }
                if start.clicked() {
                    answer = Some(Answer::Start);
                }
                if ui.button("Not in this folder").clicked() {
                    answer = Some(Answer::NotHere);
                }
                if ui.button(format!("Never start {}", q.server)).clicked() {
                    answer = Some(Answer::Never);
                }
            });
        });
        let Some(answer) = answer else {
            return;
        };
        let s = &mut self.settings;
        match answer {
            Answer::Start => s.lsp_trusted.push(q.root.clone()),
            Answer::NotHere => s.lsp_declined.push(q.root.clone()),
            Answer::Never => s.lsp_never.push(q.server.clone()),
        }
        self.language_servers.question = None;
        self.view_state_mut().request_focus();
    }

    /// View > Language Servers.
    pub(super) fn language_servers_window(&mut self, ui: &mut Ui) {
        if !self.show_language_servers {
            return;
        }
        let mut open = true;
        let mut restart = None;
        let mut stop = None;
        let mut edit_config = false;
        let mut forget = None;
        egui::Window::new("Language Servers")
            .open(&mut open)
            .default_size([640.0, 420.0])
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ui.ctx().content_rect().center())
            .show(ui.ctx(), |ui| {
                ui.checkbox(&mut self.settings.language_servers, "Use language servers");
                let ls = &self.language_servers;
                ui.horizontal(|ui| {
                    if let Some(f) = &ls.lsp_file {
                        ui.label(RichText::new(f.display().to_string()).weak());
                    }
                    edit_config = ui.button("Edit lsp.json…").clicked();
                });
                if let Some(e) = &ls.lsp_error {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                ui.separator();
                if ls.sessions.is_empty() {
                    ui.label(
                        "No server is running. One starts when you open a file of its \
                         language, if its program is on PATH.",
                    );
                }
                for (i, s) in ls.sessions.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(s.status()).strong());
                        if ui.small_button("Restart").clicked() {
                            restart = Some(i);
                        }
                        if ui.small_button("Stop").clicked() {
                            stop = Some(i);
                        }
                    });
                    ui.label(RichText::new(s.root.display().to_string()).weak());
                    egui::CollapsingHeader::new("Log")
                        .id_salt(("ochrepad-lsp-log", i))
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .max_height(200.0)
                                .stick_to_bottom(true)
                                .show(ui, |ui| {
                                    if let Ok(log) = s.log.lock() {
                                        let lines: Vec<&str> = log.lines().collect();
                                        let start = lines.len().saturating_sub(500);
                                        for line in &lines[start..] {
                                            ui.label(RichText::new(*line).monospace().small());
                                        }
                                    }
                                });
                        });
                    ui.separator();
                }
                if !self.settings.lsp_trusted.is_empty() {
                    egui::CollapsingHeader::new("Folders where servers may run").show(ui, |ui| {
                        for (i, f) in self.settings.lsp_trusted.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(f.display().to_string());
                                if ui.small_button("Forget").clicked() {
                                    forget = Some(i);
                                }
                            });
                        }
                    });
                }
            });
        if let Some(i) = restart {
            self.language_servers.restart(i);
        }
        if let Some(i) = stop {
            self.language_servers.stop(i);
        }
        if let Some(i) = forget {
            // Asked again the next time a document there is open.
            self.settings.lsp_trusted.remove(i);
        }
        if edit_config {
            self.edit_lsp_config();
        }
        if !open {
            self.show_language_servers = false;
        }
    }

    /// Opens `lsp.json`, starting from an example if it does not exist.
    fn edit_lsp_config(&mut self) {
        let Some(path) = self.language_servers.lsp_file.clone() else {
            self.message = Some("There is no configuration directory for lsp.json.".to_owned());
            return;
        };
        if path.exists() {
            self.open_path(&path);
            return;
        }
        if let Some(dir) = path.parent()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            self.message = Some(format!("Could not create {}:\n{e}", dir.display()));
            return;
        }
        let meta = FileMeta {
            path: Some(path),
            line_ending: fileio::default_line_ending(),
            ..FileMeta::default()
        };
        let buffer = self.editor.add_buffer(LSP_TEMPLATE, meta);
        if let Some(b) = self.editor.buffer_mut(buffer) {
            b.mark_modified();
        }
        self.attach_highlighting(buffer);
        self.add_tab(buffer);
        self.apply(Action::GoToLine { line: 1, column: 1 });
    }

    pub(super) fn is_lsp_file(&self, path: &Path) -> bool {
        self.language_servers
            .lsp_file
            .as_deref()
            .is_some_and(|f| super::canonical(f) == super::canonical(path))
    }

    /// The status bar's note on the active document's server.
    pub(super) fn language_server_status(&mut self, ui: &mut Ui) {
        let Some(s) = self.language_servers.session_of(self.buffer_id()) else {
            return;
        };
        ui.separator();
        let diagnostics = self.language_servers.diagnostics_of(self.buffer_id());
        let count = |severity| {
            diagnostics
                .iter()
                .filter(|d| d.severity == severity)
                .count()
        };
        let (errors, warnings) = (
            count(crate::lsp::Severity::Error),
            count(crate::lsp::Severity::Warning),
        );
        let plural = |n: usize, what: &str| match n {
            1 => format!("1 {what}"),
            n => format!("{n} {what}s"),
        };
        let mut text = s.status();
        if errors + warnings > 0 {
            text.push_str(&format!(
                " · {}, {}",
                plural(errors, "error"),
                plural(warnings, "warning")
            ));
        }
        if ui
            .add(
                egui::Label::new(RichText::new(text).weak())
                    .truncate()
                    .sense(egui::Sense::click()),
            )
            .on_hover_text("View > Language Servers")
            .clicked()
        {
            self.show_language_servers = true;
        }
    }
}
