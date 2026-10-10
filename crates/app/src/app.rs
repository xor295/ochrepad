//! The application shell: one file, menus, status bar, title, and the
//! save-changes prompt.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{
    Button, CentralPanel, Id, Key, KeyboardShortcut, Modal, Modifiers, Panel, Ui, ViewportCommand,
};
use ochrepad_core::position::byte_to_visual_col;
use ochrepad_core::{Action, Buffer, Editor, FileMeta, LineEnding, ViewId};
use ochrepad_fileio as fileio;

use crate::editor_view::{EditorViewState, LineStyler, editor_view, selected_text};
use crate::highlight::Highlighting;
use crate::keymap::Keymap;

/// What to do once unsaved changes have been dealt with.
#[derive(Clone, Debug, PartialEq)]
enum Pending {
    New,
    Open(Option<PathBuf>),
    Exit,
}

pub struct OchrepadApp {
    pub editor: Editor,
    pub view: ViewId,
    pub view_state: EditorViewState,
    keymap: Keymap,
    message: Option<String>,
    confirm: Option<Pending>,
    close_confirmed: bool,
    title: String,
    pub highlighting: Highlighting,
    /// Whether egui is in dark mode (last frame).
    dark: bool,
}

impl Default for OchrepadApp {
    fn default() -> Self {
        Self::new()
    }
}

const NEW: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
const OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
const SAVE_AS: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S);
const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);

impl OchrepadApp {
    pub fn new() -> Self {
        let mut editor = Editor::new();
        let view = new_buffer_view(&mut editor, "", new_file_meta());
        let mut app = Self {
            editor,
            view,
            view_state: EditorViewState::default(),
            keymap: Keymap::default(),
            message: None,
            confirm: None,
            close_confirmed: false,
            title: String::new(),
            highlighting: Highlighting::new(Arc::new(ochrepad_syntax::Registry::load())),
            dark: true,
        };
        app.attach_highlighting();
        app
    }

    fn attach_highlighting(&mut self) {
        let dark = self.dark;
        let id = self
            .editor
            .view(self.view)
            .expect("app view exists")
            .buffer();
        if let Some(buffer) = self.editor.buffer_mut(id) {
            self.highlighting.attach(buffer, dark);
        }
    }

    /// Changes the buffer's language (from the status-bar picker).
    pub fn set_language(&mut self, language: &str) {
        let dark = self.dark;
        let id = self
            .editor
            .view(self.view)
            .expect("app view exists")
            .buffer();
        if let Some(buffer) = self.editor.buffer_mut(id) {
            self.highlighting.set_language(buffer, language, dark);
        }
    }

    pub fn buffer(&self) -> &Buffer {
        let id = self
            .editor
            .view(self.view)
            .expect("app view exists")
            .buffer();
        self.editor.buffer(id).expect("app buffer exists")
    }

    fn buffer_mut(&mut self) -> &mut Buffer {
        let id = self
            .editor
            .view(self.view)
            .expect("app view exists")
            .buffer();
        self.editor.buffer_mut(id).expect("app buffer exists")
    }

    pub fn file_name(&self) -> String {
        self.buffer()
            .meta
            .path
            .as_deref()
            .and_then(Path::file_name)
            .map_or_else(|| "new 1".to_owned(), |n| n.to_string_lossy().into_owned())
    }

    /// Error or info text currently shown in a dialog.
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Replaces the current buffer with the file at `path`.
    pub fn open_path(&mut self, path: &Path) {
        match fileio::load(path) {
            Ok(decoded) => {
                self.replace_buffer(&decoded.text, decoded.meta);
            }
            Err(e) => self.message = Some(format!("Could not open {}:\n{e}", path.display())),
        }
    }

    fn replace_buffer(&mut self, text: &str, meta: FileMeta) {
        let old = self.editor.view(self.view).map(|v| v.buffer());
        self.view = new_buffer_view(&mut self.editor, text, meta);
        if let Some(old) = old {
            self.editor.remove_buffer(old);
        }
        self.view_state = EditorViewState::default();
        self.attach_highlighting();
    }

    /// Saves to the buffer's path, asking for one if it has none. Returns
    /// whether the buffer was saved.
    pub fn save(&mut self) -> bool {
        if self.buffer().meta.path.is_none() {
            return self.save_as();
        }
        match fileio::save_buffer(self.buffer_mut()) {
            Ok(()) => true,
            Err(e) => {
                self.message = Some(format!("Could not save:\n{e}"));
                false
            }
        }
    }

    pub fn save_as(&mut self) -> bool {
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(self.file_name())
            .save_file()
        else {
            return false;
        };
        self.save_to(&path)
    }

    pub fn save_to(&mut self, path: &Path) -> bool {
        match fileio::save_buffer_as(self.buffer_mut(), path) {
            Ok(()) => {
                // A new name may mean a new language, as in Notepad++.
                self.attach_highlighting();
                true
            }
            Err(e) => {
                self.message = Some(format!("Could not save {}:\n{e}", path.display()));
                false
            }
        }
    }

    fn request(&mut self, pending: Pending) {
        if self.buffer().is_dirty() {
            self.confirm = Some(pending);
        } else {
            self.perform(pending);
        }
    }

    fn perform(&mut self, pending: Pending) {
        match pending {
            Pending::New => self.replace_buffer("", new_file_meta()),
            Pending::Open(Some(path)) => self.open_path(&path),
            Pending::Open(None) => {
                if let Some(path) = rfd::FileDialog::new().pick_file() {
                    self.open_path(&path);
                }
            }
            Pending::Exit => self.close_confirmed = true,
        }
    }

    fn apply(&mut self, action: Action) {
        let _ = self.editor.apply(self.view, action);
        self.view_state.reveal_caret();
        self.view_state.request_focus();
    }

    /// Draws the whole application into `ui`.
    pub fn show(&mut self, ui: &mut Ui) {
        if ui.input_mut(|i| i.consume_shortcut(&NEW)) {
            self.request(Pending::New);
        }
        if ui.input_mut(|i| i.consume_shortcut(&OPEN)) {
            self.request(Pending::Open(None));
        }
        if ui.input_mut(|i| i.consume_shortcut(&SAVE_AS)) {
            self.save_as();
        }
        if ui.input_mut(|i| i.consume_shortcut(&SAVE)) {
            self.save();
        }
        if ui.input(|i| i.viewport().close_requested()) && !self.close_confirmed {
            if self.buffer().is_dirty() {
                ui.ctx().send_viewport_cmd(ViewportCommand::CancelClose);
                self.confirm = Some(Pending::Exit);
            } else {
                self.close_confirmed = true;
            }
        }

        Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        self.dark = ui.visuals().dark_mode;
        let first = self
            .editor
            .view(self.view)
            .map_or(0, |v| v.scroll.first_line);
        let id = self
            .editor
            .view(self.view)
            .expect("app view exists")
            .buffer();
        if let Some(buffer) = self.editor.buffer_mut(id)
            && self
                .highlighting
                .frame(buffer, first..first + 200, self.dark)
        {
            ui.ctx().request_repaint();
        }
        CentralPanel::no_frame().show(ui, |ui| {
            let label = format!("Editor: {}", self.file_name());
            let mut styler = self.highlighting.styler();
            editor_view(
                ui,
                &mut self.editor,
                self.view,
                &mut self.view_state,
                &self.keymap,
                &label,
                styler.as_mut().map(|s| s as &mut dyn LineStyler),
            );
        });

        self.dialogs(ui);
        if self.close_confirmed && self.confirm.is_none() {
            ui.ctx().send_viewport_cmd(ViewportCommand::Close);
        }

        let title = format!(
            "{}{} - ochrepad",
            if self.buffer().is_dirty() { "*" } else { "" },
            self.file_name()
        );
        if title != self.title {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    fn menu_bar(&mut self, ui: &mut Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.add(Button::new("New").shortcut_text("Ctrl+N")).clicked() {
                    self.request(Pending::New);
                }
                if ui
                    .add(Button::new("Open…").shortcut_text("Ctrl+O"))
                    .clicked()
                {
                    self.request(Pending::Open(None));
                }
                if ui
                    .add(Button::new("Save").shortcut_text("Ctrl+S"))
                    .clicked()
                {
                    self.save();
                }
                if ui
                    .add(Button::new("Save As…").shortcut_text("Ctrl+Shift+S"))
                    .clicked()
                {
                    self.save_as();
                }
                ui.separator();
                if ui.button("Exit").clicked() {
                    self.request(Pending::Exit);
                }
            });
            ui.menu_button("View", |ui| self.view_menu(ui));
            ui.menu_button("Edit", |ui| {
                if ui
                    .add(Button::new("Undo").shortcut_text("Ctrl+Z"))
                    .clicked()
                {
                    self.apply(Action::Undo);
                }
                if ui
                    .add(Button::new("Redo").shortcut_text("Ctrl+Y"))
                    .clicked()
                {
                    self.apply(Action::Redo);
                }
                ui.separator();
                if ui.add(Button::new("Cut").shortcut_text("Ctrl+X")).clicked() {
                    let text = selected_text(&self.editor, self.view);
                    if !text.is_empty() {
                        ui.ctx().copy_text(text);
                        self.apply(Action::InsertText(String::new()));
                    }
                }
                if ui
                    .add(Button::new("Copy").shortcut_text("Ctrl+C"))
                    .clicked()
                {
                    let text = selected_text(&self.editor, self.view);
                    if !text.is_empty() {
                        ui.ctx().copy_text(text);
                    }
                    self.view_state.request_focus();
                }
                if ui
                    .add(Button::new("Paste").shortcut_text("Ctrl+V"))
                    .clicked()
                {
                    // The integration answers with an Event::Paste, which the
                    // (refocused) editor handles next frame.
                    ui.ctx().send_viewport_cmd(ViewportCommand::RequestPaste);
                    self.view_state.request_focus();
                }
                ui.separator();
                if ui
                    .add(Button::new("Select All").shortcut_text("Ctrl+A"))
                    .clicked()
                {
                    self.apply(Action::SelectAll);
                }
            });
        });
    }

    /// "Ln 3, Col 7" for the primary caret (columns are visual, 1-based).
    pub fn caret_status(&self) -> String {
        let doc = self.buffer().doc();
        let Some(v) = self.editor.view(self.view) else {
            return String::new();
        };
        let head = v.selections.primary().head;
        let col = byte_to_visual_col(doc, head, self.editor.settings.tab_width);
        format!("Ln {}, Col {}", doc.line_of(head) + 1, col.0 + 1)
    }

    fn view_menu(&mut self, ui: &mut Ui) {
        let dark = ui.visuals().dark_mode;
        if ui.radio(dark, "Dark mode").clicked() {
            ui.ctx().set_theme(egui::ThemePreference::Dark);
        }
        if ui.radio(!dark, "Light mode").clicked() {
            ui.ctx().set_theme(egui::ThemePreference::Light);
        }
        ui.separator();
        ui.menu_button("Syntax theme", |ui| {
            egui::ScrollArea::vertical()
                .max_height(400.0)
                .show(ui, |ui| {
                    let current = self.highlighting.theme_override.clone();
                    if ui
                        .radio(current.is_none(), "Automatic (follows dark/light)")
                        .clicked()
                    {
                        self.highlighting.theme_override = None;
                    }
                    for name in ochrepad_syntax::Registry::theme_names() {
                        if ui.radio(current.as_deref() == Some(name), name).clicked() {
                            self.highlighting.theme_override = Some(name.to_owned());
                        }
                    }
                });
        });
    }

    fn language_picker(&mut self, ui: &mut Ui) {
        let current = self.highlighting.language().to_owned();
        ui.menu_button(current.clone(), |ui| {
            egui::ScrollArea::vertical()
                .max_height(400.0)
                .show(ui, |ui| {
                    let mut chosen = None;
                    for name in self.highlighting.languages() {
                        if ui.selectable_label(*name == current, name).clicked() {
                            chosen = Some(name.clone());
                        }
                    }
                    if let Some(name) = chosen {
                        self.set_language(&name);
                        ui.close();
                    }
                });
        })
        .response
        .on_hover_text("Language");
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        let carets = self
            .editor
            .view(self.view)
            .map_or(1, |v| v.selections.len());
        let meta = self.buffer().meta.clone();
        ui.horizontal(|ui| {
            ui.label(self.caret_status());
            if carets > 1 {
                ui.separator();
                ui.label(format!("{carets} selections"));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let enc = if meta.bom { format!("{} BOM", meta.encoding) } else { meta.encoding.clone() };
                ui.label(enc);
                ui.separator();
                ui.label(match meta.line_ending {
                    LineEnding::CrLf => "Windows (CR LF)",
                    LineEnding::Lf => "Unix (LF)",
                    LineEnding::Cr => "Macintosh (CR)",
                });
                if meta.mixed_line_endings {
                    ui.separator();
                    ui.label("Mixed line endings (saved as shown)")
                        .on_hover_text("The file had more than one kind of line ending. Saving converts all of them to the one shown.");
                }
                ui.separator();
                self.language_picker(ui);
                if let Some(problem) = self.highlighting.problem() {
                    ui.separator();
                    ui.label(problem);
                }
            });
        });
    }

    fn dialogs(&mut self, ui: &mut Ui) {
        if let Some(msg) = self.message.clone() {
            let resp = Modal::new(Id::new("ochrepad-message")).show(ui.ctx(), |ui| {
                ui.label(msg);
                ui.add_space(8.0);
                ui.button("OK").clicked()
            });
            if resp.inner || resp.should_close() {
                self.message = None;
                self.view_state.request_focus();
            }
        }
        if let Some(pending) = self.confirm.clone() {
            let name = self.file_name();
            let resp = Modal::new(Id::new("ochrepad-confirm")).show(ui.ctx(), |ui| {
                ui.label(format!("Save changes to {name}?"));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        Some(1)
                    } else if ui.button("Don't Save").clicked() {
                        Some(2)
                    } else if ui.button("Cancel").clicked() {
                        Some(3)
                    } else {
                        None
                    }
                })
                .inner
            });
            let choice = if resp.should_close() {
                Some(3)
            } else {
                resp.inner
            };
            match choice {
                Some(1) => {
                    self.confirm = None;
                    if self.save() {
                        self.perform(pending);
                    }
                }
                Some(2) => {
                    self.confirm = None;
                    self.perform(pending);
                }
                Some(_) => {
                    self.confirm = None;
                    self.view_state.request_focus();
                }
                None => {}
            }
        }
    }
}

fn new_file_meta() -> FileMeta {
    FileMeta {
        line_ending: fileio::default_line_ending(),
        ..FileMeta::default()
    }
}

fn new_buffer_view(editor: &mut Editor, text: &str, meta: FileMeta) -> ViewId {
    let buffer = editor.add_buffer(text, meta);
    editor.add_view(buffer).expect("buffer was just added")
}
