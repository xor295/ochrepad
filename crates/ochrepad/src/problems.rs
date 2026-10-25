//! View > Problems (PLAN_completion L2): every file's diagnostics from the
//! language servers, files by name and problems worst first, in a panel at
//! the bottom like Search Results. A click opens the file at the problem.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use egui::{FontId, RichText, ScrollArea, Sense, Ui};

use crate::lsp::{FileDiagnostics, Severity};

#[derive(Default)]
pub struct ProblemsPanel {
    pub open: bool,
    /// Errors only.
    errors_only: bool,
}

/// A problem clicked: its file, line and column (from 0, the column in
/// the server's units, close enough for Go to Line).
pub struct GoTo {
    pub path: PathBuf,
    pub line: u32,
    pub column: u32,
}

enum Row<'a> {
    File(&'a Path, usize),
    Problem(&'a Path, &'a crate::lsp::Diagnostic),
}

impl ProblemsPanel {
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        diagnostics: &HashMap<String, FileDiagnostics>,
        root: Option<&Path>,
    ) -> Option<GoTo> {
        let mut files: Vec<(&Path, Vec<&crate::lsp::Diagnostic>)> = diagnostics
            .values()
            .filter_map(|f| {
                let mut items: Vec<_> = f
                    .items
                    .iter()
                    .filter(|d| d.severity != Severity::Hint)
                    .filter(|d| !self.errors_only || d.severity == Severity::Error)
                    .collect();
                items.sort_by_key(|d| (d.severity, d.lsp_range[0]));
                (!items.is_empty()).then_some((f.path.as_deref()?, items))
            })
            .collect();
        files.sort_by(|a, b| a.0.cmp(b.0));
        let count = |s: Severity| {
            diagnostics
                .values()
                .flat_map(|f| &f.items)
                .filter(|d| d.severity == s)
                .count()
        };
        let mut go = None;
        ui.horizontal(|ui| {
            ui.strong("Problems");
            ui.weak(format!(
                "{} errors, {} warnings",
                count(Severity::Error),
                count(Severity::Warning)
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    self.open = false;
                }
                ui.checkbox(&mut self.errors_only, "Errors only");
            });
        });
        ui.separator();
        if files.is_empty() {
            ui.weak(
                "Problems that language servers report in the open folder's files show here \
                 (View > Language Servers).",
            );
            return None;
        }
        let rows: Vec<Row> = files
            .iter()
            .flat_map(|(path, items)| {
                std::iter::once(Row::File(path, items.len()))
                    .chain(items.iter().map(|d| Row::Problem(path, d)))
            })
            .collect();
        let font = FontId::monospace(13.0);
        let row_h = ui.fonts_mut(|f| f.row_height(&font)) + 6.0;
        ScrollArea::vertical()
            .auto_shrink(false)
            .show_rows(ui, row_h, rows.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for row in &rows[range] {
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), row_h),
                        Sense::click(),
                    );
                    let painter = ui.painter_at(rect);
                    match row {
                        Row::File(path, n) => {
                            let shown =
                                root.and_then(|r| path.strip_prefix(r).ok()).unwrap_or(path);
                            painter.text(
                                rect.left_center() + egui::vec2(4.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                format!("{}  ({n})", shown.display()),
                                egui::TextStyle::Body.resolve(ui.style()),
                                ui.visuals().strong_text_color(),
                            );
                        }
                        Row::Problem(path, d) => {
                            if response.hovered() {
                                painter.rect_filled(
                                    rect,
                                    0.0,
                                    ui.visuals().widgets.hovered.weak_bg_fill,
                                );
                            }
                            let color =
                                crate::theme::diagnostic_color(ui.visuals(), d.severity as u8);
                            let (line, col) = d.lsp_range[0];
                            painter.circle_filled(
                                rect.left_center() + egui::vec2(20.0, 0.0),
                                4.0,
                                color,
                            );
                            let source = d
                                .source
                                .as_deref()
                                .map(|s| format!("  ({s})"))
                                .unwrap_or_default();
                            let message = d.message.lines().next().unwrap_or("");
                            painter.text(
                                rect.left_center() + egui::vec2(32.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                format!("{}:{}  {message}{source}", line + 1, col + 1),
                                egui::TextStyle::Body.resolve(ui.style()),
                                ui.visuals().text_color(),
                            );
                            let response = response.on_hover_text(RichText::new(&d.message));
                            if response.clicked() {
                                go = Some(GoTo {
                                    path: path.to_path_buf(),
                                    line,
                                    column: col,
                                });
                            }
                        }
                    }
                }
            });
        go
    }
}
