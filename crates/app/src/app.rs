//! The application shell: a workspace of buffers and views in a dock of tabs
//! and split panes, with menus, status bar, title and save prompts.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{
    Button, CentralPanel, Id, Key, KeyboardShortcut, Modal, Modifiers, Panel, Ui, ViewportCommand,
    WidgetText,
};
use egui_dock::{
    AllowedSplits, DockArea, DockState, Node, NodePath, Split, Style, TabIndex, TabViewer,
};
use ochrepad_core::position::byte_to_visual_col;
use ochrepad_core::{Action, Buffer, BufferId, Editor, FileMeta, LineEnding, ViewId};
use ochrepad_fileio as fileio;

use egui_dock::tab_viewer::OnCloseResponse;

use crate::editor_view::{
    Decorations, EditorViewState, LineStyler, copy_selection, editor_id, editor_view,
};
use crate::find_panel::{FindPanel, FindRequest};
use crate::highlight::Highlighting;
use crate::keymap::Keymap;

const NEW: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
const OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
const SAVE_AS: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S);
const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
const CLOSE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::W);
const FIND: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::F);
const REPLACE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::H);
const FIND_NEXT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F3);
const FIND_PREV: KeyboardShortcut = KeyboardShortcut::new(Modifiers::SHIFT, Key::F3);
const NEXT_TAB: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Tab);
const PREV_TAB: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Tab);

/// Requests collected while the dock is drawn, handled afterwards.
enum TabCommand {
    Close(ViewId),
    CloseOthers(ViewId),
    CloneToOtherView(ViewId),
}

pub struct OchrepadApp {
    pub editor: Editor,
    pub dock: DockState<ViewId>,
    view_states: HashMap<ViewId, EditorViewState>,
    /// Names of buffers that have never been saved ("new 1", "new 2", …).
    untitled: HashMap<BufferId, String>,
    next_untitled: usize,
    active: ViewId,
    focus_request: Option<ViewId>,
    keymap: Keymap,
    message: Option<String>,
    /// Views waiting to be closed, in order; dirty buffers are asked about.
    close_queue: VecDeque<ViewId>,
    /// Buffers whose unsaved changes the user agreed to discard.
    discard_approved: HashSet<BufferId>,
    /// Dirty buffers still to ask about before exiting.
    exit_queue: VecDeque<BufferId>,
    /// The view whose save prompt is showing.
    confirming: Option<ViewId>,
    exiting: bool,
    close_confirmed: bool,
    close_sent: bool,
    title: String,
    pub highlighting: Highlighting,
    pub find: FindPanel,
    /// Visible line range per buffer in the last frame, for highlighting.
    visible: HashMap<BufferId, Range<usize>>,
    /// Whether egui is in dark mode (last frame).
    dark: bool,
}

impl Default for OchrepadApp {
    fn default() -> Self {
        Self::new()
    }
}

impl OchrepadApp {
    pub fn new() -> Self {
        let mut editor = Editor::new();
        let buffer = editor.add_buffer("", new_file_meta());
        let view = editor.add_view(buffer).expect("buffer was just added");
        let mut app = Self {
            editor,
            dock: DockState::new(vec![view]),
            view_states: HashMap::new(),
            untitled: HashMap::from([(buffer, "new 1".to_owned())]),
            next_untitled: 2,
            active: view,
            focus_request: Some(view),
            keymap: Keymap::default(),
            message: None,
            close_queue: VecDeque::new(),
            discard_approved: HashSet::new(),
            exit_queue: VecDeque::new(),
            confirming: None,
            exiting: false,
            close_confirmed: false,
            close_sent: false,
            title: String::new(),
            highlighting: Highlighting::new(Arc::new(ochrepad_syntax::Registry::load())),
            find: FindPanel::default(),
            visible: HashMap::new(),
            dark: true,
        };
        app.attach_highlighting(buffer);
        app
    }

    // --- Accessors -------------------------------------------------------------

    /// The view with keyboard focus (the active tab of the focused pane).
    pub fn active_view(&self) -> ViewId {
        self.active
    }

    pub fn buffer_id(&self) -> BufferId {
        self.editor
            .view(self.active)
            .expect("active view exists")
            .buffer()
    }

    pub fn buffer(&self) -> &Buffer {
        self.editor
            .buffer(self.buffer_id())
            .expect("active buffer exists")
    }

    pub fn view_state(&self) -> Option<&EditorViewState> {
        self.view_states.get(&self.active)
    }

    /// Scrolls the active view to its primary caret on the next frame.
    pub fn reveal_caret(&mut self) {
        self.view_state_mut().reveal_caret();
    }

    fn view_state_mut(&mut self) -> &mut EditorViewState {
        self.view_states.entry(self.active).or_default()
    }

    /// Display name of a buffer: its file name, or "new N".
    pub fn buffer_name(&self, buffer: BufferId) -> String {
        buffer_name(&self.editor, &self.untitled, buffer)
    }

    pub fn file_name(&self) -> String {
        self.buffer_name(self.buffer_id())
    }

    /// Tab views in dock order.
    pub fn tabs(&self) -> Vec<ViewId> {
        self.dock.iter_all_tabs().map(|(_, v)| *v).collect()
    }

    /// Error or info text currently shown in a dialog.
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// The buffer whose save prompt is showing.
    pub fn confirming(&self) -> Option<BufferId> {
        self.confirming
            .and_then(|v| self.editor.view(v))
            .map(|v| v.buffer())
    }

    // --- Buffers and views -----------------------------------------------------

    fn attach_highlighting(&mut self, buffer: BufferId) {
        let dark = self.dark;
        if let Some(b) = self.editor.buffer_mut(buffer) {
            self.highlighting.attach(b, dark);
        }
    }

    /// Changes the active buffer's language (from the status-bar picker).
    pub fn set_language(&mut self, language: &str) {
        let dark = self.dark;
        let id = self.buffer_id();
        if let Some(b) = self.editor.buffer_mut(id) {
            self.highlighting.set_language(b, language, dark);
        }
    }

    /// Makes `view` the active tab of its pane and gives it keyboard focus.
    pub fn activate(&mut self, view: ViewId) {
        if let Some(path) = self.dock.find_tab(&view) {
            let _ = self.dock.set_active_tab(path);
            self.dock.set_focused_node_and_surface(NodePath {
                surface: path.surface,
                node: path.node,
            });
        }
        self.active = view;
        self.focus_request = Some(view);
    }

    fn add_tab(&mut self, buffer: BufferId) -> ViewId {
        let view = self.editor.add_view(buffer).expect("buffer was just added");
        self.dock.push_to_focused_leaf(view);
        self.activate(view);
        view
    }

    /// A new, empty, untitled buffer in a new tab.
    pub fn new_file(&mut self) -> ViewId {
        let buffer = self.editor.add_buffer("", new_file_meta());
        self.untitled
            .insert(buffer, format!("new {}", self.next_untitled));
        self.next_untitled += 1;
        self.attach_highlighting(buffer);
        self.add_tab(buffer)
    }

    /// Opens `path` in a new tab, or switches to it if it is already open.
    /// An untouched "new" tab is replaced, as in Notepad++.
    pub fn open_path(&mut self, path: &Path) {
        let wanted = canonical(path);
        let existing = self
            .editor
            .buffers()
            .find(|b| b.meta.path.as_deref().map(canonical).as_ref() == Some(&wanted));
        let existing_view = existing
            .map(Buffer::id)
            .and_then(|b| self.editor.views_of(b).next().map(|v| v.id()));
        if let Some(view) = existing_view {
            self.activate(view);
            return;
        }
        let decoded = match fileio::load(path) {
            Ok(d) => d,
            Err(e) => {
                self.message = Some(format!("Could not open {}:\n{e}", path.display()));
                return;
            }
        };
        let replace = {
            let b = self.buffer();
            b.meta.path.is_none()
                && b.doc().is_empty()
                && !b.is_dirty()
                && self.editor.views_of(b.id()).count() == 1
        }
        .then_some(self.active);
        let buffer = self.editor.add_buffer(&decoded.text, decoded.meta);
        self.attach_highlighting(buffer);
        self.add_tab(buffer);
        if let Some(old) = replace {
            self.close_now(old);
        }
    }

    pub fn open_dialog(&mut self) {
        if let Some(paths) = rfd::FileDialog::new().pick_files() {
            for p in paths {
                self.open_path(&p);
            }
        }
    }

    /// A second view of the active buffer in the other pane (splitting the
    /// window if there is only one). It starts with the same carets and
    /// scroll position, then moves independently.
    pub fn clone_to_other_view(&mut self) -> ViewId {
        self.clone_view(self.active)
    }

    fn clone_view(&mut self, source: ViewId) -> ViewId {
        let Some(src) = self.editor.view(source).cloned() else {
            return self.active;
        };
        let view = self
            .editor
            .add_view(src.buffer())
            .expect("source buffer exists");
        if let Some(v) = self.editor.view_mut(view) {
            v.selections = src.selections.clone();
            v.scroll = src.scroll;
        }
        let here = self.dock.find_tab(&source).map(|p| NodePath {
            surface: p.surface,
            node: p.node,
        });
        let other = self
            .dock
            .iter_leaves()
            .map(|(path, _)| path)
            .find(|path| Some(*path) != here);
        match (other, here) {
            (Some(other), _) => {
                if let Ok(leaf) = self.dock.leaf_mut(other) {
                    leaf.tabs.push(view);
                }
            }
            (None, Some(here)) => {
                self.dock.split(here, Split::Right, 0.5, Node::leaf(view));
            }
            (None, None) => self.dock.push_to_focused_leaf(view),
        }
        self.activate(view);
        view
    }

    // --- Saving ------------------------------------------------------------------

    /// Saves the active buffer, asking for a path if it has none.
    pub fn save(&mut self) -> bool {
        self.save_buffer(self.buffer_id())
    }

    fn save_buffer(&mut self, buffer: BufferId) -> bool {
        let has_path = self
            .editor
            .buffer(buffer)
            .is_some_and(|b| b.meta.path.is_some());
        if !has_path {
            return self.save_buffer_as_dialog(buffer);
        }
        let Some(b) = self.editor.buffer_mut(buffer) else {
            return false;
        };
        match fileio::save_buffer(b) {
            Ok(()) => true,
            Err(e) => {
                self.message = Some(format!("Could not save:\n{e}"));
                false
            }
        }
    }

    pub fn save_as(&mut self) -> bool {
        self.save_buffer_as_dialog(self.buffer_id())
    }

    fn save_buffer_as_dialog(&mut self, buffer: BufferId) -> bool {
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(self.buffer_name(buffer))
            .save_file()
        else {
            return false;
        };
        self.save_buffer_to(buffer, &path)
    }

    /// Saves the active buffer to `path` (Save As without the dialog).
    pub fn save_to(&mut self, path: &Path) -> bool {
        self.save_buffer_to(self.buffer_id(), path)
    }

    fn save_buffer_to(&mut self, buffer: BufferId, path: &Path) -> bool {
        let Some(b) = self.editor.buffer_mut(buffer) else {
            return false;
        };
        match fileio::save_buffer_as(b, path) {
            Ok(()) => {
                self.untitled.remove(&buffer);
                // A new name may mean a new language, as in Notepad++.
                self.attach_highlighting(buffer);
                true
            }
            Err(e) => {
                self.message = Some(format!("Could not save {}:\n{e}", path.display()));
                false
            }
        }
    }

    // --- Closing -----------------------------------------------------------------

    /// Closes a tab, asking first if it is the last view of a dirty buffer.
    pub fn close_view(&mut self, view: ViewId) {
        self.close_queue.push_back(view);
    }

    pub fn close_others(&mut self, keep: ViewId) {
        let others: Vec<ViewId> = self.tabs().into_iter().filter(|v| *v != keep).collect();
        self.close_queue.extend(others);
    }

    pub fn close_all(&mut self) {
        let all = self.tabs();
        self.close_queue.extend(all);
    }

    /// Starts the exit sequence: asks about each dirty buffer in tab order;
    /// cancelling any prompt cancels the exit. Tabs stay open throughout.
    pub fn request_exit(&mut self) {
        self.exiting = true;
        let mut dirty = Vec::new();
        for v in self.tabs() {
            if let Some(b) = self.editor.view(v).map(|v| v.buffer())
                && self.editor.buffer(b).is_some_and(Buffer::is_dirty)
                && !dirty.contains(&b)
            {
                dirty.push(b);
            }
        }
        self.exit_queue = dirty.into();
    }

    fn may_discard(&self, buffer: BufferId) -> bool {
        !self.editor.buffer(buffer).is_some_and(Buffer::is_dirty)
            || self.discard_approved.contains(&buffer)
    }

    /// Shows the save prompt for `buffer` (switching to one of its views).
    fn ask_about(&mut self, buffer: BufferId) {
        let shown = self
            .confirming
            .and_then(|v| self.editor.view(v))
            .is_some_and(|v| v.buffer() == buffer);
        let view = self.editor.views_of(buffer).next().map(|v| v.id());
        if !shown && let Some(view) = view {
            self.confirming = Some(view);
            self.activate(view);
        }
    }

    /// Asks about the next dirty buffer before exiting, or allows the exit.
    fn process_exit_queue(&mut self) {
        if !self.exiting {
            return;
        }
        while let Some(&buffer) = self.exit_queue.front() {
            if self.editor.buffer(buffer).is_none() || self.may_discard(buffer) {
                self.exit_queue.pop_front();
                continue;
            }
            self.ask_about(buffer);
            return;
        }
        self.close_confirmed = true;
    }

    /// Whether the app has finished its exit sequence and may close.
    pub fn may_close(&self) -> bool {
        self.close_confirmed
    }

    /// Closes the queued views until one needs a prompt.
    fn process_close_queue(&mut self) {
        while let Some(&view) = self.close_queue.front() {
            let Some(buffer) = self.editor.view(view).map(|v| v.buffer()) else {
                self.close_queue.pop_front();
                continue;
            };
            let last_view = self.editor.views_of(buffer).count() == 1;
            if last_view && !self.may_discard(buffer) {
                if self.confirming != Some(view) {
                    self.confirming = Some(view);
                    self.activate(view);
                }
                return;
            }
            self.close_queue.pop_front();
            self.close_now(view);
        }
        if self.tabs().is_empty() {
            self.new_file();
        }
    }

    /// Removes a tab and its view, and the buffer if this was its last view.
    fn close_now(&mut self, view: ViewId) {
        if let Some(path) = self.dock.find_tab(&view) {
            self.dock.remove_tab(path);
        }
        let buffer = self.editor.view(view).map(|v| v.buffer());
        self.editor.remove_view(view);
        self.view_states.remove(&view);
        if self.confirming == Some(view) {
            self.confirming = None;
        }
        if let Some(buffer) = buffer
            && self.editor.views_of(buffer).next().is_none()
        {
            self.editor.remove_buffer(buffer);
            self.discard_approved.remove(&buffer);
            self.highlighting.detach(buffer);
            self.untitled.remove(&buffer);
        }
        if self.active == view {
            let next = self
                .dock
                .find_active_focused()
                .map(|(_, v)| *v)
                .or_else(|| self.tabs().first().copied());
            if let Some(next) = next {
                self.activate(next);
            }
        }
    }

    fn cycle_tab(&mut self, forward: bool) {
        let Some(path) = self.dock.find_tab(&self.active) else {
            return;
        };
        let node = NodePath {
            surface: path.surface,
            node: path.node,
        };
        let Ok(leaf) = self.dock.leaf(node) else {
            return;
        };
        let n = leaf.tabs.len();
        if n < 2 {
            return;
        }
        let i = path.tab.0;
        let next = if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        };
        let view = leaf.tabs[next];
        let _ = self.dock.set_active_tab(egui_dock::TabPath {
            tab: TabIndex(next),
            ..path
        });
        self.activate(view);
    }

    fn apply(&mut self, action: Action) {
        let _ = self.editor.apply(self.active, action);
        let state = self.view_state_mut();
        state.reveal_caret();
        state.request_focus();
    }

    // --- UI ------------------------------------------------------------------------

    /// Draws the whole application into `ui`.
    pub fn show(&mut self, ui: &mut Ui) {
        self.shortcuts(ui);
        if ui.input(|i| i.viewport().close_requested()) && !self.close_confirmed {
            ui.ctx().send_viewport_cmd(ViewportCommand::CancelClose);
            if self.editor.buffers().any(Buffer::is_dirty) {
                if !self.exiting {
                    self.request_exit();
                }
            } else {
                self.close_confirmed = true;
            }
        }
        if self.exit_queue.is_empty() || !self.exiting {
            self.process_close_queue();
        }
        self.process_exit_queue();

        Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        if self.find.open {
            let requests = Panel::bottom("find")
                .show(ui, |ui| self.find.ui(ui, &mut self.editor, self.active))
                .inner;
            // Escape closes the panel unless the editor has the keyboard
            // (there it collapses selections).
            let editor_focused = ui.memory(|m| m.has_focus(editor_id(self.active)));
            let mut requests = requests;
            if !editor_focused && ui.input(|i| i.key_pressed(Key::Escape)) {
                requests.push(FindRequest::Close);
            }
            for r in requests {
                match r {
                    FindRequest::Close => {
                        self.find.open = false;
                        self.view_state_mut().request_focus();
                    }
                    FindRequest::Reveal => self.view_state_mut().reveal_caret(),
                }
            }
        }

        self.dark = ui.visuals().dark_mode;
        if self
            .highlighting
            .frame(self.editor.buffers_mut(), &self.visible, self.dark)
        {
            ui.ctx().request_repaint();
        }
        self.visible.clear();

        let mut commands = Vec::new();
        let mut tab_rects = Vec::new();
        CentralPanel::no_frame().show(ui, |ui| {
            let style = Style::from_egui(ui.style());
            let find_highlight = self.find.highlighter();
            let mut tabs = Tabs {
                find_highlight,
                editor: &mut self.editor,
                view_states: &mut self.view_states,
                untitled: &self.untitled,
                highlighting: &mut self.highlighting,
                keymap: &self.keymap,
                focus_request: &mut self.focus_request,
                visible: &mut self.visible,
                commands: &mut commands,
                tab_rects: &mut tab_rects,
            };
            DockArea::new(&mut self.dock)
                .id(Id::new("ochrepad-dock"))
                .style(style)
                .show_add_buttons(false)
                .show_close_buttons(true)
                .allowed_splits(AllowedSplits::All)
                .show_leaf_close_all_buttons(false)
                .show_leaf_collapse_buttons(false)
                .show_inside(ui, &mut tabs);
        });
        self.new_tab_on_tab_bar_double_click(ui, &tab_rects);
        if let Some((_, view)) = self.dock.find_active_focused()
            && *view != self.active
        {
            self.active = *view;
            self.focus_request = Some(self.active);
        }
        for cmd in commands {
            match cmd {
                TabCommand::Close(v) => self.close_view(v),
                TabCommand::CloseOthers(v) => self.close_others(v),
                TabCommand::CloneToOtherView(v) => {
                    self.clone_view(v);
                }
            }
        }

        self.dialogs(ui);
        if self.close_confirmed && !self.close_sent {
            self.close_sent = true;
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

    /// Double-clicking the empty part of a pane's tab bar opens a new tab in
    /// that pane, as in Notepad++. egui_dock has no hook for this, so the
    /// click is matched against the tab-bar strip (leaf rect above its body)
    /// minus the tab buttons drawn this frame.
    fn new_tab_on_tab_bar_double_click(&mut self, ui: &Ui, tab_rects: &[egui::Rect]) {
        let Some(p) = ui.input(|i| {
            i.pointer
                .button_double_clicked(egui::PointerButton::Primary)
                .then(|| i.pointer.interact_pos())
                .flatten()
        }) else {
            return;
        };
        if tab_rects.iter().any(|r| r.contains(p)) {
            return;
        }
        let pane = self.dock.iter_leaves().find_map(|(path, leaf)| {
            let bar = egui::Rect::from_min_max(
                leaf.rect.min,
                egui::pos2(leaf.rect.max.x, leaf.viewport.min.y),
            );
            bar.contains(p).then_some(path)
        });
        if let Some(path) = pane {
            self.dock.set_focused_node_and_surface(path);
            self.new_file();
        }
    }

    fn shortcuts(&mut self, ui: &mut Ui) {
        let pressed = |ui: &mut Ui, s: &KeyboardShortcut| ui.input_mut(|i| i.consume_shortcut(s));
        if pressed(ui, &NEW) {
            self.new_file();
        }
        if pressed(ui, &OPEN) {
            self.open_dialog();
        }
        if pressed(ui, &SAVE_AS) {
            self.save_as();
        }
        if pressed(ui, &SAVE) {
            self.save();
        }
        if pressed(ui, &FIND) {
            self.find.open(&self.editor, self.active, false);
        }
        if pressed(ui, &REPLACE) {
            self.find.open(&self.editor, self.active, true);
        }
        if pressed(ui, &FIND_PREV) && !self.find.pattern.is_empty() {
            self.find.find(&mut self.editor, self.active, true);
            self.view_state_mut().reveal_caret();
        }
        if pressed(ui, &FIND_NEXT) && !self.find.pattern.is_empty() {
            self.find.find(&mut self.editor, self.active, false);
            self.view_state_mut().reveal_caret();
        }
        if pressed(ui, &CLOSE) {
            self.close_view(self.active);
        }
        if pressed(ui, &PREV_TAB) {
            self.cycle_tab(false);
        }
        if pressed(ui, &NEXT_TAB) {
            self.cycle_tab(true);
        }
    }

    fn menu_bar(&mut self, ui: &mut Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| self.file_menu(ui));
            ui.menu_button("Edit", |ui| self.edit_menu(ui));
            ui.menu_button("Search", |ui| self.search_menu(ui));
            ui.menu_button("View", |ui| self.view_menu(ui));
        });
    }

    fn file_menu(&mut self, ui: &mut Ui) {
        let item = |ui: &mut Ui, label: &str, shortcut: &str| {
            ui.add(Button::new(label).shortcut_text(shortcut)).clicked()
        };
        if item(ui, "New", "Ctrl+N") {
            self.new_file();
        }
        if item(ui, "Open…", "Ctrl+O") {
            self.open_dialog();
        }
        if item(ui, "Save", "Ctrl+S") {
            self.save();
        }
        if item(ui, "Save As…", "Ctrl+Shift+S") {
            self.save_as();
        }
        ui.separator();
        if item(ui, "Close", "Ctrl+W") {
            self.close_view(self.active);
        }
        if ui.button("Close All but Active").clicked() {
            self.close_others(self.active);
        }
        if ui.button("Close All").clicked() {
            self.close_all();
        }
        ui.separator();
        if ui.button("Exit").clicked() {
            self.request_exit();
        }
    }

    fn edit_menu(&mut self, ui: &mut Ui) {
        let item = |ui: &mut Ui, label: &str, shortcut: &str| {
            ui.add(Button::new(label).shortcut_text(shortcut)).clicked()
        };
        if item(ui, "Undo", "Ctrl+Z") {
            self.apply(Action::Undo);
        }
        if item(ui, "Redo", "Ctrl+Y") {
            self.apply(Action::Redo);
        }
        ui.separator();
        if item(ui, "Cut", "Ctrl+X") && copy_selection(ui.ctx(), &self.editor, self.active) {
            self.apply(Action::InsertText(String::new()));
        }
        if item(ui, "Copy", "Ctrl+C") {
            copy_selection(ui.ctx(), &self.editor, self.active);
            self.view_state_mut().request_focus();
        }
        if item(ui, "Paste", "Ctrl+V") {
            // The integration answers with an Event::Paste, which the
            // (refocused) editor handles next frame.
            ui.ctx().send_viewport_cmd(ViewportCommand::RequestPaste);
            self.view_state_mut().request_focus();
        }
        ui.separator();
        if item(ui, "Select All", "Ctrl+A") {
            self.apply(Action::SelectAll);
        }
    }

    fn search_menu(&mut self, ui: &mut Ui) {
        let item = |ui: &mut Ui, label: &str, shortcut: &str| {
            ui.add(Button::new(label).shortcut_text(shortcut)).clicked()
        };
        if item(ui, "Find…", "Ctrl+F") {
            self.find.open(&self.editor, self.active, false);
        }
        if item(ui, "Replace…", "Ctrl+H") {
            self.find.open(&self.editor, self.active, true);
        }
        if item(ui, "Find Next", "F3") && !self.find.pattern.is_empty() {
            self.find.find(&mut self.editor, self.active, false);
            self.view_state_mut().reveal_caret();
        }
        if item(ui, "Find Previous", "Shift+F3") && !self.find.pattern.is_empty() {
            self.find.find(&mut self.editor, self.active, true);
            self.view_state_mut().reveal_caret();
        }
    }

    fn view_menu(&mut self, ui: &mut Ui) {
        if ui.button("Clone to Other View").clicked() {
            self.clone_to_other_view();
        }
        if ui
            .add(Button::new("Next Tab").shortcut_text("Ctrl+Tab"))
            .clicked()
        {
            self.cycle_tab(true);
        }
        if ui
            .add(Button::new("Previous Tab").shortcut_text("Ctrl+Shift+Tab"))
            .clicked()
        {
            self.cycle_tab(false);
        }
        ui.separator();
        let dark = ui.visuals().dark_mode;
        if ui.radio(dark, "Dark mode").clicked() {
            ui.ctx().set_theme(egui::ThemePreference::Dark);
        }
        if ui.radio(!dark, "Light mode").clicked() {
            ui.ctx().set_theme(egui::ThemePreference::Light);
        }
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

    /// "Ln 3, Col 7" for the primary caret (columns are visual, 1-based).
    pub fn caret_status(&self) -> String {
        let doc = self.buffer().doc();
        let Some(v) = self.editor.view(self.active) else {
            return String::new();
        };
        let head = v.selections.primary().head;
        let col = byte_to_visual_col(doc, head, self.editor.settings.tab_width);
        format!("Ln {}, Col {}", doc.line_of(head) + 1, col.0 + 1)
    }

    fn language_picker(&mut self, ui: &mut Ui) {
        let current = self.highlighting.language(self.buffer_id()).to_owned();
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
        let (carets, column) = self.editor.view(self.active).map_or((1, false), |v| {
            (v.selections.len(), v.selections.rect().is_some())
        });
        let meta = self.buffer().meta.clone();
        ui.horizontal(|ui| {
            ui.label(self.caret_status());
            if column {
                ui.separator();
                ui.label(format!("Column selection, {carets} lines"));
            } else if carets > 1 {
                ui.separator();
                ui.label(format!("{carets} selections"));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let enc = if meta.bom {
                    format!("{} BOM", meta.encoding)
                } else {
                    meta.encoding.clone()
                };
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
                        .on_hover_text(
                            "The file had more than one kind of line ending. \
                         Saving converts all of them to the one shown.",
                        );
                }
                ui.separator();
                self.language_picker(ui);
                if let Some(problem) = self.highlighting.problem(self.buffer_id()) {
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
                self.view_state_mut().request_focus();
            }
        }
        let Some(view) = self.confirming else {
            return;
        };
        let Some(buffer) = self.editor.view(view).map(|v| v.buffer()) else {
            self.confirming = None;
            return;
        };
        let name = self.buffer_name(buffer);
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
            Some(1) if self.save_buffer(buffer) => {
                self.confirming = None;
            }
            Some(2) => {
                self.discard_approved.insert(buffer);
                self.confirming = None;
            }
            // Cancel, or a save that failed or was cancelled.
            Some(_) => {
                self.confirming = None;
                self.close_queue.clear();
                self.exit_queue.clear();
                self.exiting = false;
                self.view_state_mut().request_focus();
            }
            None => {}
        }
    }
}

/// Draws the tabs: one editor per view.
struct Tabs<'a> {
    find_highlight: Option<&'a ochrepad_core::Searcher>,
    editor: &'a mut Editor,
    view_states: &'a mut HashMap<ViewId, EditorViewState>,
    untitled: &'a HashMap<BufferId, String>,
    highlighting: &'a mut Highlighting,
    keymap: &'a Keymap,
    focus_request: &'a mut Option<ViewId>,
    visible: &'a mut HashMap<BufferId, Range<usize>>,
    commands: &'a mut Vec<TabCommand>,
    /// Tab buttons drawn this frame (to tell the empty tab bar apart).
    tab_rects: &'a mut Vec<egui::Rect>,
}

impl TabViewer for Tabs<'_> {
    type Tab = ViewId;

    fn id(&mut self, tab: &mut ViewId) -> Id {
        Id::new(("ochrepad-tab", tab.0))
    }

    fn title(&mut self, tab: &mut ViewId) -> WidgetText {
        let Some(buffer) = self.editor.view(*tab).map(|v| v.buffer()) else {
            return "?".into();
        };
        let name = buffer_name(self.editor, self.untitled, buffer);
        let dirty = self.editor.buffer(buffer).is_some_and(Buffer::is_dirty);
        if dirty {
            format!("{name} *").into()
        } else {
            name.into()
        }
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut ViewId) {
        let view = *tab;
        let Some(v) = self.editor.view(view) else {
            return;
        };
        let buffer = v.buffer();
        let first = v.scroll.first_line;
        let lines = first..first + 200;
        self.visible
            .entry(buffer)
            .and_modify(|r| *r = r.start.min(lines.start)..r.end.max(lines.end))
            .or_insert(lines);
        let label = format!(
            "Editor: {}",
            buffer_name(self.editor, self.untitled, buffer)
        );
        let state = self.view_states.entry(view).or_default();
        if *self.focus_request == Some(view) {
            state.request_focus();
            *self.focus_request = None;
        }
        let mut styler = self.highlighting.styler(buffer);
        editor_view(
            ui,
            self.editor,
            view,
            state,
            self.keymap,
            &label,
            Decorations {
                styler: styler.as_mut().map(|s| s as &mut dyn LineStyler),
                find_highlight: self.find_highlight,
            },
        );
    }

    fn context_menu(&mut self, ui: &mut Ui, tab: &mut ViewId, _path: NodePath) {
        if ui.button("Close").clicked() {
            self.commands.push(TabCommand::Close(*tab));
        }
        if ui.button("Close All but This").clicked() {
            self.commands.push(TabCommand::CloseOthers(*tab));
        }
        if ui.button("Clone to Other View").clicked() {
            self.commands.push(TabCommand::CloneToOtherView(*tab));
        }
    }

    fn on_tab_button(&mut self, tab: &mut ViewId, response: &egui::Response) {
        self.tab_rects.push(response.rect);
        // egui_dock gives tab buttons no accessibility info.
        let title = self.title(tab).text().to_owned();
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &title));
    }

    fn on_close(&mut self, tab: &mut ViewId) -> OnCloseResponse {
        // The app closes it after any save prompt.
        self.commands.push(TabCommand::Close(*tab));
        OnCloseResponse::Ignore
    }

    fn allowed_in_windows(&self, _tab: &mut ViewId) -> bool {
        false
    }

    fn scroll_bars(&self, _tab: &ViewId) -> [bool; 2] {
        [false, false]
    }
}

fn buffer_name(editor: &Editor, untitled: &HashMap<BufferId, String>, buffer: BufferId) -> String {
    if let Some(name) = untitled.get(&buffer) {
        return name.clone();
    }
    editor
        .buffer(buffer)
        .and_then(|b| b.meta.path.as_deref())
        .and_then(Path::file_name)
        .map_or_else(
            || "untitled".to_owned(),
            |n| n.to_string_lossy().into_owned(),
        )
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn new_file_meta() -> FileMeta {
    FileMeta {
        line_ending: fileio::default_line_ending(),
        ..FileMeta::default()
    }
}
