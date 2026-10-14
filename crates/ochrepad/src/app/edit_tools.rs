//! Notepad++'s editing extras: more Line Operations, Blank Operations, the
//! Column Editor (Alt+C), Copy to Clipboard, the tab context menu's file
//! commands, and View > Full Screen (F11) and Always on Top.

use std::collections::HashMap;
use std::path::Path;

use egui::{Button, Id, Key, KeyboardShortcut, Modal, Modifiers, Ui, ViewportCommand};
use ochrepad_core::{
    Action, BufferId, ColumnInsert, Editor, Leading, NumberFormat, NumberSequence, ViewId,
};

use super::{OchrepadApp, Tab, TabCommand};

const COLUMN_EDITOR: KeyboardShortcut = KeyboardShortcut::new(Modifiers::ALT, Key::C);
const FULL_SCREEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F11);

/// The Column Editor's fields, kept between uses as Notepad++ does.
struct ColumnFields {
    numbers: bool,
    text: String,
    initial: String,
    increase: String,
    repeat: String,
    leading: Leading,
    format: NumberFormat,
}

impl Default for ColumnFields {
    fn default() -> Self {
        Self {
            numbers: false,
            text: String::new(),
            initial: "1".to_owned(),
            increase: "1".to_owned(),
            repeat: "1".to_owned(),
            leading: Leading::None,
            format: NumberFormat::Dec,
        }
    }
}

impl ColumnFields {
    /// What to insert, or why the fields cannot be used.
    fn insert(&self) -> Result<ColumnInsert, &'static str> {
        if !self.numbers {
            return Ok(ColumnInsert::Text(self.text.clone()));
        }
        let initial = self
            .initial
            .trim()
            .parse()
            .map_err(|_| "The initial number must be a whole number.")?;
        let increase = self
            .increase
            .trim()
            .parse()
            .map_err(|_| "Increase by must be a whole number.")?;
        let repeat: usize = self
            .repeat
            .trim()
            .parse()
            .ok()
            .filter(|r| *r >= 1)
            .ok_or("Repeat must be 1 or more.")?;
        Ok(ColumnInsert::Numbers(NumberSequence {
            initial,
            increase,
            repeat,
            leading: self.leading,
            format: self.format,
        }))
    }
}

/// The state of these tools.
#[derive(Default)]
pub(super) struct EditTools {
    column: ColumnFields,
    column_open: bool,
    /// The dialog has just opened: its field takes the keyboard.
    column_focus: bool,
    /// A buffer with unsaved changes waiting for Reload from Disk to be
    /// confirmed.
    confirm_reload: Option<BufferId>,
    /// The window level last asked for (Always on Top).
    applied_on_top: Option<bool>,
}

impl EditTools {
    /// Whether one of these dialogs is showing.
    pub(super) fn dialog_open(&self) -> bool {
        self.column_open || self.confirm_reload.is_some()
    }
}

/// A file command from the tab context menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TabFileCommand {
    CloseLeft,
    CloseRight,
    CloseUnchanged,
    Reveal,
    Reload,
}

/// Copy Full File Path, Copy File Name and Copy Folder Path, for the
/// document at `path` (if it has one) named `name`.
fn copy_path_items(ui: &mut Ui, path: Option<&Path>, name: &str) {
    if ui
        .add_enabled(path.is_some(), Button::new("Copy Full File Path"))
        .clicked()
        && let Some(p) = path
    {
        ui.ctx().copy_text(p.display().to_string());
    }
    if ui.button("Copy File Name").clicked() {
        ui.ctx().copy_text(name.to_owned());
    }
    let folder = path.and_then(Path::parent);
    if ui
        .add_enabled(folder.is_some(), Button::new("Copy Folder Path"))
        .clicked()
        && let Some(f) = folder
    {
        ui.ctx().copy_text(f.display().to_string());
    }
}

/// The tab context menu's items after Close, Close All but This and Clone
/// to Other View.
pub(super) fn tab_menu(
    ui: &mut Ui,
    view: ViewId,
    editor: &Editor,
    untitled: &HashMap<BufferId, String>,
    commands: &mut Vec<TabCommand>,
) {
    for (label, command) in [
        ("Close All to the Left", TabFileCommand::CloseLeft),
        ("Close All to the Right", TabFileCommand::CloseRight),
        ("Close All Unchanged", TabFileCommand::CloseUnchanged),
    ] {
        if ui.button(label).clicked() {
            commands.push(TabCommand::File(command, view));
        }
    }
    ui.separator();
    let Some(buffer) = editor.view(view).map(|v| v.buffer()) else {
        return;
    };
    let path = editor.buffer(buffer).and_then(|b| b.meta.path.clone());
    copy_path_items(
        ui,
        path.as_deref(),
        &super::buffer_name(editor, untitled, buffer),
    );
    ui.separator();
    for (label, command) in [
        ("Reveal in File Manager", TabFileCommand::Reveal),
        ("Reload from Disk", TabFileCommand::Reload),
    ] {
        if ui.add_enabled(path.is_some(), Button::new(label)).clicked() {
            commands.push(TabCommand::File(command, view));
        }
    }
}

impl OchrepadApp {
    /// Alt+C and F11 (called with the app's other fixed shortcuts).
    pub(super) fn edit_tools_shortcuts(&mut self, ui: &mut Ui) {
        // An extra Alt means another shortcut (egui ignores extra Alt and
        // Shift when matching).
        let pressed = |ui: &mut Ui, s: &KeyboardShortcut| {
            ui.input_mut(|i| (s.modifiers.alt || !i.modifiers.alt) && i.consume_shortcut(s))
        };
        if pressed(ui, &COLUMN_EDITOR) {
            self.open_column_editor();
        }
        if pressed(ui, &FULL_SCREEN) {
            toggle_full_screen(ui);
        }
    }

    /// Opens the Column Editor (Alt+C).
    pub fn open_column_editor(&mut self) {
        if self.focused_viewer().is_some() {
            self.note_text(super::READ_ONLY);
            return;
        }
        self.edit_tools.column_open = true;
        self.edit_tools.column_focus = true;
    }

    /// Per frame, after the dock: the dialogs, and the window level.
    pub(super) fn edit_tools_frame(&mut self, ui: &mut Ui) {
        let on_top = self.settings.always_on_top;
        if self.edit_tools.applied_on_top.unwrap_or(false) != on_top {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::WindowLevel(if on_top {
                    egui::WindowLevel::AlwaysOnTop
                } else {
                    egui::WindowLevel::Normal
                }));
        }
        self.edit_tools.applied_on_top = Some(on_top);
        self.column_editor_dialog(ui);
        self.reload_dialog(ui);
    }

    /// Edit > Line Operations: the items Notepad++ (and Scintilla) add to
    /// the duplicate, move and join ones.
    pub(super) fn more_line_operations(&mut self, ui: &mut Ui) {
        for (label, shortcut, action) in [
            ("Delete Line", "Ctrl+Shift+L", Action::DeleteLine),
            ("Transpose Line", "Ctrl+T", Action::TransposeLines),
            (
                "Insert Blank Line Above Current",
                "Ctrl+Alt+Enter",
                Action::InsertLineAbove,
            ),
            (
                "Insert Blank Line Below Current",
                "Ctrl+Alt+Shift+Enter",
                Action::InsertLineBelow,
            ),
            ("Reverse Line Order", "", Action::ReverseLines),
        ] {
            if ui.add(Button::new(label).shortcut_text(shortcut)).clicked() {
                self.apply(action);
            }
        }
    }

    /// Edit > Blank Operations, as in Notepad++.
    pub(super) fn blank_operations_menu(&mut self, ui: &mut Ui) {
        for (label, action) in [
            ("Trim Trailing Space", Action::TrimTrailingWhitespace),
            ("Trim Leading Space", Action::TrimLeadingWhitespace),
            (
                "Trim Leading and Trailing Space",
                Action::TrimLeadingAndTrailingWhitespace,
            ),
            ("EOL to Space", Action::EolToSpace),
        ] {
            if ui.button(label).clicked() {
                self.apply(action);
            }
        }
        ui.separator();
        for (label, action) in [
            ("TAB to Space", Action::TabsToSpaces),
            ("Space to TAB (All)", Action::SpacesToTabs),
            (
                "Space to TAB (Leading)",
                Action::ConvertIndentation { to_spaces: false },
            ),
        ] {
            if ui.button(label).clicked() {
                self.apply(action);
            }
        }
    }

    /// Edit > Copy to Clipboard: the active document's path, name or
    /// folder.
    pub(super) fn copy_to_clipboard_menu(&mut self, ui: &mut Ui) {
        let buffer = self.buffer_id();
        let path = self.buffer().meta.path.clone();
        copy_path_items(ui, path.as_deref(), &self.buffer_name(buffer));
    }

    /// Edit > Column Editor….
    pub(super) fn column_editor_item(&mut self, ui: &mut Ui) {
        if ui
            .add(Button::new("Column Editor…").shortcut_text("Alt+C"))
            .clicked()
        {
            self.open_column_editor();
        }
    }

    /// View > Full Screen and Always on Top.
    pub(super) fn window_view_items(&mut self, ui: &mut Ui) {
        let full = ui.input(|i| i.viewport().fullscreen.unwrap_or(false));
        if ui
            .add(
                Button::new("Full Screen")
                    .selected(full)
                    .shortcut_text("F11"),
            )
            .clicked()
        {
            toggle_full_screen(ui);
        }
        ui.checkbox(&mut self.settings.always_on_top, "Always on Top");
    }

    /// Runs a file command from the tab context menu.
    pub(super) fn tab_file_command(&mut self, command: TabFileCommand, view: ViewId) {
        match command {
            TabFileCommand::CloseLeft | TabFileCommand::CloseRight => {
                let Some(path) = self.dock.find_tab(&Tab::Editor(view)) else {
                    return;
                };
                let node = egui_dock::NodePath {
                    surface: path.surface,
                    node: path.node,
                };
                let Ok(leaf) = self.dock.leaf(node) else {
                    return;
                };
                let at = path.tab.0;
                let left = command == TabFileCommand::CloseLeft;
                let tabs: Vec<Tab> = leaf
                    .tabs
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| if left { *i < at } else { *i > at })
                    .map(|(_, t)| *t)
                    .collect();
                for tab in tabs {
                    match tab {
                        Tab::Editor(v) => self.close_queue.push_back(v),
                        Tab::Viewer(id) => self.close_viewer(id),
                        Tab::Preview(_) | Tab::Compare(_) => {}
                    }
                }
            }
            TabFileCommand::CloseUnchanged => {
                let clean: Vec<ViewId> = self
                    .tabs()
                    .into_iter()
                    .filter(|v| {
                        self.editor
                            .view(*v)
                            .and_then(|v| self.editor.buffer(v.buffer()))
                            .is_some_and(|b| !b.is_dirty())
                    })
                    .collect();
                self.close_queue.extend(clean);
            }
            TabFileCommand::Reveal => {
                let path = self
                    .editor
                    .view(view)
                    .and_then(|v| self.editor.buffer(v.buffer()))
                    .and_then(|b| b.meta.path.clone());
                if let Some(path) = path
                    && let Err(e) = super::workspace::reveal_in_file_manager(&path)
                {
                    self.message = Some(format!("Could not open the file manager:\n{e}"));
                }
            }
            TabFileCommand::Reload => {
                let Some(buffer) = self.editor.view(view).map(|v| v.buffer()) else {
                    return;
                };
                if self.editor.buffer(buffer).is_some_and(|b| b.is_dirty()) {
                    self.edit_tools.confirm_reload = Some(buffer);
                } else {
                    self.reload_from_disk(buffer);
                }
            }
        }
    }

    fn reload_dialog(&mut self, ui: &mut Ui) {
        let Some(buffer) = self.edit_tools.confirm_reload else {
            return;
        };
        if self.editor.buffer(buffer).is_none() {
            self.edit_tools.confirm_reload = None;
            return;
        }
        let name = self.buffer_name(buffer);
        let resp = Modal::new(Id::new("ochrepad-confirm-reload")).show(ui.ctx(), |ui| {
            ui.label(format!(
                "Reload {name} from disk?\nYour unsaved changes to it will be lost \
                 (Undo brings them back)."
            ));
            ui.add_space(8.0);
            ui.horizontal(|ui| (ui.button("Reload").clicked(), ui.button("Cancel").clicked()))
                .inner
        });
        let (reload, cancel) = resp.inner;
        if reload {
            self.edit_tools.confirm_reload = None;
            self.reload_from_disk(buffer);
        } else if cancel || resp.should_close() {
            self.edit_tools.confirm_reload = None;
        }
    }

    fn column_editor_dialog(&mut self, ui: &mut Ui) {
        if !self.edit_tools.column_open {
            return;
        }
        let opened = std::mem::take(&mut self.edit_tools.column_focus);
        let f = &mut self.edit_tools.column;
        let resp = Modal::new(Id::new("ochrepad-column-editor")).show(ui.ctx(), |ui| {
            ui.strong("Column Editor");
            ui.add_space(4.0);
            ui.radio_value(&mut f.numbers, false, "Text to Insert");
            let text = ui.add_enabled(
                !f.numbers,
                egui::TextEdit::singleline(&mut f.text)
                    .id(Id::new("ochrepad-column-text"))
                    .desired_width(240.0),
            );
            ui.add_space(4.0);
            let switched = ui
                .radio_value(&mut f.numbers, true, "Number to Insert")
                .clicked();
            let mut first_number = None;
            ui.add_enabled_ui(f.numbers, |ui| {
                egui::Grid::new("ochrepad-column-numbers")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        let field = |ui: &mut Ui, label: &str, value: &mut String, id: &str| {
                            ui.label(label);
                            let r = ui.add(
                                egui::TextEdit::singleline(value)
                                    .id(Id::new(id))
                                    .desired_width(100.0),
                            );
                            ui.end_row();
                            r
                        };
                        first_number = Some(field(
                            ui,
                            "Initial number",
                            &mut f.initial,
                            "ochrepad-column-initial",
                        ));
                        field(
                            ui,
                            "Increase by",
                            &mut f.increase,
                            "ochrepad-column-increase",
                        );
                        field(ui, "Repeat", &mut f.repeat, "ochrepad-column-repeat");
                        ui.label("Leading");
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut f.leading, Leading::None, "None");
                            ui.radio_value(&mut f.leading, Leading::Zeros, "Zeros");
                            ui.radio_value(&mut f.leading, Leading::Spaces, "Spaces");
                        });
                        ui.end_row();
                        ui.label("Format");
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut f.format, NumberFormat::Dec, "Dec");
                            ui.radio_value(&mut f.format, NumberFormat::Hex, "Hex");
                            ui.radio_value(&mut f.format, NumberFormat::Oct, "Oct");
                            ui.radio_value(&mut f.format, NumberFormat::Bin, "Bin");
                        });
                        ui.end_row();
                    });
            });
            // Keep the keyboard in the dialog: on the field of the chosen
            // kind whenever nothing else has it.
            let focus = if f.numbers {
                first_number.unwrap_or(text)
            } else {
                text
            };
            if opened || switched || ui.memory(|m| m.focused().is_none()) {
                focus.request_focus();
            }
            let enter = ui.input(|i| i.key_pressed(Key::Enter));
            let problem = f.insert().err();
            if let Some(p) = problem {
                ui.colored_label(ui.visuals().error_fg_color, p);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let ok = ui
                    .add_enabled(problem.is_none(), Button::new("OK"))
                    .clicked();
                if (ok || enter) && problem.is_none() {
                    Some(true)
                } else if ui.button("Cancel").clicked() {
                    Some(false)
                } else {
                    None
                }
            })
            .inner
        });
        let choice = if resp.should_close() {
            Some(false)
        } else {
            resp.inner
        };
        match choice {
            Some(true) => {
                self.edit_tools.column_open = false;
                if let Ok(insert) = self.edit_tools.column.insert() {
                    self.apply(Action::ColumnInsert(insert));
                }
            }
            Some(false) => {
                self.edit_tools.column_open = false;
                self.view_state_mut().request_focus();
            }
            None => {}
        }
    }
}

/// Asks the window to leave or enter full screen.
fn toggle_full_screen(ui: &Ui) {
    let full = ui.input(|i| i.viewport().fullscreen.unwrap_or(false));
    ui.ctx()
        .send_viewport_cmd(ViewportCommand::Fullscreen(!full));
}
