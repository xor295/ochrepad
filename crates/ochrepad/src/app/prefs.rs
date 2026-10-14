//! Settings (applied every frame, saved when changed), the keymap file,
//! recent files and drag-and-drop (PLAN M13).

use std::path::{Path, PathBuf};

use egui::{Align2, Color32, FontId, Id, Ui};
use ochrepad_core::FileMeta;
use ochrepad_fileio as fileio;

use super::OchrepadApp;
use crate::keymap::Keymap;
use crate::settings::{AppSettings, ThemeChoice};

impl OchrepadApp {
    /// Loads `settings.json` and `keymap.json` from `config_dir` and uses
    /// them from now on.
    pub(super) fn load_config(&mut self, config_dir: &Path) {
        let settings_file = config_dir.join("settings.json");
        match fileio::load_json::<AppSettings>(&settings_file) {
            Ok(Some(mut s)) => {
                s.clamp();
                self.settings = s;
            }
            Ok(None) => {}
            Err(e) => self.message = Some(format!("Could not read the settings:\n{e}")),
        }
        self.saved_settings = Some(self.settings.clone());
        self.settings_file = Some(settings_file);
        self.keymap_file = Some(config_dir.join("keymap.json"));
        self.reload_keymap();
    }

    /// Re-reads `keymap.json`; on an error the previous keymap stays.
    pub(super) fn reload_keymap(&mut self) {
        let Some(path) = &self.keymap_file else {
            return;
        };
        match std::fs::read_to_string(path) {
            Ok(json) => match Keymap::with_file(&json) {
                Ok(k) => self.keymap = k,
                Err(e) => self.message = Some(format!("The keymap was not changed:\n{e}")),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => self.keymap = Keymap::default(),
            Err(e) => self.message = Some(format!("Could not read {}:\n{e}", path.display())),
        }
    }

    pub(super) fn is_keymap_file(&self, path: &Path) -> bool {
        self.keymap_file
            .as_deref()
            .is_some_and(|k| super::canonical(k) == super::canonical(path))
    }

    /// Opens `keymap.json` for editing, starting from every default binding
    /// if it does not exist yet. Saving it applies it.
    pub fn edit_keymap(&mut self) {
        let Some(path) = self.keymap_file.clone() else {
            self.message = Some("There is no configuration directory for a keymap.".to_owned());
            return;
        };
        if path.exists() {
            self.open_path(&path);
            return;
        }
        // So that saving it works on a first run.
        if let Some(dir) = path.parent()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            self.message = Some(format!("Could not create {}:\n{e}", dir.display()));
            return;
        }
        let json = serde_json::to_string_pretty(&Keymap::default_entries()).unwrap_or_default();
        let meta = FileMeta {
            path: Some(path),
            line_ending: fileio::default_line_ending(),
            ..FileMeta::default()
        };
        let buffer = self.editor.add_buffer(&json, meta);
        if let Some(b) = self.editor.buffer_mut(buffer) {
            b.mark_modified();
        }
        self.attach_highlighting(buffer);
        self.add_tab(buffer);
    }

    /// When the settings changed: pushes them into the parts that use them
    /// and saves them (in the background).
    pub(super) fn apply_settings(&mut self, ui: &Ui) {
        if self.applied_settings.as_ref() == Some(&self.settings) {
            return;
        }
        if self.applied_settings.is_none() {
            crate::theme::install(ui.ctx());
            // Ctrl+plus/minus/0 zoom the editors' text (View > Zoom), not the
            // whole window as egui would.
            ui.ctx().options_mut(|o| o.zoom_with_keyboard = false);
        }
        self.applied_settings = Some(self.settings.clone());
        let s = self.settings.clone();
        self.editor.settings.tab_width = s.tab_width;
        self.editor.settings.use_spaces = s.use_spaces;
        self.editor.settings.auto_close = s.auto_close;
        self.editor.settings.smart_indent = s.smart_indent;
        // Documents whose indentation was detected or chosen keep it.
        let indent = self.editor.settings.indent();
        for b in self.editor.buffers_mut().filter(|b| !b.indent_set) {
            b.indent = indent;
        }
        let a = self.autosave_settings();
        a.snapshots = s.backups;
        a.interval_secs = s.autosave_secs;
        a.write_through = s.write_through;
        self.highlighting.theme_override = s.syntax_theme.clone();
        self.log_rules = crate::log_rules::LogRules::new(&s.highlight_rules, &s.log_files);
        self.highlighting.size_limit = s.highlight_limit_mb * 1024 * 1024;
        if self.applied_theme != Some(s.theme) {
            self.applied_theme = Some(s.theme);
            ui.ctx().set_theme(match s.theme {
                ThemeChoice::System => egui::ThemePreference::System,
                ThemeChoice::Dark => egui::ThemePreference::Dark,
                ThemeChoice::Light => egui::ThemePreference::Light,
            });
        }
        if self.applied_font != s.font {
            self.applied_font = s.font.clone();
            let path = self
                .font_choices
                .iter()
                .find(|(n, _)| *n == s.font)
                .map(|(_, p)| p.clone());
            crate::fonts::install_with(ui.ctx(), path.as_deref());
        }
        if let Some(file) = self.settings_file.clone()
            && self.saved_settings.as_ref() != Some(&self.settings)
            && let Ok(bytes) = serde_json::to_vec_pretty(&self.settings)
        {
            self.saved_settings = Some(self.settings.clone());
            self.write_config(file, bytes);
        }
    }

    pub(super) fn preferences_window(&mut self, ui: &Ui) {
        if !self.show_preferences {
            return;
        }
        let mut open = true;
        let themes = ochrepad_syntax::Registry::theme_names();
        egui::Window::new("Preferences")
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ui.ctx().content_rect().center())
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ui.ctx(), |ui| {
                self.settings
                    .ui(ui, &self.font_choices, &themes, &self.log_rules.errors);
            });
        self.show_preferences = open;
    }

    pub(super) fn recent_files_menu(&mut self, ui: &mut Ui) {
        let recent = self.settings.recent_files.clone();
        ui.add_enabled_ui(!recent.is_empty(), |ui| {
            ui.menu_button("Recent Files", |ui| {
                for path in &recent {
                    let name = path.file_name().map_or_else(
                        || path.display().to_string(),
                        |n| n.to_string_lossy().into_owned(),
                    );
                    if ui
                        .button(name)
                        .on_hover_text(path.display().to_string())
                        .clicked()
                    {
                        if path.exists() {
                            self.open_path(path);
                        } else {
                            self.settings.recent_files.retain(|p| p != path);
                            self.message = Some(format!("{} no longer exists.", path.display()));
                        }
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("Clear Recent Files").clicked() {
                    self.settings.recent_files.clear();
                }
            });
        });
    }

    /// Opens files dropped on the window, and shows a hint while dragging.
    pub(super) fn drag_and_drop(&mut self, ui: &Ui) {
        let (hovering, dropped): (bool, Vec<PathBuf>) = ui.input(|i| {
            (
                !i.raw.hovered_files.is_empty(),
                i.raw
                    .dropped_files
                    .iter()
                    .map(|f| f.path().to_path_buf())
                    .collect(),
            )
        });
        for path in dropped {
            if path.is_file() {
                self.open_path(&path);
            } else if path.is_dir() {
                self.open_folder(&path);
            }
        }
        if hovering {
            let rect = ui.ctx().content_rect();
            let painter = ui.ctx().layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                Id::new("ochrepad-drop"),
            ));
            painter.rect_filled(rect, 0.0, Color32::from_black_alpha(140));
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Drop files to open them",
                FontId::proportional(24.0),
                Color32::WHITE,
            );
        }
    }
}
