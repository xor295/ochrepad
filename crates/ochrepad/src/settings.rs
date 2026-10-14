//! User settings (PLAN M13), saved as `settings.json` in the config
//! directory, and the Preferences window.

use std::path::PathBuf;

use egui::Ui;
use serde::{Deserialize, Serialize};

/// How many recent files to remember.
pub const MAX_RECENT: usize = 15;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeChoice {
    #[default]
    System,
    Dark,
    Light,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    /// Name of a system monospace font; empty for the built-in one.
    pub font: String,
    pub font_size: f32,
    pub tab_width: usize,
    /// The Tab key inserts spaces.
    pub use_spaces: bool,
    pub theme: ThemeChoice,
    /// `None` follows light/dark mode.
    pub syntax_theme: Option<String>,
    /// Periodic backups and session snapshot.
    pub backups: bool,
    pub autosave_secs: f64,
    /// Save files with a name automatically.
    pub write_through: bool,
    /// Files larger than this are not highlighted.
    pub highlight_limit_mb: usize,
    /// Fold chevrons in the gutter.
    pub fold_margin: bool,
    pub recent_files: Vec<PathBuf>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            font: String::new(),
            font_size: 14.0,
            tab_width: 4,
            use_spaces: false,
            theme: ThemeChoice::System,
            syntax_theme: None,
            backups: true,
            autosave_secs: 7.0,
            write_through: false,
            highlight_limit_mb: 10,
            fold_margin: true,
            recent_files: Vec::new(),
        }
    }
}

impl AppSettings {
    /// Puts `path` first in the recent files list.
    pub fn add_recent(&mut self, path: PathBuf) {
        self.recent_files.retain(|p| *p != path);
        self.recent_files.insert(0, path);
        self.recent_files.truncate(MAX_RECENT);
    }

    /// Keeps values in sensible ranges (the file is user-editable).
    pub fn clamp(&mut self) {
        self.font_size = self.font_size.clamp(6.0, 72.0);
        self.tab_width = self.tab_width.clamp(1, 16);
        self.autosave_secs = self.autosave_secs.clamp(1.0, 3600.0);
        self.highlight_limit_mb = self.highlight_limit_mb.clamp(1, 4096);
    }

    /// The Preferences window body.
    pub fn ui(&mut self, ui: &mut Ui, fonts: &[(String, PathBuf)], themes: &[&str]) {
        egui::Grid::new("preferences")
            .num_columns(2)
            .spacing([16.0, 8.0])
            .show(ui, |ui| {
                ui.label("Font");
                let shown = if self.font.is_empty() {
                    "Built-in (Hack)".to_owned()
                } else {
                    self.font.clone()
                };
                egui::ComboBox::from_id_salt("pref-font")
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.font, String::new(), "Built-in (Hack)");
                        for (name, _) in fonts {
                            ui.selectable_value(&mut self.font, name.clone(), name);
                        }
                    });
                ui.end_row();

                ui.label("Font size");
                ui.add(
                    egui::DragValue::new(&mut self.font_size)
                        .range(6.0..=72.0)
                        .speed(0.5),
                );
                ui.end_row();

                ui.label("Tab width");
                ui.add(egui::DragValue::new(&mut self.tab_width).range(1..=16));
                ui.end_row();

                ui.label("Indent with");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.use_spaces, false, "Tabs");
                    ui.radio_value(&mut self.use_spaces, true, "Spaces");
                });
                ui.end_row();

                ui.label("Theme");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.theme, ThemeChoice::System, "System");
                    ui.radio_value(&mut self.theme, ThemeChoice::Dark, "Dark");
                    ui.radio_value(&mut self.theme, ThemeChoice::Light, "Light");
                });
                ui.end_row();

                ui.label("Syntax theme");
                egui::ComboBox::from_id_salt("pref-syntax")
                    .selected_text(self.syntax_theme.as_deref().unwrap_or("Automatic"))
                    .height(300.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.syntax_theme, None, "Automatic");
                        for t in themes {
                            ui.selectable_value(&mut self.syntax_theme, Some((*t).to_owned()), *t);
                        }
                    });
                ui.end_row();

                ui.label("Backups and session");
                ui.checkbox(&mut self.backups, "Back up unsaved work and restore it");
                ui.end_row();

                ui.label("Autosave every");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut self.autosave_secs).range(1.0..=3600.0));
                    ui.label("seconds");
                });
                ui.end_row();

                ui.label("");
                ui.checkbox(&mut self.write_through, "Also save files that have a name");
                ui.end_row();

                ui.label("Highlight files up to");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut self.highlight_limit_mb).range(1..=4096));
                    ui.label("MB");
                });
                ui.end_row();
            });
    }
}
