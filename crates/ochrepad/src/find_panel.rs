//! The find/replace panel, docked at the bottom of the window. Its searches
//! go through recordable `Action`s; counting and highlighting use the core
//! `Searcher` directly.

use egui::text::{CCursor, CCursorRange};
use egui::{Id, Key, RichText, TextEdit, Ui};
use ochrepad_core::{Action, ActionError, Editor, SearchOptions, SearchQuery, Searcher, ViewId};

/// What the panel asks the app to do after drawing.
#[derive(Clone, Debug, PartialEq)]
pub enum FindRequest {
    /// Close the panel and give the editor the keyboard.
    Close,
    /// The editor should scroll to its caret.
    Reveal,
    /// List every match in the Search Results panel: in the current
    /// document, or in every open one.
    FindAll { all_documents: bool },
    /// Replace every match in every open document (after confirming).
    ReplaceAllInAll,
    /// Find in Files with the folder and filters in the panel.
    FindInFiles,
    /// Find Next/Previous in a viewer (see [`FindTarget::Viewer`]).
    Next { backward: bool },
    /// Count the matches in a viewer.
    Count,
}

/// What the panel searches.
pub enum FindTarget<'a> {
    Editor(&'a mut Editor, ViewId),
    /// A read-only viewer of a large file: Find Next/Previous and Count
    /// are requests the app runs in the background, and there is nothing
    /// to select or replace.
    Viewer,
}

pub struct FindPanel {
    pub open: bool,
    pub show_replace: bool,
    /// The Find in Files row: folder, filters and options.
    pub show_files: bool,
    pub folder: String,
    pub filters: String,
    pub subfolders: bool,
    pub include_ignored: bool,
    pub pattern: String,
    pub replacement: String,
    pub options: SearchOptions,
    message: Option<(String, bool)>,
    focus_field: bool,
    /// The compiled query, rebuilt when the query changes.
    compiled: Option<(SearchQuery, Result<Searcher, String>)>,
}

impl Default for FindPanel {
    fn default() -> Self {
        Self {
            open: false,
            show_replace: false,
            show_files: false,
            folder: String::new(),
            filters: String::new(),
            subfolders: true,
            include_ignored: false,
            pattern: String::new(),
            replacement: String::new(),
            // Notepad++ wraps by default.
            options: SearchOptions {
                wrap: true,
                ..SearchOptions::default()
            },
            message: None,
            focus_field: false,
            compiled: None,
        }
    }
}

fn field_id() -> Id {
    Id::new("ochrepad-find-field")
}

impl FindPanel {
    pub fn query(&self) -> SearchQuery {
        SearchQuery::new(self.pattern.clone(), self.options.clone())
    }

    /// The compiled searcher for the current query, or the pattern error.
    pub fn searcher(&mut self) -> Option<Result<&Searcher, &str>> {
        if self.pattern.is_empty() {
            return None;
        }
        let query = self.query();
        if self.compiled.as_ref().is_none_or(|(q, _)| *q != query) {
            let compiled = Searcher::new(&query).map_err(|e| e.0);
            self.compiled = Some((query, compiled));
        }
        self.compiled
            .as_ref()
            .map(|(_, r)| r.as_ref().map_err(String::as_str))
    }

    /// The searcher for highlighting matches, if the panel is open and the
    /// pattern is valid.
    pub fn highlighter(&mut self) -> Option<&Searcher> {
        if !self.open {
            return None;
        }
        self.searcher().and_then(Result::ok)
    }

    /// Puts the keyboard in the find field on the next frame.
    pub fn focus_field(&mut self) {
        self.focus_field = true;
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_ref().map(|(m, _)| m.as_str())
    }

    pub fn clear_message(&mut self) {
        self.message = None;
    }

    /// Shows `text` under the fields (an error in red).
    pub fn set_message(&mut self, text: impl Into<String>, is_error: bool) {
        self.message = Some((text.into(), is_error));
    }

    /// Opens the panel (Ctrl+F / Ctrl+H), taking a one-line selection as
    /// the pattern.
    pub fn open(&mut self, editor: &Editor, view: ViewId, replace: bool) {
        self.open = true;
        self.show_replace = replace;
        self.focus_field = true;
        self.message = None;
        if let Some(v) = editor.view(view)
            && let Some(b) = editor.buffer(v.buffer())
        {
            let sel = v.selections.primary();
            if !sel.is_empty() {
                let text = b.doc().slice(sel.range());
                if !text.contains('\n') {
                    self.pattern = text;
                }
            }
        }
    }

    /// Shows the Find in Files row, searching `folder` if one is given (or
    /// `fallback` if the folder field is empty).
    pub fn open_files(
        &mut self,
        folder: Option<&std::path::Path>,
        fallback: Option<&std::path::Path>,
    ) {
        self.open = true;
        self.show_files = true;
        self.focus_field = true;
        self.message = None;
        if let Some(f) = folder.or(fallback.filter(|_| self.folder.trim().is_empty())) {
            self.folder = f.display().to_string();
        }
    }

    /// Find Next / Find Previous through the recordable action.
    pub fn find(&mut self, editor: &mut Editor, view: ViewId, backward: bool) {
        let before = editor.view(view).map(|v| v.selections.primary().start());
        let result = editor.apply(
            view,
            Action::Find {
                query: self.query(),
                backward,
            },
        );
        let after = editor.view(view).map(|v| v.selections.primary().start());
        self.message = match result {
            Ok(()) => {
                let wrapped = match (before, after) {
                    (Some(b), Some(a)) if !backward => a < b,
                    (Some(b), Some(a)) => a > b,
                    _ => false,
                };
                wrapped.then(|| {
                    let msg = if backward {
                        "Wrapped to the end"
                    } else {
                        "Wrapped to the start"
                    };
                    (msg.to_owned(), false)
                })
            }
            Err(e) => Some(error_text(&e)),
        };
    }

    fn count(&mut self, editor: &Editor, view: ViewId) -> Option<usize> {
        let v = editor.view(view)?;
        let doc = editor.buffer(v.buffer())?.doc();
        let selections = v.selections.clone();
        match self.searcher()? {
            Ok(s) => Some(s.find_in(doc, &selections).len()),
            Err(_) => None,
        }
    }

    fn run(
        &mut self,
        editor: &mut Editor,
        view: ViewId,
        action: Action,
    ) -> Result<(), ActionError> {
        let r = editor.apply(view, action);
        if let Err(e) = &r {
            self.message = Some(error_text(e));
        }
        r
    }

    pub fn ui(&mut self, ui: &mut Ui, mut target: FindTarget<'_>) -> Vec<FindRequest> {
        let mut requests = Vec::new();
        let in_viewer = matches!(target, FindTarget::Viewer);
        let field_width = (ui.available_width() * 0.4).clamp(120.0, 360.0);
        ui.horizontal_wrapped(|ui| {
            ui.label("Find:");
            let mut output = TextEdit::singleline(&mut self.pattern)
                .id(field_id())
                .desired_width(field_width)
                .hint_text("text or regex")
                .show(ui);
            // Ctrl+F / Ctrl+H select the whole pattern, as in Notepad++, so
            // typing replaces it whether or not the panel was already open.
            if std::mem::take(&mut self.focus_field) {
                output.response.request_focus();
                let end = CCursor::new(self.pattern.chars().count());
                output
                    .state
                    .cursor
                    .set_char_range(Some(CCursorRange::two(CCursor::new(0), end)));
                output.state.store(ui.ctx(), field_id());
            }
            let field = output.response;
            if field.changed() {
                self.message = None;
            }
            // Enter: next; Shift+Enter: previous; Escape: close.
            if field.lost_focus() {
                let (enter, shift, esc) = ui.input(|i| {
                    (
                        i.key_pressed(Key::Enter),
                        i.modifiers.shift,
                        i.key_pressed(Key::Escape),
                    )
                });
                if enter {
                    match &mut target {
                        FindTarget::Editor(editor, view) => {
                            self.find(editor, *view, shift);
                            requests.push(FindRequest::Reveal);
                        }
                        FindTarget::Viewer => requests.push(FindRequest::Next { backward: shift }),
                    }
                    field.request_focus();
                } else if esc {
                    requests.push(FindRequest::Close);
                }
            }
            for (label, backward) in [("Find Previous", true), ("Find Next", false)] {
                if ui.button(label).clicked() {
                    match &mut target {
                        FindTarget::Editor(editor, view) => {
                            self.find(editor, *view, backward);
                            requests.push(FindRequest::Reveal);
                        }
                        FindTarget::Viewer => requests.push(FindRequest::Next { backward }),
                    }
                }
            }
            if ui.button("Count").clicked() {
                match &mut target {
                    FindTarget::Editor(editor, view) => {
                        self.message = self
                            .count(editor, *view)
                            .map(|n| (format!("{n} matches"), false));
                    }
                    FindTarget::Viewer => requests.push(FindRequest::Count),
                }
            }
            if let FindTarget::Editor(editor, view) = &mut target
                && ui.button("Select All").clicked()
            {
                let n = self.count(editor, *view);
                if self
                    .run(editor, *view, Action::SelectAllMatches(self.query()))
                    .is_ok()
                {
                    self.message = n.map(|n| (format!("{n} matches selected"), false));
                    requests.push(FindRequest::Reveal);
                }
            }
            let valid = matches!(self.searcher(), Some(Ok(_)));
            ui.add_enabled_ui(valid, |ui| {
                ui.menu_button("Find All", |ui| {
                    if ui.button("In Current Document").clicked() {
                        requests.push(FindRequest::FindAll {
                            all_documents: false,
                        });
                    }
                    if ui
                        .button("In All Open Documents")
                        .on_hover_text("\"In selection\" does not apply")
                        .clicked()
                    {
                        requests.push(FindRequest::FindAll {
                            all_documents: true,
                        });
                    }
                })
                .response
                .on_hover_text("List every match in the Search Results panel");
            });
            ui.separator();
            // The extra rows, as toggles.
            if !in_viewer
                && ui
                    .selectable_label(self.show_replace, "Replace…")
                    .on_hover_text("Ctrl+H")
                    .clicked()
            {
                self.show_replace = !self.show_replace;
            }
            if ui
                .selectable_label(self.show_files, "In Files…")
                .on_hover_text("Find in Files (Ctrl+Shift+F)")
                .clicked()
            {
                self.show_files = !self.show_files;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(egui::Button::new(RichText::new("×").size(18.0)).frame(false))
                    .on_hover_text("Close (Esc)")
                    .clicked()
                {
                    requests.push(FindRequest::Close);
                }
            });
        });
        if self.show_files {
            ui.horizontal_wrapped(|ui| {
                ui.label("Folder:");
                ui.add(
                    TextEdit::singleline(&mut self.folder)
                        .id(Id::new("ochrepad-files-folder"))
                        .desired_width(field_width)
                        .hint_text("folder to search"),
                );
                if ui.button("Browse…").clicked()
                    && let Some(dir) = rfd::FileDialog::new().pick_folder()
                {
                    self.folder = dir.display().to_string();
                }
                ui.label("Filters:");
                ui.add(
                    TextEdit::singleline(&mut self.filters)
                        .id(Id::new("ochrepad-files-filters"))
                        .desired_width(120.0)
                        .hint_text("*.rs; *.toml"),
                );
                ui.checkbox(&mut self.subfolders, "Subfolders");
                ui.checkbox(&mut self.include_ignored, "Ignored files")
                    .on_hover_text("Also search files .gitignore leaves out");
                let valid =
                    matches!(self.searcher(), Some(Ok(_))) && !self.folder.trim().is_empty();
                if ui
                    .add_enabled(valid, egui::Button::new("Find in Files"))
                    .clicked()
                {
                    requests.push(FindRequest::FindInFiles);
                }
            });
        }
        if let (true, FindTarget::Editor(editor, view)) = (self.show_replace, &mut target) {
            let (editor, view) = (&mut **editor, *view);
            ui.horizontal_wrapped(|ui| {
                ui.label("Replace:");
                ui.add(
                    TextEdit::singleline(&mut self.replacement)
                        .id(Id::new("ochrepad-replace-field"))
                        .desired_width(field_width)
                        .hint_text(if self.options.regex {
                            "$1, ${name}, \\n, \\t"
                        } else {
                            "replacement"
                        }),
                );
                if ui.button("Replace").clicked() {
                    let query = self.query();
                    let replacement = self.replacement.clone();
                    if self
                        .run(editor, view, Action::Replace { query, replacement })
                        .is_ok()
                    {
                        self.message = None;
                        requests.push(FindRequest::Reveal);
                    }
                }
                if ui.button("Replace All").clicked() {
                    let n = self.count(editor, view);
                    let query = self.query();
                    let replacement = self.replacement.clone();
                    if self
                        .run(editor, view, Action::ReplaceAll { query, replacement })
                        .is_ok()
                    {
                        self.message = n.map(|n| (format!("Replaced {n} occurrences"), false));
                    }
                }
                let valid = matches!(self.searcher(), Some(Ok(_)));
                if ui
                    .add_enabled(
                        valid,
                        egui::Button::new("Replace All in All Open Documents"),
                    )
                    .clicked()
                {
                    requests.push(FindRequest::ReplaceAllInAll);
                }
            });
        }
        ui.horizontal_wrapped(|ui| {
            let o = &mut self.options;
            ui.checkbox(&mut o.regex, "Regular expression")
                .on_hover_text(
                    "Rust regex syntax (not Notepad++'s Boost): no lookaround or \
                 backreferences. ^ and $ match at line starts and ends.",
                );
            ui.checkbox(&mut o.match_case, "Match case");
            ui.checkbox(&mut o.whole_word, "Whole word");
            ui.checkbox(&mut o.wrap, "Wrap around");
            ui.checkbox(&mut o.in_selection, "In selection")
                .on_hover_text("Count, Select All and Replace All only look inside the selection");
            ui.separator();
            // Pattern errors show as you type.
            let error = match self.searcher() {
                Some(Err(e)) => Some(e.to_owned()),
                _ => None,
            };
            if let Some(e) = error {
                ui.label(
                    RichText::new(format!("Invalid pattern: {e}"))
                        .color(ui.visuals().error_fg_color),
                );
            } else if let Some((m, is_error)) = &self.message {
                let color = if *is_error {
                    ui.visuals().error_fg_color
                } else {
                    ui.visuals().text_color()
                };
                ui.label(RichText::new(m).color(color));
            }
        });
        requests
    }
}

fn error_text(e: &ActionError) -> (String, bool) {
    match e {
        ActionError::NotFound => ("Not found".to_owned(), true),
        ActionError::InvalidPattern(p) => (format!("Invalid pattern: {p}"), true),
        other => (other.to_string(), true),
    }
}
