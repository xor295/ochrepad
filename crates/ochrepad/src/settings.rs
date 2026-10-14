//! User settings (PLAN M13), saved as `settings.json` in the config
//! directory, and the Preferences window.

use std::path::PathBuf;

use egui::Ui;
use serde::{Deserialize, Serialize};

/// How many recent files to remember.
pub const MAX_RECENT: usize = 15;

/// The editor zoom levels, in percent: Zoom In and Zoom Out step through
/// them.
pub const ZOOM_LEVELS: [u32; 17] = [
    10, 25, 33, 50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300, 400,
];

/// The zoom `steps` levels above (or, negative, below) `percent`. A value
/// between two levels counts as the one below it.
pub fn zoom_step(percent: u32, steps: i32) -> u32 {
    let at = ZOOM_LEVELS.iter().rposition(|&l| l <= percent).unwrap_or(0) as i32;
    let i = (at + steps).clamp(0, ZOOM_LEVELS.len() as i32 - 1);
    ZOOM_LEVELS[i as usize]
}

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
    /// Keep each document's undo history after it is closed (PLAN_v0.3.0
    /// P5).
    pub undo_history: bool,
    /// Files larger than this are not highlighted.
    pub highlight_limit_mb: usize,
    /// Fold chevrons in the gutter.
    pub fold_margin: bool,
    /// Wrap long lines at the pane's width.
    pub word_wrap: bool,
    /// The zoomed-out map beside the text.
    pub document_map: bool,
    /// Editor text size in percent of `font_size` (View > Zoom).
    pub zoom: u32,
    /// View > Show Symbol: spaces as dots and tabs as arrows.
    pub show_space_tab: bool,
    /// View > Show Symbol: each line's ending (CRLF, LF or CR).
    pub show_eol: bool,
    /// View > Show Symbol: format characters and unusual spaces as labelled
    /// boxes (control characters always are).
    pub show_non_printing: bool,
    /// View > Show Symbol: dotted lines at each indent level.
    pub show_indent_guides: bool,
    /// View > Show Symbol: a mark where a wrapped line continues.
    pub show_wrap_symbol: bool,
    /// A vertical line after this column, if any.
    pub edge_column: Option<usize>,
    /// Files the log highlight rules apply to: globs separated by `;`.
    pub log_files: String,
    /// Log highlight rules (PLAN_v0.3.0 P6).
    pub highlight_rules: Vec<crate::log_rules::HighlightRule>,
    /// Files larger than this (in MB) open read-only in the viewer for
    /// large files.
    pub viewer_threshold_mb: u32,
    /// The sidebar with the open folder's files.
    pub show_sidebar: bool,
    pub sidebar_width: f32,
    /// View > Always on Top.
    pub always_on_top: bool,
    pub recent_folders: Vec<PathBuf>,
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
            undo_history: true,
            highlight_limit_mb: 10,
            fold_margin: true,
            word_wrap: false,
            document_map: true,
            zoom: 100,
            show_space_tab: false,
            show_eol: false,
            show_non_printing: false,
            show_indent_guides: false,
            show_wrap_symbol: false,
            edge_column: None,
            log_files: "*.log; *.log.*".to_owned(),
            highlight_rules: crate::log_rules::default_rules(),
            viewer_threshold_mb: 256,
            show_sidebar: true,
            sidebar_width: 240.0,
            always_on_top: false,
            recent_folders: Vec::new(),
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
        self.zoom = self
            .zoom
            .clamp(ZOOM_LEVELS[0], ZOOM_LEVELS[ZOOM_LEVELS.len() - 1]);
        self.edge_column = self.edge_column.map(|c| c.clamp(1, 1000));
        // Up to what the editor can load.
        self.viewer_threshold_mb = self.viewer_threshold_mb.clamp(1, 1024);
    }

    /// The editor font size after zooming.
    pub fn editor_font_size(&self) -> f32 {
        self.font_size * self.zoom as f32 / 100.0
    }

    /// Whether Show All Characters is on: spaces and tabs, line endings and
    /// non-printing characters are all shown.
    pub fn show_all_characters(&self) -> bool {
        self.show_space_tab && self.show_eol && self.show_non_printing
    }

    /// How editors are drawn: the zoomed font size and the View menu's
    /// symbols.
    pub fn look(&self) -> crate::editor_view::Look {
        crate::editor_view::Look {
            font_size: self.editor_font_size(),
            symbols: crate::editor_view::Symbols {
                space_tab: self.show_space_tab,
                eol: self.show_eol,
                non_printing: self.show_non_printing,
                indent_guides: self.show_indent_guides,
                wrap: self.show_wrap_symbol,
            },
            edge_column: self.edge_column,
        }
    }

    /// Turns Show All Characters on or off.
    pub fn set_show_all_characters(&mut self, on: bool) {
        self.show_space_tab = on;
        self.show_eol = on;
        self.show_non_printing = on;
    }

    /// The Preferences window body.
    /// `log_errors`: the highlight rules whose pattern does not compile,
    /// by index.
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        fonts: &[(String, PathBuf)],
        themes: &[&str],
        log_errors: &[(usize, String)],
    ) {
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

                ui.label("Undo history");
                ui.checkbox(
                    &mut self.undo_history,
                    "Keep it after closing a file or ochrepad",
                )
                .on_hover_text(
                    "Up to 2 MB of the newest changes per file, restored when the file is \
                     opened again unchanged",
                );
                ui.end_row();

                ui.label("Highlight files up to");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut self.highlight_limit_mb).range(1..=4096));
                    ui.label("MB");
                });
                ui.end_row();

                ui.label("Open read-only above");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut self.viewer_threshold_mb).range(1..=1024));
                    ui.label("MB (in the viewer for large files)");
                });
                ui.end_row();

                ui.label("Vertical edge");
                ui.horizontal(|ui| {
                    let mut on = self.edge_column.is_some();
                    if ui.checkbox(&mut on, "after column").changed() {
                        self.edge_column = on.then_some(80);
                    }
                    if let Some(col) = &mut self.edge_column {
                        ui.add(egui::DragValue::new(col).range(1..=1000));
                    }
                });
                ui.end_row();
            });
        ui.add_space(8.0);
        egui::CollapsingHeader::new("Log highlighting")
            .id_salt("pref-log-rules")
            .show(ui, |ui| self.log_rules_ui(ui, log_errors));
    }

    /// The highlight rules' editor: the files they apply to, then one row
    /// per rule.
    fn log_rules_ui(&mut self, ui: &mut Ui, errors: &[(usize, String)]) {
        use crate::log_rules::{HighlightRule, RuleStyle, default_rules};
        ui.horizontal(|ui| {
            ui.label("For files named");
            ui.add(
                egui::TextEdit::singleline(&mut self.log_files)
                    .desired_width(200.0)
                    .hint_text("*.log; *.log.*"),
            )
            .on_hover_text("Patterns with * and ?, separated by ;");
        });
        let mut remove = None;
        egui::Grid::new("pref-log-rule-rows")
            .num_columns(6)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                for (i, r) in self.highlight_rules.iter_mut().enumerate() {
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut r.pattern)
                            .desired_width(220.0)
                            .hint_text("text or pattern"),
                    );
                    if let Some((_, e)) = errors.iter().find(|(j, _)| *j == i) {
                        field.on_hover_text(e);
                    }
                    ui.checkbox(&mut r.regex, "Regex");
                    ui.checkbox(&mut r.match_case, "Match case");
                    ui.color_edit_button_srgb(&mut r.color);
                    egui::ComboBox::from_id_salt(("pref-log-rule-style", i))
                        .selected_text(r.style.name())
                        .show_ui(ui, |ui| {
                            for s in RuleStyle::ALL {
                                ui.selectable_value(&mut r.style, s, s.name());
                            }
                        });
                    if ui
                        .small_button("×")
                        .on_hover_text("Remove this rule")
                        .clicked()
                    {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        if let Some(i) = remove {
            self.highlight_rules.remove(i);
        }
        for (i, e) in errors {
            ui.colored_label(ui.visuals().error_fg_color, format!("Rule {}: {e}", i + 1));
        }
        ui.horizontal(|ui| {
            if ui.button("Add Rule").clicked() {
                self.highlight_rules.push(HighlightRule::default());
            }
            if ui.button("Restore Defaults").clicked() {
                self.highlight_rules = default_rules();
                self.log_files = AppSettings::default().log_files;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_steps_through_the_levels_and_stops_at_the_ends() {
        assert_eq!(zoom_step(100, 1), 110);
        assert_eq!(zoom_step(100, -1), 90);
        assert_eq!(zoom_step(100, 3), 150);
        assert_eq!(zoom_step(400, 1), 400);
        assert_eq!(zoom_step(10, -5), 10);
        // A hand-edited value between levels.
        assert_eq!(zoom_step(105, 1), 110);
        assert_eq!(zoom_step(105, -1), 90);
        assert_eq!(zoom_step(5, 1), 25);
    }
}
