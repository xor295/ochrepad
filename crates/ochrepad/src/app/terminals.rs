//! Terminal tabs in the app (PLAN_terminal): opening them below the
//! editors, Ctrl+` and Ctrl+Shift+`, the keys a focused terminal leaves to
//! the app, the status bar and closing.

use std::path::PathBuf;

use egui::{Key, KeyboardShortcut, Modifiers, Ui};
use egui_dock::{NodeIndex, NodePath, SurfaceIndex};

use super::{OchrepadApp, Tab};
use crate::terminal::{ShellCommand, TerminalId};
use crate::terminal_view::terminal_widget_id;

/// Shows the terminal (opening one when there is none), or goes back to
/// the editor from it.
pub(super) const TOGGLE_TERMINAL: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND, Key::Backtick);
pub(super) const NEW_TERMINAL: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Backtick);

/// The share of the window's height the editors keep when the first
/// terminal opens below them.
const EDITOR_SHARE: f32 = 0.7;

impl OchrepadApp {
    /// The folder a new terminal starts in: the open folder, else the
    /// current document's, else the home folder.
    fn terminal_folder(&self) -> PathBuf {
        if let Some(tree) = &self.tree {
            return tree.root().to_path_buf();
        }
        if let Some(dir) = self.buffer().meta.path.as_deref().and_then(|p| p.parent()) {
            return dir.to_path_buf();
        }
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default()
    }

    /// Opens a new terminal: next to the terminal used last, or, for the
    /// first one, in a new pane below the editors.
    pub fn new_terminal(&mut self) -> TerminalId {
        let cwd = self.terminal_folder();
        self.new_terminal_in(cwd)
    }

    /// Opens a new terminal that starts in `cwd`.
    pub fn new_terminal_in(&mut self, cwd: PathBuf) -> TerminalId {
        let shell = ShellCommand::new(&self.settings.terminal_shell, &self.settings.terminal_args);
        let id = self
            .terminals
            .add(shell, cwd, self.settings.terminal_scrollback);
        let beside = self
            .terminals
            .last
            .and_then(|t| self.dock.find_tab(&Tab::Terminal(t)))
            .or_else(|| {
                self.dock
                    .iter_all_tabs()
                    .find(|(_, t)| matches!(t, Tab::Terminal(_)))
                    .map(|(path, _)| path)
            });
        match beside {
            Some(path) => {
                let node = NodePath {
                    surface: path.surface,
                    node: path.node,
                };
                if let Ok(leaf) = self.dock.leaf_mut(node) {
                    leaf.append_tab(Tab::Terminal(id));
                }
            }
            None => {
                self.dock.main_surface_mut().split_below(
                    NodeIndex::root(),
                    EDITOR_SHARE,
                    vec![Tab::Terminal(id)],
                );
            }
        }
        self.activate_terminal(id);
        id
    }

    /// Makes terminal `id` the focused tab and gives it the keyboard.
    pub fn activate_terminal(&mut self, id: TerminalId) {
        if let Some(path) = self.dock.find_tab(&Tab::Terminal(id)) {
            let _ = self.dock.set_active_tab(path);
            self.dock.set_focused_node_and_surface(NodePath {
                surface: path.surface,
                node: path.node,
            });
        }
        if let Some(t) = self.terminals.get_mut(id) {
            t.view.request_focus();
        }
        self.terminals.last = Some(id);
    }

    /// The terminal in the focused pane, if a terminal is what it shows.
    pub fn focused_terminal(&self) -> Option<TerminalId> {
        match super::focused_tab(&self.dock) {
            Some(Tab::Terminal(id)) => Some(id),
            _ => None,
        }
    }

    /// Whether a terminal has the keyboard (it then gets nearly every key).
    pub(super) fn terminal_has_keyboard(&self, ui: &Ui) -> bool {
        self.focused_terminal()
            .is_some_and(|id| ui.memory(|m| m.has_focus(terminal_widget_id(id))))
    }

    /// Ctrl+`: from a terminal back to the editor; otherwise to the terminal
    /// used last, opening one when there is none.
    pub fn toggle_terminal(&mut self) {
        if self.focused_terminal().is_some() {
            self.activate(self.active);
            return;
        }
        let shown = self
            .terminals
            .last
            .filter(|t| self.dock.find_tab(&Tab::Terminal(*t)).is_some())
            .or_else(|| self.terminals.iter().map(|t| t.id).next());
        match shown {
            Some(id) => self.activate_terminal(id),
            None => {
                self.new_terminal();
            }
        }
    }

    /// Closes terminal `id`, ending its shell.
    pub fn close_terminal(&mut self, id: TerminalId) {
        if let Some(path) = self.dock.find_tab(&Tab::Terminal(id)) {
            self.dock.remove_tab(path);
        }
        self.terminals.remove(id);
        // Back to an editor, rather than to an empty pane's focus.
        if matches!(
            self.dock.find_active_focused(),
            None | Some((_, Tab::Terminal(_)))
        ) && self.focused_terminal().is_none()
        {
            self.activate(self.active);
        }
    }

    /// The keys the app keeps while a terminal has the keyboard: the
    /// terminal shortcuts, switching tabs, and full screen. Everything else
    /// goes to the shell (Ctrl+W, Ctrl+N, Ctrl+R…).
    pub(super) fn terminal_shortcuts(&mut self, ui: &mut Ui) {
        let pressed = |ui: &mut Ui, s: &KeyboardShortcut| {
            ui.input_mut(|i| (s.modifiers.alt || !i.modifiers.alt) && i.consume_shortcut(s))
        };
        // Before Ctrl+`, which an extra Shift would match too.
        if pressed(ui, &NEW_TERMINAL) {
            self.new_terminal();
        }
        if pressed(ui, &TOGGLE_TERMINAL) {
            self.toggle_terminal();
        }
        if pressed(ui, &super::PREV_TAB) {
            self.cycle_tab(false);
        }
        if pressed(ui, &super::NEXT_TAB) {
            self.cycle_tab(true);
        }
        if pressed(ui, &super::PREV_TAB_PAGE) {
            self.cycle_tab(false);
        }
        if pressed(ui, &super::NEXT_TAB_PAGE) {
            self.cycle_tab(true);
        }
        if pressed(ui, &super::edit_tools::FULL_SCREEN) {
            super::edit_tools::toggle_full_screen(ui);
        }
    }

    /// The status bar while a terminal has the focus: its shell, folder and
    /// size.
    pub(super) fn terminal_status_bar(&mut self, ui: &mut Ui, id: TerminalId) {
        let Some(t) = self.terminals.get(id) else {
            return;
        };
        let size = t.size();
        let state = match &t.ended {
            None if t.started() => "Running".to_owned(),
            None => "Starting".to_owned(),
            Some(crate::terminal::Ended::Exited(Some(code))) => format!("Ended (code {code})"),
            Some(crate::terminal::Ended::Exited(None)) => "Ended".to_owned(),
            Some(crate::terminal::Ended::Failed(_)) => "Could not start".to_owned(),
        };
        let shell = t.shell.program.clone();
        let cwd = t.cwd.display().to_string();
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(format!("{} × {}", size.cols, size.rows))
                .on_hover_text("Columns and rows");
            ui.separator();
            ui.label(state);
            ui.separator();
            ui.label(shell);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(format!("Terminal in {cwd}")).truncate());
            });
        });
    }

    /// The main surface's dock, for tests: whether a terminal sits in a
    /// pane below another.
    pub fn terminal_is_below_editors(&self, id: TerminalId) -> bool {
        let Some(path) = self.dock.find_tab(&Tab::Terminal(id)) else {
            return false;
        };
        path.surface == SurfaceIndex::main() && path.node != NodeIndex::root()
    }
}
