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
use egui_dock::{AllowedSplits, DockArea, DockState, Node, NodePath, Split, TabIndex, TabViewer};
use ochrepad_core::position::byte_to_visual_col;
use ochrepad_core::{Action, Buffer, BufferId, CaseMode, Editor, FileMeta, SortOptions, ViewId};
use ochrepad_fileio as fileio;

use egui_dock::tab_viewer::OnCloseResponse;

use crate::editor_view::{
    Decorations, EditorViewState, LineStyler, copy_selection, editor_id, editor_view,
};
mod autosave;
mod document;
mod edit_tools;
mod external;
mod file_extras;
mod history_store;
mod prefs;
mod replace_in_files;
mod search;
mod summary;
mod tools;
mod viewers;
mod workspace;

pub use autosave::{AutosaveSettings, Session, SessionBuffer, SessionTab, SessionViewer};
pub use external::DiskChange;

use crate::compare::{Compare, CompareId, CompareSide, Compares};
use crate::find_panel::{FindPanel, FindRequest, FindTarget};
use crate::highlight::Highlighting;
use crate::keymap::Keymap;
use crate::macro_ui::{MacroSource, Macros};
use crate::preview::Previews;
use crate::settings::{AppSettings, ThemeChoice};

/// How many closed files Restore Recent Closed File remembers.
const MAX_CLOSED_FILES: usize = 20;

/// What a command that would change a file says when a viewer has the focus.
pub(crate) const READ_ONLY: &str = "This file is open read-only (View > Open as Editable)";

const NEW: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
const OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
const SAVE_AS: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S);
const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
const CLOSE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::W);
const FIND: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::F);
const REPLACE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::H);
const FIND_NEXT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F3);
const SELECT_FIND_NEXT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::F3);
const SELECT_FIND_PREV: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::F3);
const RESTORE_CLOSED: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::T);
const FIND_PREV: KeyboardShortcut = KeyboardShortcut::new(Modifiers::SHIFT, Key::F3);
const RECORD_MACRO: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::R);
const PLAY_MACRO: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::P);
const NEXT_TAB: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Tab);
const NEXT_TAB_PAGE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::PageDown);
const PREV_TAB_PAGE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::PageUp);
const MOVE_TAB_FORWARD: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::PageDown);
const MOVE_TAB_BACKWARD: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::PageUp);
const PREV_TAB: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Tab);

/// What a dock tab shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tab {
    Editor(ViewId),
    /// The Markdown preview of a buffer.
    Preview(BufferId),
    /// A side-by-side comparison.
    Compare(CompareId),
    /// A file too large to edit, read-only (`crate::viewer`).
    Viewer(crate::viewer::ViewerId),
}

const NEXT_DIFF: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F8);
const PREV_DIFF: KeyboardShortcut = KeyboardShortcut::new(Modifiers::SHIFT, Key::F8);

const GO_TO_LINE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::G);
/// Ctrl+Shift+L is Notepad++'s Delete Line.
const FILTER_LINES: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::ALT.plus(Modifiers::SHIFT), Key::L);
const ZOOM_IN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Plus);
const ZOOM_IN_EQUALS: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Equals);
const ZOOM_OUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Minus);
const ZOOM_RESET: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Num0);
/// Word's shortcut for showing formatting marks.
const SHOW_ALL_CHARACTERS: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Num8);
const FIND_IN_FILES: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::F);
const SIDEBAR: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::E);
const PREVIEW: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::M);

/// Requests collected while the dock is drawn, handled afterwards.
enum TabCommand {
    Close(ViewId),
    CloseOthers(ViewId),
    CloneToOtherView(ViewId),
    /// A link clicked in a Markdown preview.
    Link(crate::preview::LinkClick),
    /// A file command from a tab's context menu.
    File(edit_tools::TabFileCommand, ViewId),
    CloseViewer(crate::viewer::ViewerId),
    OpenAsEditable(crate::viewer::ViewerId),
    /// A preview or comparison closed by a middle click.
    CloseTab(Tab),
    /// A command from an editor's right-click menu.
    Menu(ViewId, crate::editor_menu::MenuCommand),
}

pub struct OchrepadApp {
    pub editor: Editor,
    pub dock: DockState<Tab>,
    pub previews: Previews,
    pub compares: Compares,
    /// Files too large to edit, open read-only.
    pub viewers: crate::viewer::Viewers,
    /// The viewer whose Open as Editable question is showing.
    confirm_editable: Option<crate::viewer::ViewerId>,
    view_states: HashMap<ViewId, EditorViewState>,
    /// Names of buffers that have never been saved ("new 1", "new 2", …).
    untitled: HashMap<BufferId, String>,
    /// Files closed this session, the latest last (Restore Recent Closed
    /// File).
    closed_files: Vec<PathBuf>,
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
    /// Each tab's filter (Search > Filter Lines).
    pub filter: crate::filter_bar::FilterBar,
    /// The lines changed since each file was saved (the gutter's marks).
    pub changes: crate::change_history::ChangeHistory,
    /// View > Function List.
    pub function_list: crate::function_list::FunctionList,
    /// View > Document List.
    document_list: crate::document_list::DocumentList,
    /// Search > Style All Occurrences of Token.
    pub token_styles: crate::token_styles::TokenStyles,
    /// The View > Summary dialog, while it is open.
    summary: Option<summary::SummaryDialog>,
    /// The File > Rename… prompt, while it is open.
    rename: Option<file_extras::RenameDialog>,
    /// Pages File > Export > HTML… is still making.
    exports: Vec<crate::export::Export>,
    /// Replace in Files, while it counts, asks or writes.
    replace_in_files: replace_in_files::Stage,
    /// Saves Replace in Files made that the worker finished: whether it
    /// worked, the matches, the error.
    replace_saves: Vec<(bool, usize, Option<String>)>,
    /// The log highlight rules from the settings, compiled.
    log_rules: crate::log_rules::LogRules,
    /// The Search Results panel.
    pub results: crate::search_results::SearchResults,
    replace_all_plan: Option<search::ReplaceAllPlan>,
    /// The open folder's file tree.
    tree: Option<crate::file_tree::FileTree>,
    new_entry: Option<workspace::NewEntry>,
    /// The time at the start of this frame.
    now: f64,
    /// A short note in the status bar (an action that could not run), and
    /// when it goes away.
    status_note: Option<(String, f64)>,
    /// The Go to Line dialog's field, while it is open.
    go_to_line: Option<String>,
    /// The hash dialog (Text Tools > Hashes), while it is open.
    hashes: Option<tools::HashDialog>,
    /// The Column Editor and the other Notepad++ editing extras.
    edit_tools: edit_tools::EditTools,
    pub macros: Macros,
    persist: autosave::Persist,
    external: external::External,
    /// User settings; changes are applied every frame and saved.
    pub settings: AppSettings,
    saved_settings: Option<AppSettings>,
    applied_settings: Option<AppSettings>,
    settings_file: Option<PathBuf>,
    keymap_file: Option<PathBuf>,
    applied_theme: Option<ThemeChoice>,
    applied_font: String,
    font_choices: Vec<(String, PathBuf)>,
    pub show_preferences: bool,
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
            dock: DockState::new(vec![Tab::Editor(view)]),
            previews: Previews::default(),
            compares: Compares::default(),
            viewers: Default::default(),
            confirm_editable: None,
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
            filter: Default::default(),
            changes: Default::default(),
            function_list: Default::default(),
            document_list: Default::default(),
            token_styles: Default::default(),
            summary: None,
            rename: None,
            exports: Vec::new(),
            replace_in_files: Default::default(),
            replace_saves: Vec::new(),
            closed_files: Vec::new(),
            log_rules: Default::default(),
            results: Default::default(),
            replace_all_plan: None,
            tree: None,
            new_entry: None,
            now: 0.0,
            status_note: None,
            go_to_line: None,
            hashes: None,
            edit_tools: Default::default(),
            macros: Macros::default(),
            persist: autosave::Persist::default(),
            external: external::External::default(),
            settings: AppSettings::default(),
            saved_settings: None,
            applied_settings: None,
            settings_file: None,
            keymap_file: None,
            applied_theme: None,
            applied_font: String::new(),
            font_choices: crate::fonts::monospace_choices(),
            show_preferences: false,
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
        self.dock
            .iter_all_tabs()
            .filter_map(|(_, t)| match t {
                Tab::Editor(v) => Some(*v),
                Tab::Preview(_) | Tab::Compare(_) | Tab::Viewer(_) => None,
            })
            .collect()
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
        if let Some(path) = self.dock.find_tab(&Tab::Editor(view)) {
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
        self.dock.push_to_focused_leaf(Tab::Editor(view));
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
        if self.opens_in_viewer(path) {
            self.open_viewer(path);
            return;
        }
        self.open_in_editor(path);
    }

    /// Opens `path` in a new editor tab, however large it is (up to what
    /// `fileio` loads): `open_path` without the viewer.
    fn open_in_editor(&mut self, path: &Path) {
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
        self.detect_indent(buffer);
        self.restore_file_history(buffer, path);
        self.record_disk_stamp(buffer);
        self.settings.add_recent(canonical(path));
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
        let here = self.dock.find_tab(&Tab::Editor(source)).map(|p| NodePath {
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
                    leaf.tabs.push(Tab::Editor(view));
                }
            }
            (None, Some(here)) => {
                self.dock
                    .split(here, Split::Right, 0.5, Node::leaf(Tab::Editor(view)));
            }
            (None, None) => self.dock.push_to_focused_leaf(Tab::Editor(view)),
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
        let Some(path) = self.editor.buffer(buffer).and_then(|b| b.meta.path.clone()) else {
            return false;
        };
        self.submit_save(buffer, path)
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

    /// Saves the active buffer to `path` (Save As without the dialog). The
    /// write happens in the background; the buffer takes the path and is
    /// marked clean when it finishes.
    pub fn save_to(&mut self, path: &Path) -> bool {
        self.save_buffer_to(self.buffer_id(), path)
    }

    fn save_buffer_to(&mut self, buffer: BufferId, path: &Path) -> bool {
        self.submit_save(buffer, path.to_path_buf())
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
        if self.writing_in_files() {
            self.message = Some(
                "Replace in Files is still saving files.\n\
                 Wait for it to finish, or press Stop, then exit."
                    .to_owned(),
            );
            return;
        }
        if self.snapshots_active() {
            self.exit_with_snapshots();
            return;
        }
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
        self.write_all_histories();
        self.write_session();
        self.flush_disk();
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
            if last_view && self.is_saving(buffer) {
                return;
            }
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

    /// The tab the filter bar is for: the focused viewer, or the active
    /// editor.
    /// The document revision an editor's filter was last made for (tests).
    pub fn filter_applied_at(&self, view: ViewId) -> Option<u64> {
        self.filter.applied_revision(view)
    }

    fn filter_target(&mut self) -> crate::filter_bar::FilterTarget {
        match self.focused_viewer() {
            Some(id) => id.into(),
            None => self.active.into(),
        }
    }

    /// Search > Filter Lines (Alt+Shift+L) on the focused tab.
    pub fn open_filter(&mut self) {
        let target = self.filter_target();
        self.filter
            .open(&self.editor, &self.viewers, target, self.now);
    }

    /// Removes a tab and its view, and the buffer if this was its last view.
    fn close_now(&mut self, view: ViewId) {
        if let Some(path) = self.dock.find_tab(&Tab::Editor(view)) {
            self.dock.remove_tab(path);
        }
        if self.filter.is_open(view) {
            self.filter
                .close(&mut self.editor, &mut self.viewers, view.into());
        }
        if let Some(b) = self.editor.view(view).map(|v| v.buffer())
            && self.editor.views_of(b).count() == 1
        {
            self.set_following(b, false);
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
            // Its undo history is kept if the text is what is on disk.
            if self.editor.buffer(buffer).is_some_and(|b| !b.is_dirty()) {
                self.write_history(buffer);
            }
            self.persist.history_written.remove(&buffer);
            // Closed without saving: its backup goes too, and its preview.
            self.delete_snapshot(buffer);
            if let Some(path) = self.dock.find_tab(&Tab::Preview(buffer)) {
                self.dock.remove_tab(path);
            }
            self.previews.remove(buffer);
            for id in self.compares.involving(buffer) {
                if let Some(path) = self.dock.find_tab(&Tab::Compare(id)) {
                    self.dock.remove_tab(path);
                }
                self.compares.remove(id, &mut self.editor);
            }
            if let Some(path) = self.editor.buffer(buffer).and_then(|b| b.meta.path.clone())
                && !self.exiting
            {
                self.closed_files.retain(|p| *p != path);
                self.closed_files.push(path);
                let extra = self.closed_files.len().saturating_sub(MAX_CLOSED_FILES);
                self.closed_files.drain(..extra);
            }
            self.editor.remove_buffer(buffer);
            self.discard_approved.remove(&buffer);
            self.highlighting.detach(buffer);
            self.untitled.remove(&buffer);
        }
        if self.active == view {
            let next = self
                .dock
                .find_active_focused()
                .and_then(|(_, t)| match t {
                    Tab::Editor(v) => Some(*v),
                    Tab::Preview(_) | Tab::Compare(_) | Tab::Viewer(_) => None,
                })
                .or_else(|| self.tabs().first().copied());
            if let Some(next) = next {
                self.activate(next);
            }
        }
    }

    fn cycle_tab(&mut self, forward: bool) {
        let current = self
            .focused_viewer()
            .map_or(Tab::Editor(self.active), Tab::Viewer);
        let Some(path) = self.dock.find_tab(&current) else {
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
        let tab = leaf.tabs[next];
        let _ = self.dock.set_active_tab(egui_dock::TabPath {
            tab: TabIndex(next),
            ..path
        });
        match tab {
            Tab::Editor(view) => self.activate(view),
            Tab::Viewer(id) => self.activate_viewer(id),
            Tab::Preview(_) | Tab::Compare(_) => {}
        }
    }

    /// Moves the active tab one place along its tab bar (Notepad++'s Move
    /// Tab Forward and Backward); at either end it stays.
    pub fn move_tab(&mut self, forward: bool) {
        let current = self
            .focused_viewer()
            .map_or(Tab::Editor(self.active), Tab::Viewer);
        let Some(path) = self.dock.find_tab(&current) else {
            return;
        };
        let node = NodePath {
            surface: path.surface,
            node: path.node,
        };
        let Ok(leaf) = self.dock.leaf_mut(node) else {
            return;
        };
        let i = path.tab.0;
        let j = if forward { i + 1 } else { i.wrapping_sub(1) };
        if j < leaf.tabs.len() {
            leaf.tabs.swap(i, j);
            leaf.active = TabIndex(j);
        }
    }

    /// Plays a macro on the active view; failures are shown in a dialog.
    pub fn play_macro(&mut self, source: MacroSource, repeat: ochrepad_core::Repeat) {
        if self.focused_viewer().is_some() {
            self.note_text(READ_ONLY);
            return;
        }
        match self
            .macros
            .play(&mut self.editor, self.active, source, repeat)
        {
            Ok(_) => {}
            Err(msg) => self.message = Some(msg),
        }
        let state = self.view_state_mut();
        state.reveal_caret();
        state.request_focus();
    }

    /// Uses `path` for saved macros (normally `macros.json` in the config
    /// directory).
    pub fn set_macro_file(&mut self, path: PathBuf) {
        if let Err(e) = self.macros.set_file(path) {
            self.message = Some(format!("Could not load saved macros:\n{e}"));
        }
    }

    fn apply(&mut self, action: Action) {
        if let Err(e) = self.editor.apply(self.active, action) {
            self.note(&e);
        }
        let state = self.view_state_mut();
        state.reveal_caret();
        state.request_focus();
    }

    // --- UI ------------------------------------------------------------------------

    /// Draws the whole application into `ui`.
    pub fn show(&mut self, ui: &mut Ui) {
        self.now = ui.input(|i| i.time);
        self.apply_settings(ui);
        self.poll_disk(ui);
        self.poll_exports(ui);
        self.watch_files(ui);
        self.drag_and_drop(ui);
        self.shortcuts(ui);
        if ui.input(|i| i.viewport().close_requested()) && !self.close_confirmed {
            ui.ctx().send_viewport_cmd(ViewportCommand::CancelClose);
            if !self.exiting {
                self.request_exit();
            }
        }
        if self.exit_queue.is_empty() || !self.exiting {
            self.process_close_queue();
        }
        self.process_exit_queue();

        let bar = |ui: &Ui, y: i8| {
            egui::Frame::new()
                .fill(ui.visuals().panel_fill)
                .inner_margin(egui::Margin::symmetric(8, y))
        };
        Panel::top("menu")
            .frame(bar(ui, 3))
            .show(ui, |ui| self.menu_bar(ui));
        Panel::bottom("status")
            .frame(bar(ui, 2))
            .show(ui, |ui| self.status_bar(ui));
        if self.find.open {
            self.find.replacing = self.replacing_in_files();
            let viewer = self.focused_viewer();
            let target = match viewer {
                Some(_) => FindTarget::Viewer,
                None => FindTarget::Editor(&mut self.editor, self.active),
            };
            let requests = Panel::bottom("find")
                .show(ui, |ui| self.find.ui(ui, target))
                .inner;
            // Escape closes the panel unless the editor (or viewer) has the
            // keyboard (there it collapses selections).
            let editor_focused = ui.memory(|m| {
                m.has_focus(editor_id(self.active))
                    || viewer.is_some_and(|id| m.has_focus(crate::viewer::viewer_widget_id(id)))
            });
            let mut requests = requests;
            if !editor_focused && ui.input(|i| i.key_pressed(Key::Escape)) {
                requests.push(FindRequest::Close);
            }
            for r in requests {
                match r {
                    FindRequest::Close => {
                        self.find.open = false;
                        match viewer.and_then(|id| self.viewers.get_mut(id)) {
                            Some(v) => v.state.request_focus(),
                            None => self.view_state_mut().request_focus(),
                        }
                    }
                    FindRequest::Reveal => self.view_state_mut().reveal_caret(),
                    FindRequest::FindAll {
                        all_documents: false,
                    } if viewer.is_some() => {
                        if let Some(id) = viewer {
                            self.viewer_find_all(id);
                        }
                    }
                    FindRequest::FindAll { all_documents } => self.find_all(all_documents),
                    FindRequest::Next { backward } => {
                        if let Some(id) = viewer {
                            self.viewer_find(id, backward);
                        }
                    }
                    FindRequest::Count => {
                        if let Some(id) = viewer {
                            self.viewer_count(id);
                        }
                    }
                    FindRequest::ReplaceAllInAll => self.plan_replace_all_in_all(),
                    FindRequest::FindInFiles => self.find_in_files(),
                    FindRequest::ReplaceInFiles => self.plan_replace_in_files(),
                }
            }
        }
        let target = self.filter_target();
        if self.filter.is_open(target) {
            let requests = Panel::bottom("filter")
                .show(ui, |ui| {
                    self.filter
                        .ui(ui, &self.editor, &self.viewers, target, self.now)
                })
                .inner;
            for r in requests {
                match r {
                    crate::filter_bar::FilterRequest::Close => {
                        self.filter
                            .close(&mut self.editor, &mut self.viewers, target);
                        match target {
                            crate::filter_bar::FilterTarget::Editor(_) => {
                                self.view_state_mut().request_focus();
                            }
                            crate::filter_bar::FilterTarget::Viewer(id) => {
                                self.activate_viewer(id);
                            }
                        }
                    }
                }
            }
        }
        let following = &self.external.follow;
        if self
            .filter
            .tick(&mut self.editor, &mut self.viewers, following, self.now)
        {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }
        if self.results.poll() {
            ui.ctx().request_repaint();
        }
        if self.results.open {
            let match_bg = crate::theme::match_bg(ui.visuals());
            let go = Panel::bottom("search-results")
                .resizable(true)
                .default_size(200.0)
                .show(ui, |ui| self.results.ui(ui, match_bg))
                .inner;
            if let Some(go) = go {
                self.go_to_result(go);
            }
        }

        self.sidebar(ui);
        self.function_list_panel(ui);
        self.document_list_panel(ui);

        self.dark = ui.visuals().dark_mode;
        if self
            .highlighting
            .frame(self.editor.buffers_mut(), &self.visible, self.dark)
        {
            ui.ctx().request_repaint();
        }
        self.visible.clear();
        let editor = &self.editor;
        self.token_styles.retain(|b| editor.buffer(b).is_some());
        let limit = self.settings.highlight_limit_mb * 1024 * 1024;
        if !self.settings.change_history {
            self.changes.clear();
        } else if self.changes.tick(&self.editor, self.now, limit) {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }

        let mut commands = Vec::new();
        let mut tab_rects = Vec::new();
        CentralPanel::no_frame().show(ui, |ui| {
            let style = crate::theme::dock_style(ui.style());
            let find_highlight = self.find.highlighter();
            let mut tabs = Tabs {
                look: self.settings.look(),
                log_rules: &self.log_rules,
                tab_width: self.settings.tab_width,
                viewers: &mut self.viewers,
                fold_margin: self.settings.fold_margin,
                word_wrap: self.settings.word_wrap,
                document_map: self.settings.document_map,
                changes: self.settings.change_history.then_some(&self.changes),
                token_styles: &self.token_styles,
                workspace_root: self.tree.as_ref().map(|t| t.root()),
                compares: &mut self.compares,
                previews: &mut self.previews,
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
                // The tabs' right-click menu is ours (`on_tab_button`):
                // egui_dock's adds a second Close after it.
                .tab_context_menus(false)
                .show_inside(ui, &mut tabs);
        });
        self.paint_tab_accents(ui, &tab_rects);
        let zoom = crate::editor_view::take_zoom_steps(ui.ctx());
        if zoom != 0 {
            self.zoom_by(zoom);
        }
        let errors: Vec<_> = self
            .view_states
            .values_mut()
            .filter_map(|s| s.last_error.take())
            .collect();
        for e in errors {
            self.note(&e);
        }
        let tab_rects: Vec<egui::Rect> = tab_rects.iter().map(|(_, r, _)| *r).collect();
        self.new_tab_on_tab_bar_double_click(ui, &tab_rects);
        if let Some((_, Tab::Editor(view))) = self.dock.find_active_focused()
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
                TabCommand::Link(crate::preview::LinkClick::Open(path)) => {
                    if path.is_file() {
                        self.open_path(&path);
                    } else {
                        self.note_text(format!("{} is not a file", path.display()));
                    }
                }
                TabCommand::Link(crate::preview::LinkClick::Refused(why)) => self.note_text(why),
                TabCommand::File(command, v) => self.tab_file_command(command, v),
                TabCommand::CloseViewer(id) => self.close_viewer(id),
                TabCommand::OpenAsEditable(id) => self.ask_open_as_editable(id),
                TabCommand::Menu(view, command) => {
                    self.activate(view);
                    match command {
                        crate::editor_menu::MenuCommand::StyleToken(style) => {
                            self.style_token(style)
                        }
                        crate::editor_menu::MenuCommand::ClearStyle(style) => {
                            let buffer = self.buffer_id();
                            self.token_styles.clear(buffer, style);
                        }
                        crate::editor_menu::MenuCommand::OpenFile(path, place) => {
                            self.open_path(&path);
                            // Only in the file itself: a large one opens in
                            // the viewer, and one that can't be read leaves
                            // the document clicked in active.
                            let wanted = Some(canonical(&path));
                            let opened = self.buffer().meta.path.as_deref().map(canonical);
                            if let Some((line, column)) = place
                                && opened == wanted
                            {
                                self.apply(Action::GoToLine { line, column });
                            }
                        }
                    }
                }
                TabCommand::CloseTab(tab) => {
                    if let Some(path) = self.dock.find_tab(&tab) {
                        self.dock.remove_tab(path);
                    }
                    if let Tab::Compare(id) = tab {
                        self.compares.remove(id, &mut self.editor);
                    }
                }
            }
        }
        self.viewer_events();

        self.autosave_tick(ui);
        self.dialogs(ui);
        self.replace_all_dialog(ui);
        self.new_entry_dialog(ui);
        self.go_to_line_dialog(ui);
        self.open_as_editable_dialog(ui);
        self.hash_dialog(ui);
        self.summary_dialog(ui);
        self.rename_dialog(ui);
        self.replace_in_files_frame(ui);
        self.edit_tools_frame(ui);
        self.external_dialog(ui);
        self.preferences_window(ui);
        if let Some((source, repeat)) = self.macros.dialogs(ui) {
            self.play_macro(source, repeat);
        }
        // When nothing has the keyboard (e.g. after a menu closes or a panel
        // button is clicked) and no dialog is up, give it back: to the find
        // field while the find panel is open, otherwise to the editor.
        let idle = self.message.is_none()
            && self.confirming.is_none()
            && self.confirm_editable.is_none()
            && !self.macros.dialog_open()
            && !self.edit_tools.dialog_open()
            && ui.memory(|m| m.focused().is_none());
        if idle && self.find.open {
            self.find.focus_field();
        } else if idle {
            match self
                .focused_viewer()
                .and_then(|id| self.viewers.get_mut(id))
            {
                Some(v) => v.state.request_focus(),
                None => self.focus_request = Some(self.active),
            }
        }
        if self.close_confirmed && !self.close_sent {
            self.close_sent = true;
            ui.ctx().send_viewport_cmd(ViewportCommand::Close);
        }

        let title = match self.focused_viewer() {
            Some(id) => format!("{} (read-only) - ochrepad", self.viewer_name(id)),
            None => format!(
                "{}{} - ochrepad",
                if self.buffer().is_dirty() { "*" } else { "" },
                self.file_name()
            ),
        };
        if title != self.title {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    /// The ochre line on each pane's active tab (egui_dock has no such
    /// option), drawn over the tab bar after the dock.
    fn paint_tab_accents(&mut self, ui: &Ui, tab_rects: &[(Tab, egui::Rect, egui::LayerId)]) {
        // Before any pane was clicked, the focused one is the active editor's.
        let focused = self
            .dock
            .find_active_focused()
            .map_or(Tab::Editor(self.active), |(_, t)| *t);
        for (_, leaf) in self.dock.iter_leaves() {
            let Some(active) = leaf.tabs.get(leaf.active.0) else {
                continue;
            };
            if let Some((_, rect, layer)) = tab_rects.iter().find(|(t, _, _)| t == active) {
                crate::theme::paint_tab_accent(
                    &ui.ctx().layer_painter(*layer),
                    *rect,
                    focused == *active,
                );
            }
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
        // egui matches shortcuts ignoring extra Shift and Alt; an extra Alt
        // means a different binding (Ctrl+Alt+F folds, Ctrl+F finds).
        let pressed = |ui: &mut Ui, s: &KeyboardShortcut| {
            ui.input_mut(|i| (s.modifiers.alt || !i.modifiers.alt) && i.consume_shortcut(s))
        };
        if pressed(ui, &NEW) {
            self.new_file();
        }
        if pressed(ui, &OPEN) {
            self.open_dialog();
        }
        let viewer = self.focused_viewer();
        if pressed(ui, &SAVE_AS) {
            if viewer.is_some() {
                self.note_text(READ_ONLY);
            } else {
                self.save_as();
            }
        }
        if pressed(ui, &SAVE) {
            if viewer.is_some() {
                self.note_text(READ_ONLY);
            } else {
                self.save();
            }
        }
        if pressed(ui, &GO_TO_LINE) {
            self.go_to_line = Some(String::new());
        }
        if pressed(ui, &FILTER_LINES) {
            self.open_filter();
        }
        if pressed(ui, &ZOOM_IN) || pressed(ui, &ZOOM_IN_EQUALS) {
            self.zoom_by(1);
        }
        if pressed(ui, &ZOOM_OUT) {
            self.zoom_by(-1);
        }
        if pressed(ui, &ZOOM_RESET) {
            self.settings.zoom = 100;
        }
        if pressed(ui, &SHOW_ALL_CHARACTERS) {
            let on = !self.settings.show_all_characters();
            self.settings.set_show_all_characters(on);
        }
        // Before Ctrl+F, which would also match Ctrl+Shift+F.
        if pressed(ui, &FIND_IN_FILES) {
            self.open_find_in_files(None);
        }
        for (shortcut, replace) in [(&FIND, false), (&REPLACE, true)] {
            if pressed(ui, shortcut) {
                match viewer {
                    Some(id) => self.open_find_on_viewer(id),
                    None => self.find.open(&self.editor, self.active, replace),
                }
            }
        }
        // Before Ctrl+F3, which would also match Ctrl+Shift+F3.
        for (shortcut, backward) in [(&SELECT_FIND_PREV, true), (&SELECT_FIND_NEXT, false)] {
            if pressed(ui, shortcut) && viewer.is_none() {
                self.select_and_find(backward);
            }
        }
        if pressed(ui, &RESTORE_CLOSED) {
            self.restore_closed_file();
        }
        for (shortcut, backward) in [(&FIND_PREV, true), (&FIND_NEXT, false)] {
            if pressed(ui, shortcut) && !self.find.pattern.is_empty() {
                match viewer {
                    Some(id) => self.viewer_find(id, backward),
                    None => {
                        self.find.find(&mut self.editor, self.active, backward);
                        self.view_state_mut().reveal_caret();
                    }
                }
            }
        }
        if pressed(ui, &RECORD_MACRO) {
            self.macros.toggle_recording(&mut self.editor);
        }
        if pressed(ui, &PLAY_MACRO) {
            self.play_macro(MacroSource::Recorded, ochrepad_core::Repeat::Times(1));
        }
        if !self.macros.dialog_open()
            && let Some(source) = self.macros.shortcut_pressed(ui)
        {
            self.play_macro(source, ochrepad_core::Repeat::Times(1));
        }
        // F8 and Shift+F8: a comparison's next or previous difference, or
        // in an editor the next or previous change since the file was saved.
        for (shortcut, backward) in [(&PREV_DIFF, true), (&NEXT_DIFF, false)] {
            if !pressed(ui, shortcut) {
                continue;
            }
            match self
                .focused_compare()
                .and_then(|id| self.compares.get_mut(id))
            {
                Some(c) if backward => c.previous_change(&mut self.editor),
                Some(c) => c.next_change(&mut self.editor),
                None if viewer.is_none() => self.go_to_change(backward),
                None => {}
            }
        }
        if pressed(ui, &PREVIEW) {
            self.open_preview();
        }
        if pressed(ui, &SIDEBAR) {
            self.settings.show_sidebar = !self.settings.show_sidebar;
        }
        if pressed(ui, &CLOSE) {
            match viewer {
                Some(id) => self.close_viewer(id),
                None => self.close_view(self.active),
            }
        }
        if pressed(ui, &PREV_TAB) {
            self.cycle_tab(false);
        }
        if pressed(ui, &NEXT_TAB) {
            self.cycle_tab(true);
        }
        // Before Ctrl+PageUp/PageDown, which an extra Shift would match too.
        if pressed(ui, &MOVE_TAB_BACKWARD) {
            self.move_tab(false);
        }
        if pressed(ui, &MOVE_TAB_FORWARD) {
            self.move_tab(true);
        }
        // Notepad++'s and browsers' other keys for the same.
        if pressed(ui, &PREV_TAB_PAGE) {
            self.cycle_tab(false);
        }
        if pressed(ui, &NEXT_TAB_PAGE) {
            self.cycle_tab(true);
        }
        self.edit_tools_shortcuts(ui);
    }

    fn menu_bar(&mut self, ui: &mut Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            let titles = [
                ui.menu_button("File", |ui| self.file_menu(ui)).response,
                ui.menu_button("Edit", |ui| self.edit_menu(ui)).response,
                ui.menu_button("Search", |ui| self.search_menu(ui)).response,
                ui.menu_button("Macro", |ui| self.macro_menu(ui)).response,
                ui.menu_button("Compare", |ui| self.compare_menu(ui))
                    .response,
                ui.menu_button("View", |ui| self.view_menu(ui)).response,
                ui.menu_button("Settings", |ui| self.settings_menu(ui))
                    .response,
            ];
            switch_menu_on_hover(ui, &titles);
        });
    }

    fn settings_menu(&mut self, ui: &mut Ui) {
        if ui.button("Preferences…").clicked() {
            self.show_preferences = true;
            ui.close();
        }
        if ui
            .button("Edit Keymap…")
            .on_hover_text(
                "Opens keymap.json; saving it applies it. Entries replace the default \
                 binding of their keys; \"command\": null removes one.",
            )
            .clicked()
        {
            self.edit_keymap();
            ui.close();
        }
        ui.separator();
        let s = &mut self.settings;
        ui.checkbox(&mut s.backups, "Periodic backup and session snapshot")
            .on_hover_text(
                "Every few seconds, unsaved text is backed up; on exit the open \
                 tabs are remembered and unsaved work comes back next time.",
            );
        ui.checkbox(&mut s.write_through, "Save files automatically")
            .on_hover_text(
                "Also save files that have a name, on the backup timer and when \
                 the window loses focus.",
            );
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
        self.recent_files_menu(ui);
        if ui
            .add_enabled(
                !self.closed_files.is_empty(),
                Button::new("Restore Recent Closed File").shortcut_text("Ctrl+Shift+T"),
            )
            .clicked()
        {
            self.restore_closed_file();
        }
        if ui.button("Open Folder…").clicked() {
            self.open_folder_dialog();
        }
        let recent = self.settings.recent_folders.clone();
        ui.add_enabled_ui(!recent.is_empty(), |ui| {
            ui.menu_button("Recent Folders", |ui| {
                for dir in recent {
                    if ui.button(dir.display().to_string()).clicked() {
                        self.open_folder(&dir);
                    }
                }
            });
        });
        if ui
            .add_enabled(self.tree.is_some(), Button::new("Close Folder"))
            .clicked()
        {
            self.close_folder();
        }
        ui.separator();
        // A viewer's file is read-only; the editor behind it is not in view.
        let viewer = self.focused_viewer();
        let editing = viewer.is_none();
        let save = |ui: &mut Ui, label: &str, shortcut: &str| {
            ui.add_enabled(editing, Button::new(label).shortcut_text(shortcut))
                .on_disabled_hover_text(READ_ONLY)
                .clicked()
        };
        if save(ui, "Save", "Ctrl+S") {
            self.save();
        }
        if save(ui, "Save As…", "Ctrl+Shift+S") {
            self.save_as();
        }
        if ui
            .add_enabled(editing, Button::new("Save a Copy As…"))
            .on_hover_text("Write the document to another file; its tab keeps its own")
            .clicked()
        {
            self.save_copy_as();
        }
        ui.add_enabled_ui(editing, |ui| {
            ui.menu_button("Export", |ui| {
                if ui
                    .button("HTML…")
                    .on_hover_text(
                        "Write the document as a web page, in the syntax theme's colours",
                    )
                    .clicked()
                {
                    self.export_html();
                }
            });
        });
        if ui.add_enabled(editing, Button::new("Save All")).clicked() {
            self.save_all();
        }
        let named = editing && self.buffer().meta.path.is_some();
        if ui
            .add_enabled(named, Button::new("Reload from Disk"))
            .clicked()
        {
            self.tab_file_command(edit_tools::TabFileCommand::Reload, self.active);
        }
        if ui.add_enabled(named, Button::new("Rename…")).clicked() {
            self.open_rename();
        }
        ui.separator();
        if item(ui, "Close", "Ctrl+W") {
            match viewer {
                Some(id) => self.close_viewer(id),
                None => self.close_view(self.active),
            }
        }
        if ui
            .add_enabled(editing, Button::new("Close All but Active"))
            .clicked()
        {
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
        if let Some(id) = self.focused_viewer() {
            // A viewer is read-only: copying is all that applies.
            if item(ui, "Copy", "Ctrl+C")
                && let Some(v) = self.viewers.get_mut(id)
            {
                v.state.copy(ui.ctx(), &v.file);
            }
            ui.separator();
            ui.label(egui::RichText::new(READ_ONLY).weak());
            return;
        }
        if item(ui, "Undo", "Ctrl+Z") {
            self.apply(Action::Undo);
        }
        if item(ui, "Redo", "Ctrl+Y") {
            self.apply(Action::Redo);
        }
        if ui
            .button("Clear Undo History")
            .on_hover_text("Forget this document's undo and redo steps, also the ones kept on disk")
            .clicked()
        {
            self.clear_undo_history();
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
        let history = crate::editor_view::clip_history(ui.ctx());
        ui.add_enabled_ui(!history.is_empty(), |ui| {
            ui.menu_button("Paste from History", |ui| {
                for (text, rect) in history {
                    if ui.button(clip_label(&text)).clicked() {
                        self.apply(if rect {
                            Action::PasteBlock(text)
                        } else {
                            Action::Paste(text)
                        });
                    }
                }
            })
            .response
            .on_hover_text("Paste something copied earlier in ochrepad");
        });
        ui.separator();
        if item(ui, "Select All", "Ctrl+A") {
            self.apply(Action::SelectAll);
        }
        let begun = self
            .editor
            .view(self.active)
            .is_some_and(|v| v.begin_select.is_some());
        for (label, column) in [
            ("Begin/End Select", false),
            ("Begin/End Select in Column Mode", true),
        ] {
            if ui
                .add(Button::new(label).selected(begun))
                .on_hover_text(if begun {
                    "Select from where you began to the caret"
                } else {
                    "Mark the caret as where a selection begins; use it again to select to the caret"
                })
                .clicked()
            {
                let view = self.active;
                let state = self.view_states.entry(view).or_default();
                crate::editor_view::begin_end_select(&mut self.editor, view, state, column);
                state.request_focus();
            }
        }
        for (title, all) in [("Multi-select All", true), ("Multi-select Next", false)] {
            ui.menu_button(title, |ui| {
                for (label, match_case, whole_word) in [
                    ("Ignore Case & Whole Word", false, false),
                    ("Match Case Only", true, false),
                    ("Match Whole Word Only", false, true),
                    ("Match Case & Whole Word", true, true),
                ] {
                    if ui.button(label).clicked() {
                        self.apply(if all {
                            Action::MultiSelectAll {
                                match_case,
                                whole_word,
                            }
                        } else {
                            Action::MultiSelectNext {
                                match_case,
                                whole_word,
                            }
                        });
                    }
                }
            });
        }
        ui.menu_button("Copy to Clipboard", |ui| self.copy_to_clipboard_menu(ui));
        ui.separator();
        // As in Notepad++, the comment commands share a submenu.
        ui.menu_button("Comment/Uncomment", |ui| {
            for (label, shortcut, action) in [
                ("Toggle Comment", "Ctrl+/", Action::ToggleLineComment),
                ("Comment Lines", "Ctrl+K", Action::CommentLines),
                ("Uncomment Lines", "Ctrl+Shift+K", Action::UncommentLines),
                (
                    "Toggle Block Comment",
                    "Ctrl+Shift+/",
                    Action::ToggleBlockComment,
                ),
            ] {
                if ui.add(Button::new(label).shortcut_text(shortcut)).clicked() {
                    self.apply(action);
                }
            }
        });
        for (label, shortcut, action) in [
            ("Indent", "Tab", Action::Indent),
            ("Unindent", "Shift+Tab", Action::Unindent),
        ] {
            if ui.add(Button::new(label).shortcut_text(shortcut)).clicked() {
                self.apply(action);
            }
        }
        ui.separator();
        ui.menu_button("Convert Case", |ui| {
            // Any seed for random case: the time will do.
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(1, |d| d.as_nanos() as u64);
            for (label, shortcut, mode) in [
                ("UPPERCASE", "Ctrl+Shift+U", CaseMode::Upper),
                ("lowercase", "Ctrl+U", CaseMode::Lower),
                ("Title Case", "Alt+U", CaseMode::Title),
                ("Title Case (blend)", "Alt+Shift+U", CaseMode::TitleBlend),
                ("Sentence case", "Ctrl+Alt+U", CaseMode::Sentence),
                (
                    "Sentence case (blend)",
                    "Ctrl+Alt+Shift+U",
                    CaseMode::SentenceBlend,
                ),
                ("iNVERT cASE", "Ctrl+Alt+I", CaseMode::Invert),
                ("ranDOm cASE", "", CaseMode::Random { seed }),
            ] {
                if item(ui, label, shortcut) {
                    self.apply(Action::ConvertCase(mode));
                }
            }
        });
        ui.menu_button("Line Operations", |ui| self.line_operations_menu(ui));
        ui.menu_button("Blank Operations", |ui| self.blank_operations_menu(ui));
        ui.menu_button("Text Tools", |ui| self.text_tools_menu(ui));
        ui.separator();
        self.column_editor_item(ui);
        ui.separator();
        let buffer = self.buffer_id();
        let mut read_only = self.buffer().read_only;
        if ui
            .checkbox(&mut read_only, "Read-Only")
            .on_hover_text("Refuse edits to this document (the file on disk is not changed)")
            .changed()
            && let Some(b) = self.editor.buffer_mut(buffer)
        {
            b.read_only = read_only;
        }
    }

    fn line_operations_menu(&mut self, ui: &mut Ui) {
        let item = |ui: &mut Ui, label: &str, shortcut: &str| {
            ui.add(Button::new(label).shortcut_text(shortcut)).clicked()
        };
        for (label, shortcut, action) in [
            ("Duplicate Current Line", "Ctrl+D", Action::DuplicateLine),
            ("Move Up Current Line", "Ctrl+Shift+Up", Action::MoveLinesUp),
            (
                "Move Down Current Line",
                "Ctrl+Shift+Down",
                Action::MoveLinesDown,
            ),
            ("Join Lines", "Ctrl+J", Action::JoinLines),
        ] {
            if item(ui, label, shortcut) {
                self.apply(action);
            }
        }
        self.more_line_operations(ui);
        ui.separator();
        for (label, action) in [
            (
                "Remove Empty Lines",
                Action::RemoveEmptyLines {
                    include_whitespace: false,
                },
            ),
            (
                "Remove Empty Lines (Containing Blank characters)",
                Action::RemoveEmptyLines {
                    include_whitespace: true,
                },
            ),
            (
                "Remove Duplicate Lines",
                Action::RemoveDuplicateLines {
                    consecutive_only: false,
                },
            ),
            (
                "Remove Consecutive Duplicate Lines",
                Action::RemoveDuplicateLines {
                    consecutive_only: true,
                },
            ),
        ] {
            if ui.button(label).clicked() {
                self.apply(action);
            }
        }
        ui.separator();
        let sort = |descending, case_insensitive, numeric, remove_duplicates| {
            Action::SortLines(SortOptions {
                descending,
                case_insensitive,
                numeric,
                remove_duplicates,
                by_length: false,
            })
        };
        let by_length = |descending| {
            Action::SortLines(SortOptions {
                descending,
                by_length: true,
                ..SortOptions::default()
            })
        };
        for (label, action) in [
            ("Sort Lines Ascending", sort(false, false, false, false)),
            ("Sort Lines Descending", sort(true, false, false, false)),
            (
                "Sort Lines Ascending (Ignore Case)",
                sort(false, true, false, false),
            ),
            (
                "Sort Lines Descending (Ignore Case)",
                sort(true, true, false, false),
            ),
            (
                "Sort Lines as Numbers Ascending",
                sort(false, false, true, false),
            ),
            (
                "Sort Lines as Numbers Descending",
                sort(true, false, true, false),
            ),
            (
                "Sort Lines and Remove Duplicates",
                sort(false, false, false, true),
            ),
            ("Sort Lines by Length Ascending", by_length(false)),
            ("Sort Lines by Length Descending", by_length(true)),
        ] {
            if ui.button(label).clicked() {
                self.apply(action);
            }
        }
        if ui.button("Randomize Line Order").clicked() {
            // Any seed: the time will do.
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(1, |d| d.as_nanos() as u64);
            self.apply(Action::ShuffleLines { seed });
        }
    }

    fn macro_menu(&mut self, ui: &mut Ui) {
        let recording = self.editor.is_recording();
        let label = if recording {
            "Stop Recording"
        } else {
            "Start Recording"
        };
        if ui
            .add(Button::new(label).shortcut_text("Ctrl+Shift+R"))
            .clicked()
        {
            self.macros.toggle_recording(&mut self.editor);
        }
        let has_recording = !self.macros.recorded.is_empty();
        if ui
            .add_enabled(
                !recording && has_recording,
                Button::new("Playback").shortcut_text("Ctrl+Shift+P"),
            )
            .clicked()
        {
            self.play_macro(MacroSource::Recorded, ochrepad_core::Repeat::Times(1));
        }
        if ui
            .add_enabled(
                !recording && has_recording,
                Button::new("Save Current Recorded Macro…"),
            )
            .clicked()
        {
            self.macros.open_save_dialog();
        }
        if ui
            .add_enabled(
                !recording && (has_recording || !self.macros.saved.is_empty()),
                Button::new("Run a Macro Multiple Times…"),
            )
            .clicked()
        {
            self.macros.open_run_dialog();
        }
        if !self.macros.saved.is_empty() {
            ui.separator();
            let saved: Vec<(String, String)> = self
                .macros
                .saved
                .iter()
                .map(|m| (m.name.clone(), m.shortcut.clone().unwrap_or_default()))
                .collect();
            for (i, (name, shortcut)) in saved.iter().enumerate() {
                if ui
                    .add_enabled(!recording, Button::new(name).shortcut_text(shortcut))
                    .clicked()
                {
                    self.play_macro(MacroSource::Saved(i), ochrepad_core::Repeat::Times(1));
                }
            }
            ui.menu_button("Delete Saved Macro", |ui| {
                for (i, (name, _)) in saved.iter().enumerate() {
                    if ui.button(name).clicked()
                        && let Err(e) = self.macros.delete(i)
                    {
                        self.message = Some(e);
                    }
                }
            });
        }
    }

    fn search_menu(&mut self, ui: &mut Ui) {
        let item = |ui: &mut Ui, label: &str, shortcut: &str| {
            ui.add(Button::new(label).shortcut_text(shortcut)).clicked()
        };
        let viewer = self.focused_viewer();
        if item(ui, "Find…", "Ctrl+F") {
            match viewer {
                Some(id) => self.open_find_on_viewer(id),
                None => self.find.open(&self.editor, self.active, false),
            }
        }
        if viewer.is_none() && item(ui, "Replace…", "Ctrl+H") {
            self.find.open(&self.editor, self.active, true);
        }
        ui.separator();
        if item(ui, "Go to Line…", "Ctrl+G") {
            self.go_to_line = Some(String::new());
        }
        if item(ui, "Filter Lines…", "Alt+Shift+L") {
            self.open_filter();
        }
        if viewer.is_none() && item(ui, "Go to Matching Bracket", "Ctrl+B") {
            self.apply(Action::GoToMatchingBracket);
        }
        if viewer.is_none() && item(ui, "Select to Matching Bracket", "Ctrl+Alt+B") {
            self.apply(Action::SelectToMatchingBracket);
        }
        ui.separator();
        if item(ui, "Find in Files…", "Ctrl+Shift+F") {
            self.open_find_in_files(None);
        }
        if viewer.is_none() {
            for (label, shortcut, backward) in [
                ("Select and Find Next", "Ctrl+F3", false),
                ("Select and Find Previous", "Ctrl+Shift+F3", true),
            ] {
                if item(ui, label, shortcut) {
                    self.select_and_find(backward);
                }
            }
        }
        for (label, shortcut, backward) in [
            ("Find Next", "F3", false),
            ("Find Previous", "Shift+F3", true),
        ] {
            if item(ui, label, shortcut) && !self.find.pattern.is_empty() {
                match viewer {
                    Some(id) => self.viewer_find(id, backward),
                    None => {
                        self.find.find(&mut self.editor, self.active, backward);
                        self.view_state_mut().reveal_caret();
                    }
                }
            }
        }
        if viewer.is_none() {
            ui.separator();
            ui.menu_button("Style All Occurrences of Token", |ui| {
                for (i, name) in crate::token_styles::NAMES.iter().enumerate() {
                    if ui.button(format!("Using {name} Style")).clicked() {
                        self.style_token(i);
                    }
                }
            });
            let buffer = self.buffer_id();
            ui.menu_button("Clear Style", |ui| {
                for (i, name) in crate::token_styles::NAMES.iter().enumerate() {
                    if ui
                        .add_enabled(
                            self.token_styles.has(buffer, i),
                            Button::new(format!("Clear {name} Style")),
                        )
                        .clicked()
                    {
                        self.token_styles.clear(buffer, Some(i));
                    }
                }
                if ui.button("Clear All Styles").clicked() {
                    self.token_styles.clear(buffer, None);
                }
            });
            for (title, backward) in [("Jump Up", true), ("Jump Down", false)] {
                ui.menu_button(title, |ui| {
                    for (i, name) in crate::token_styles::NAMES.iter().enumerate() {
                        if ui
                            .add_enabled(
                                self.token_styles.has(buffer, i),
                                Button::new(format!("{name} Style")),
                            )
                            .clicked()
                        {
                            self.jump_to_style(i, backward);
                        }
                    }
                });
            }
            ui.menu_button("Copy Styled Text", |ui| {
                for (i, name) in crate::token_styles::NAMES.iter().enumerate() {
                    if ui
                        .add_enabled(
                            self.token_styles.has(buffer, i),
                            Button::new(format!("Using {name} Style")),
                        )
                        .clicked()
                    {
                        self.copy_styled_text(ui, Some(i));
                    }
                }
                if ui.button("Using All Styles").clicked() {
                    self.copy_styled_text(ui, None);
                }
            });
            ui.menu_button("Bookmark", |ui| self.bookmark_menu(ui));
        }
    }

    /// Style All Occurrences of Token: the selection (or the word at the
    /// caret) everywhere in the document, in `style`.
    fn style_token(&mut self, style: usize) {
        let buffer = self.buffer_id();
        let Some(token) = self
            .editor
            .view(self.active)
            .and_then(|v| crate::token_styles::token_at(self.buffer().doc(), &v.selections))
        else {
            self.note_text("Select a word or some text to style");
            return;
        };
        self.token_styles.add(buffer, token, style);
    }

    /// Jump Up/Down: the previous or next occurrence of `style`'s tokens.
    fn jump_to_style(&mut self, style: usize, backward: bool) {
        let buffer = self.buffer_id();
        let Some(from) = self
            .editor
            .view(self.active)
            .map(|v| v.selections.primary().range())
        else {
            return;
        };
        let found = self
            .token_styles
            .jump(buffer, self.buffer().doc(), style, from, backward);
        if let Some(m) = found {
            self.apply(Action::SetSelections(ochrepad_core::SelectionSet::single(
                ochrepad_core::Selection::new(m.start, m.end),
            )));
        }
    }

    /// Search > Copy Styled Text: every occurrence of a style's tokens (or
    /// every style's), one per line, as in Notepad++.
    fn copy_styled_text(&mut self, ui: &Ui, style: Option<usize>) {
        let buffer = self.buffer_id();
        let found = self
            .token_styles
            .styled_text(buffer, self.buffer().doc(), style);
        if found.is_empty() {
            self.note_text("No styled text to copy");
            return;
        }
        let n = found.len();
        ui.ctx().copy_text(found.join("\n"));
        self.note_text(if n == crate::token_styles::MAX_COPIED {
            format!("Copied the first {n} styled occurrences")
        } else {
            format!("Copied {n} styled occurrences")
        });
    }

    /// Search > Bookmark, as in Notepad++.
    fn bookmark_menu(&mut self, ui: &mut Ui) {
        let item = |ui: &mut Ui, enabled: bool, label: &str, shortcut: &str| {
            let button = if shortcut.is_empty() {
                Button::new(label)
            } else {
                Button::new(label).shortcut_text(shortcut)
            };
            ui.add_enabled(enabled, button).clicked()
        };
        let marked = !self.buffer().bookmarks().is_empty();
        if item(ui, true, "Toggle Bookmark", "Ctrl+F2") {
            self.apply(Action::ToggleBookmark);
        }
        if item(ui, marked, "Next Bookmark", "F2") {
            self.apply(Action::NextBookmark);
        }
        if item(ui, marked, "Previous Bookmark", "Shift+F2") {
            self.apply(Action::PrevBookmark);
        }
        if item(ui, marked, "Clear All Bookmarks", "") {
            self.apply(Action::ClearBookmarks);
        }
        ui.separator();
        if item(ui, marked, "Cut Bookmarked Lines", "") {
            self.copy_bookmarked_lines(ui.ctx());
            self.apply(Action::RemoveBookmarkedLines);
        }
        if item(ui, marked, "Copy Bookmarked Lines", "") {
            self.copy_bookmarked_lines(ui.ctx());
        }
        if item(ui, marked, "Paste to (Replace) Bookmarked Lines", "") {
            // The clipboard comes as a paste event, which the editor sends to
            // the bookmarked lines.
            let now = self.now;
            self.view_state_mut().paste_to_bookmarks(now);
            ui.ctx().send_viewport_cmd(ViewportCommand::RequestPaste);
        }
        if item(ui, marked, "Remove Bookmarked Lines", "") {
            self.apply(Action::RemoveBookmarkedLines);
        }
        if item(ui, marked, "Remove Unmarked Lines", "") {
            self.apply(Action::RemoveUnbookmarkedLines);
        }
        if item(ui, true, "Inverse Bookmark", "") {
            self.apply(Action::InverseBookmarks);
        }
    }

    /// View > Function List: the panel on the right, kept up to date for
    /// the active document.
    fn function_list_panel(&mut self, ui: &mut Ui) {
        if !self.settings.show_function_list {
            return;
        }
        let buffer = self.buffer_id();
        let viewer = self.focused_viewer();
        let limit = self.settings.highlight_limit_mb * 1024 * 1024;
        let language = self.highlighting.language(buffer).to_owned();
        let caret_line = self
            .editor
            .view(self.active)
            .zip(self.editor.buffer(buffer))
            .map_or(0, |(v, b)| b.doc().line_of(v.selections.primary().head));
        let target = self
            .editor
            .buffer(buffer)
            .filter(|b| viewer.is_none() && b.doc().len().0 <= limit)
            .map(|b| crate::function_list::Target {
                buffer,
                revision: b.revision(),
                language: &language,
                doc: b.doc(),
                indent_width: b.indent.width,
                caret_line,
            });
        if let Some(t) = &target
            && self
                .function_list
                .tick(t, self.highlighting.registry(), self.now)
        {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        let frame = egui::Frame::new()
            .fill(ui.visuals().panel_fill)
            .inner_margin(egui::Margin::same(crate::theme::space::S as i8));
        let shown = Panel::right("function-list")
            .frame(frame)
            .resizable(true)
            .default_size(self.settings.function_list_width)
            .size_range(160.0..=480.0)
            .show(ui, |ui| self.function_list.ui(ui, target.as_ref()));
        let width = shown.response.rect.width();
        if (width - self.settings.function_list_width).abs() > 0.5 {
            self.settings.function_list_width = width;
        }
        let (go, close) = shown.inner;
        if close {
            self.settings.show_function_list = false;
        }
        if let Some(line) = go {
            self.apply(Action::GoToLine {
                line: line + 1,
                column: 1,
            });
        }
    }

    /// View > Document List: the open documents and viewers in tab order,
    /// beside the Function List. A click activates a document's tab and a
    /// middle click closes it, as they do on the tab itself.
    fn document_list_panel(&mut self, ui: &mut Ui) {
        use crate::document_list::{ListRequest, Row};
        if !self.settings.show_document_list {
            return;
        }
        let current = self
            .focused_viewer()
            .map_or(Tab::Editor(self.active), Tab::Viewer);
        let (tabs, rows): (Vec<Tab>, Vec<Row>) = self
            .dock
            .iter_all_tabs()
            .filter_map(|(_, &tab)| {
                let (title, path) = match tab {
                    Tab::Editor(view) => (
                        editor_title(&self.editor, &self.untitled, view),
                        self.editor
                            .view(view)
                            .and_then(|v| self.editor.buffer(v.buffer()))
                            .and_then(|b| b.meta.path.as_deref()),
                    ),
                    Tab::Viewer(id) => (
                        viewer_title(&self.viewers, id),
                        self.viewers.get(id).map(|v| v.file.path()),
                    ),
                    Tab::Preview(_) | Tab::Compare(_) => return None,
                };
                let row = Row {
                    title,
                    path,
                    active: tab == current,
                };
                Some((tab, row))
            })
            .unzip();
        let frame = egui::Frame::new().fill(ui.visuals().panel_fill);
        let shown = Panel::right("document-list")
            .frame(frame)
            .resizable(true)
            .default_size(self.settings.document_list_width)
            .size_range(160.0..=480.0)
            .show(ui, |ui| self.document_list.ui(ui, &rows));
        let width = shown.response.rect.width();
        if (width - self.settings.document_list_width).abs() > 0.5 {
            self.settings.document_list_width = width;
        }
        match shown.inner {
            Some(ListRequest::Activate(i)) => match tabs.get(i) {
                Some(&Tab::Editor(view)) => self.activate(view),
                Some(&Tab::Viewer(id)) => self.activate_viewer(id),
                _ => {}
            },
            Some(ListRequest::Close(i)) => match tabs.get(i) {
                Some(&Tab::Editor(view)) => self.close_view(view),
                Some(&Tab::Viewer(id)) => self.close_viewer(id),
                _ => {}
            },
            Some(ListRequest::Hide) => self.settings.show_document_list = false,
            None => {}
        }
    }

    /// Goes to the start of the next (or previous) run of lines changed since
    /// the file was saved, going round the document.
    pub fn go_to_change(&mut self, backward: bool) {
        let buffer = self.buffer_id();
        let lines = self.buffer().doc().line_count();
        let starts = self
            .changes
            .marks(buffer)
            .filter(|_| self.settings.change_history)
            .map(|m| m.starts(lines))
            .unwrap_or_default();
        if starts.is_empty() {
            self.note_text("No changes since the file was saved");
            return;
        }
        let caret = self.editor.view(self.active).map_or(0, |v| {
            self.buffer().doc().line_of(v.selections.primary().head)
        });
        let target = if backward {
            starts.iter().rev().find(|&&l| l < caret).or(starts.last())
        } else {
            starts.iter().find(|&&l| l > caret).or(starts.first())
        };
        if let Some(&line) = target {
            self.apply(Action::GoToLine {
                line: line + 1,
                column: 1,
            });
        }
    }

    /// Select and Find Next/Previous (Ctrl+F3, Ctrl+Shift+F3): finds the
    /// selected text, or the word at the caret, without opening the find
    /// panel; F3 and Shift+F3 then go on with it.
    fn select_and_find(&mut self, backward: bool) {
        let empty = self
            .editor
            .view(self.active)
            .is_some_and(|v| v.selections.primary().is_empty());
        if empty {
            let _ = self.editor.apply(self.active, Action::CollapseSelections);
            let _ = self.editor.apply(self.active, Action::SelectWord);
        }
        let Some(range) = self
            .editor
            .view(self.active)
            .map(|v| v.selections.primary().range())
        else {
            return;
        };
        if range.end.0 - range.start.0 > crate::find_panel::MAX_PATTERN {
            return;
        }
        let text = self.buffer().doc().slice(range);
        if text.is_empty() || text.contains('\n') {
            return;
        }
        self.find.pattern = if self.find.options.regex {
            ochrepad_core::search::escape(&text)
        } else {
            text
        };
        self.find.find(&mut self.editor, self.active, backward);
        // With the panel closed, its message (Not found, wrapped) goes to
        // the status bar.
        if !self.find.open
            && let Some(m) = self.find.message()
        {
            let m = m.to_owned();
            self.note_text(m);
        }
        let state = self.view_state_mut();
        state.reveal_caret();
        state.request_focus();
    }

    /// Reopens the file closed last that is not open again (Ctrl+Shift+T).
    pub fn restore_closed_file(&mut self) {
        while let Some(path) = self.closed_files.pop() {
            let open = self
                .editor
                .buffers()
                .any(|b| b.meta.path.as_deref() == Some(path.as_path()));
            if !open && path.is_file() {
                self.open_path(&path);
                return;
            }
        }
        self.note_text("No closed file to reopen");
    }

    /// Copy Bookmarked Lines: their text, each with its line break.
    fn copy_bookmarked_lines(&mut self, ctx: &egui::Context) {
        let b = self.buffer();
        let text = ochrepad_core::bookmarks::marked_text(b.doc(), b.bookmarks());
        crate::editor_view::copy_text(ctx, text);
    }

    /// Opens a comparison of two buffers in a new tab.
    pub fn compare(&mut self, left: BufferId, right: BufferId) -> CompareId {
        self.open_compare(CompareSide::Buffer(left), CompareSide::Buffer(right))
    }

    /// Compares the active buffer's file on disk (left) with the buffer.
    pub fn compare_with_saved(&mut self) -> Option<CompareId> {
        let buffer = self.buffer_id();
        let path = self.buffer().meta.path.clone()?;
        match fileio::load(&path) {
            Ok(d) => {
                let saved = CompareSide::Saved {
                    buffer,
                    doc: ochrepad_core::Document::new(&d.text),
                };
                Some(self.open_compare(saved, CompareSide::Buffer(buffer)))
            }
            Err(e) => {
                self.message = Some(format!("Could not read {}:\n{e}", path.display()));
                None
            }
        }
    }

    fn open_compare(&mut self, left: CompareSide, right: CompareSide) -> CompareId {
        let original = left.buffer();
        let compare = Compare::new(&mut self.editor, left, right);
        // A saved copy is highlighted like the file it is a copy of.
        if let Some(copy) = compare.owned_buffer() {
            let language = self.highlighting.language(original).to_owned();
            let dark = self.dark;
            if let Some(b) = self.editor.buffer_mut(copy) {
                self.highlighting.set_language(b, &language, dark);
            }
        }
        let id = self.compares.add(compare);
        self.dock.push_to_focused_leaf(Tab::Compare(id));
        id
    }

    /// The comparison shown in the focused pane, if any.
    pub fn focused_compare(&mut self) -> Option<CompareId> {
        match self.dock.find_active_focused() {
            Some((_, Tab::Compare(id))) => Some(*id),
            _ => None,
        }
    }

    fn compare_menu(&mut self, ui: &mut Ui) {
        if self.focused_viewer().is_some() {
            ui.label(egui::RichText::new("Files open in the viewer cannot be compared").weak());
            return;
        }
        let current = self.buffer_id();
        let others: Vec<(BufferId, String)> = self
            .tabs()
            .into_iter()
            .filter_map(|v| self.editor.view(v).map(|v| v.buffer()))
            .filter(|b| *b != current)
            .fold(Vec::new(), |mut acc, b| {
                if !acc.iter().any(|(x, _)| *x == b) {
                    acc.push((b, self.buffer_name(b)));
                }
                acc
            });
        ui.add_enabled_ui(!others.is_empty(), |ui| {
            ui.menu_button("Compare With", |ui| {
                for (b, name) in &others {
                    if ui.button(name).clicked() {
                        self.compare(current, *b);
                        ui.close();
                    }
                }
            });
        });
        let has_file = self.buffer().meta.path.is_some();
        if ui
            .add_enabled(has_file, Button::new("Compare With Saved Version"))
            .clicked()
        {
            self.compare_with_saved();
        }
        ui.separator();
        let focused = self.focused_compare();
        if ui
            .add_enabled(
                focused.is_some(),
                Button::new("Next Difference").shortcut_text("F8"),
            )
            .clicked()
            && let Some(c) = focused.and_then(|id| self.compares.get_mut(id))
        {
            c.next_change(&mut self.editor);
        }
        if ui
            .add_enabled(
                focused.is_some(),
                Button::new("Previous Difference").shortcut_text("Shift+F8"),
            )
            .clicked()
            && let Some(c) = focused.and_then(|id| self.compares.get_mut(id))
        {
            c.previous_change(&mut self.editor);
        }
    }

    /// Shows the active buffer's Markdown preview beside it (or switches to
    /// it if it is already open). The editor keeps the keyboard.
    pub fn open_preview(&mut self) {
        let buffer = self.buffer_id();
        if let Some(path) = self.dock.find_tab(&Tab::Preview(buffer)) {
            let _ = self.dock.set_active_tab(path);
            return;
        }
        let here = self
            .dock
            .find_tab(&Tab::Editor(self.active))
            .map(|p| NodePath {
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
                    leaf.tabs.push(Tab::Preview(buffer));
                    leaf.active = TabIndex(leaf.tabs.len() - 1);
                }
            }
            (None, Some(here)) => {
                self.dock
                    .split(here, Split::Right, 0.5, Node::leaf(Tab::Preview(buffer)));
            }
            (None, None) => self.dock.push_to_focused_leaf(Tab::Preview(buffer)),
        }
        self.activate(self.active);
    }

    /// View > Show Symbol, as in Notepad++.
    fn show_symbol_menu(&mut self, ui: &mut Ui) {
        let s = &mut self.settings;
        ui.checkbox(&mut s.show_space_tab, "Show Space and Tab");
        ui.checkbox(&mut s.show_eol, "Show End of Line");
        ui.checkbox(&mut s.show_non_printing, "Show Non-Printing Characters")
            .on_hover_text(
                "Zero-width and bidi characters, the BOM and unusual spaces, as labelled \
                 boxes. Control characters are always shown.",
            );
        let all = s.show_all_characters();
        if ui
            .add(
                Button::new("Show All Characters")
                    .selected(all)
                    .shortcut_text("Ctrl+Shift+8"),
            )
            .clicked()
        {
            s.set_show_all_characters(!all);
        }
        ui.separator();
        ui.checkbox(&mut s.show_indent_guides, "Show Indent Guide");
        ui.checkbox(&mut s.show_wrap_symbol, "Show Wrap Symbol");
    }

    /// View > Zoom: the editors' text size (not the menus').
    fn zoom_menu(&mut self, ui: &mut Ui) {
        let zoom = self.settings.zoom;
        let max = crate::settings::ZOOM_LEVELS[crate::settings::ZOOM_LEVELS.len() - 1];
        let min = crate::settings::ZOOM_LEVELS[0];
        if ui
            .add_enabled(zoom < max, Button::new("Zoom In").shortcut_text("Ctrl++"))
            .clicked()
        {
            self.zoom_by(1);
        }
        if ui
            .add_enabled(zoom > min, Button::new("Zoom Out").shortcut_text("Ctrl+-"))
            .clicked()
        {
            self.zoom_by(-1);
        }
        if ui
            .add_enabled(
                zoom != 100,
                Button::new("Restore Default Zoom").shortcut_text("Ctrl+0"),
            )
            .clicked()
        {
            self.settings.zoom = 100;
        }
    }

    /// Zooms every editor's text `steps` levels in (or out, negative).
    pub fn zoom_by(&mut self, steps: i32) {
        self.settings.zoom = crate::settings::zoom_step(self.settings.zoom, steps);
    }

    fn view_menu(&mut self, ui: &mut Ui) {
        self.window_view_items(ui);
        ui.separator();
        if ui
            .add(Button::new("Markdown Preview").shortcut_text("Ctrl+Shift+M"))
            .clicked()
        {
            self.open_preview();
        }
        if ui.button("Clone to Other View").clicked() {
            self.clone_to_other_view();
        }
        if self.focused_viewer().is_none()
            && ui
                .button("Summary…")
                .on_hover_text("The document's characters, words and lines, and its file's times")
                .clicked()
        {
            self.show_summary();
        }
        let viewer = self.focused_viewer();
        if ui
            .add_enabled(viewer.is_some(), Button::new("Open as Editable…"))
            .on_hover_text("Opens the large file shown read-only in an editor")
            .clicked()
            && let Some(id) = viewer
        {
            self.ask_open_as_editable(id);
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
        if ui
            .add(Button::new("Move Tab Forward").shortcut_text("Ctrl+Shift+PageDown"))
            .clicked()
        {
            self.move_tab(true);
        }
        if ui
            .add(Button::new("Move Tab Backward").shortcut_text("Ctrl+Shift+PageUp"))
            .clicked()
        {
            self.move_tab(false);
        }
        ui.separator();
        for (label, shortcut, action) in [
            ("Fold All", "Alt+0", Action::FoldAll),
            ("Unfold All", "Alt+Shift+0", Action::UnfoldAll),
            ("Fold Current", "Ctrl+Alt+F", Action::Fold),
            ("Unfold Current", "Ctrl+Alt+Shift+F", Action::Unfold),
        ] {
            if ui.add(Button::new(label).shortcut_text(shortcut)).clicked() {
                self.apply(action);
            }
        }
        for (label, unfold) in [("Fold Level", false), ("Unfold Level", true)] {
            ui.menu_button(label, |ui| {
                for level in 1..=8 {
                    let shortcut = if unfold {
                        format!("Alt+Shift+{level}")
                    } else {
                        format!("Alt+{level}")
                    };
                    if ui
                        .add(Button::new(level.to_string()).shortcut_text(shortcut))
                        .clicked()
                    {
                        self.apply(if unfold {
                            Action::UnfoldLevel(level)
                        } else {
                            Action::FoldLevel(level)
                        });
                    }
                }
            });
        }
        ui.checkbox(&mut self.settings.fold_margin, "Show Fold Margin");
        ui.checkbox(&mut self.settings.word_wrap, "Word Wrap");
        ui.checkbox(&mut self.settings.document_map, "Document Map");
        ui.checkbox(&mut self.settings.show_document_list, "Document List")
            .on_hover_text("A panel listing the open documents, to switch between them");
        ui.checkbox(&mut self.settings.show_function_list, "Function List")
            .on_hover_text("A panel listing the document's functions, types and headings");
        ui.checkbox(&mut self.settings.change_history, "Change History")
            .on_hover_text("Mark the lines changed since the file was saved, in the gutter");
        let buffer = self.buffer_id();
        if let Some(id) = self.focused_viewer() {
            // A viewer reads what the file gains by itself; following keeps
            // its end in view.
            if let Some(v) = self.viewers.get_mut(id) {
                ui.checkbox(&mut v.state.follow, "Follow (tail -f)")
                    .on_hover_text("Scroll to the end as the file grows");
            }
        } else {
            let mut following = self.is_following(buffer);
            if ui
                .add_enabled(
                    self.buffer().meta.path.is_some(),
                    egui::Checkbox::new(&mut following, "Follow (tail -f)"),
                )
                .on_hover_text("Add what other programs append to this file as it grows")
                .changed()
            {
                self.set_following(buffer, following);
            }
        }
        ui.menu_button("Show Symbol", |ui| self.show_symbol_menu(ui));
        ui.menu_button("Zoom", |ui| self.zoom_menu(ui));
        ui.separator();
        if ui
            .add(
                Button::new("Sidebar")
                    .selected(self.settings.show_sidebar)
                    .shortcut_text("Ctrl+Shift+E"),
            )
            .clicked()
        {
            self.settings.show_sidebar = !self.settings.show_sidebar;
        }
        if ui
            .add_enabled(
                self.tree.is_some(),
                Button::new("Reveal Active File in Sidebar"),
            )
            .clicked()
        {
            self.reveal_active_file();
        }
        ui.separator();
        let theme = &mut self.settings.theme;
        ui.radio_value(theme, ThemeChoice::System, "System theme");
        ui.radio_value(theme, ThemeChoice::Dark, "Dark mode");
        ui.radio_value(theme, ThemeChoice::Light, "Light mode");
        ui.menu_button("Syntax theme", |ui| {
            egui::ScrollArea::vertical()
                .max_height(400.0)
                .show(ui, |ui| {
                    let current = &mut self.settings.syntax_theme;
                    ui.radio_value(current, None, "Automatic (follows dark/light)");
                    for name in ochrepad_syntax::Registry::theme_names() {
                        ui.radio_value(current, Some(name.to_owned()), name);
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
        let col = byte_to_visual_col(doc, head, self.buffer().indent.width);
        format!("Ln {}, Col {}", doc.line_of(head) + 1, col.0 + 1)
    }

    /// "12 selected, 3 lines": the characters selected and the lines they
    /// are on, as in Notepad++'s status bar; with several selections, how
    /// many there are ("4 selections, 20 selected"). `None` for one caret.
    pub fn selection_status(&self) -> Option<String> {
        let v = self.editor.view(self.active)?;
        let count = v.selections.len();
        if count > 1000 {
            // Counting characters in that many every frame costs too much.
            return Some(format!("{count} selections"));
        }
        let doc = self.buffer().doc();
        let (mut chars, mut lines) = (0, 0);
        for s in v.selections.ranges().iter().filter(|s| !s.is_empty()) {
            chars += doc.to_char_idx(s.end()).0 - doc.to_char_idx(s.start()).0;
            let (first, mut last) = (doc.line_of(s.start()), doc.line_of(s.end()));
            // Whole lines selected end at the start of the next one.
            if last > first && s.end() == doc.line_start(last) {
                last -= 1;
            }
            lines += last + 1 - first;
        }
        match (count, chars, lines) {
            (1, 0, _) => None,
            (1, c, 1) => Some(format!("{c} selected")),
            (1, c, l) => Some(format!("{c} selected, {l} lines")),
            (n, 0, _) => Some(format!("{n} selections")),
            (n, c, _) => Some(format!("{n} selections, {c} selected")),
        }
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
        if let Some(id) = self.focused_viewer() {
            self.viewer_status_bar(ui, id);
            return;
        }
        let (carets, column) = self.editor.view(self.active).map_or((1, false), |v| {
            (v.selections.len(), v.selections.rect().is_some())
        });
        let meta = self.buffer().meta.clone();
        // The menus on the right are laid out first; the notes on the left
        // get what is left and are cut short with "…" in a narrow window.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            self.encoding_menu(ui);
            ui.separator();
            self.line_ending_menu(ui);
            if meta.mixed_line_endings {
                ui.separator();
                ui.add(egui::Label::new("Mixed line endings (saved as shown)").truncate())
                    .on_hover_text(
                        "The file had more than one kind of line ending. \
                         Saving converts all of them to the one shown.",
                    );
            }
            ui.separator();
            self.indent_menu(ui);
            ui.separator();
            self.language_picker(ui);
            let buffer = self.buffer_id();
            if self.is_following(buffer) {
                ui.separator();
                if ui
                    .button("Following")
                    .on_hover_text("The end of the file is followed as it grows. Click to stop.")
                    .clicked()
                {
                    self.set_following(buffer, false);
                }
            }
            if self.settings.zoom != 100 {
                ui.separator();
                if ui
                    .button(format!("{}%", self.settings.zoom))
                    .on_hover_text("Zoom. Click (or Ctrl+0) for 100%.")
                    .clicked()
                {
                    self.settings.zoom = 100;
                }
            }
            if self.editor.settings.overtype {
                ui.separator();
                if ui
                    .button("OVR")
                    .on_hover_text(
                        "Overtype: typing replaces the character after the caret. \
                         Click (or press Insert) to insert instead.",
                    )
                    .clicked()
                {
                    self.editor.settings.overtype = false;
                    self.view_state_mut().request_focus();
                }
            }
            if let Some(problem) = self.highlighting.problem(self.buffer_id()) {
                ui.separator();
                ui.add(egui::Label::new(problem).truncate());
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                if ui
                    .add(egui::Label::new(self.caret_status()).sense(egui::Sense::click()))
                    .on_hover_text("Go to Line (Ctrl+G)")
                    .clicked()
                {
                    self.go_to_line = Some(String::new());
                }
                if let Some((note, until)) = &self.status_note {
                    if self.now < *until {
                        ui.separator();
                        let color = ui.visuals().warn_fg_color;
                        ui.add(egui::Label::new(egui::RichText::new(note).color(color)).truncate());
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_secs_f64(
                                (until - self.now).max(0.0),
                            ));
                    } else {
                        self.status_note = None;
                    }
                }
                if let Some(recorded) = self.editor.recorded() {
                    ui.separator();
                    let text = format!("Recording macro ({} actions)", recorded.len());
                    let color = ui.visuals().error_fg_color;
                    ui.add(egui::Label::new(egui::RichText::new(text).color(color)).truncate());
                }
                if column {
                    ui.separator();
                    ui.add(
                        egui::Label::new(format!("Column selection, {carets} lines")).truncate(),
                    );
                } else if let Some(selected) = self.selection_status() {
                    ui.separator();
                    ui.add(egui::Label::new(selected).truncate());
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

/// Draws the tabs: one editor per view, Markdown previews and comparisons.
struct Tabs<'a> {
    look: crate::editor_view::Look,
    log_rules: &'a crate::log_rules::LogRules,
    tab_width: usize,
    viewers: &'a mut crate::viewer::Viewers,
    fold_margin: bool,
    word_wrap: bool,
    document_map: bool,
    changes: Option<&'a crate::change_history::ChangeHistory>,
    token_styles: &'a crate::token_styles::TokenStyles,
    /// The open folder, where Open File also looks.
    workspace_root: Option<&'a Path>,
    compares: &'a mut Compares,
    previews: &'a mut Previews,
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
    tab_rects: &'a mut Vec<(Tab, egui::Rect, egui::LayerId)>,
}

impl Tabs<'_> {
    /// A tab's right-click menu.
    fn tab_menu(&mut self, ui: &mut Ui, tab: Tab) {
        let view = match tab {
            Tab::Editor(view) => view,
            Tab::Viewer(id) => {
                if ui.button("Close").clicked() {
                    self.commands.push(TabCommand::CloseViewer(id));
                }
                if ui.button("Open as Editable…").clicked() {
                    self.commands.push(TabCommand::OpenAsEditable(id));
                }
                return;
            }
            Tab::Preview(_) | Tab::Compare(_) => {
                if ui.button("Close").clicked() {
                    self.commands.push(TabCommand::CloseTab(tab));
                }
                return;
            }
        };
        if ui.button("Close").clicked() {
            self.commands.push(TabCommand::Close(view));
        }
        if ui.button("Close All but This").clicked() {
            self.commands.push(TabCommand::CloseOthers(view));
        }
        if ui.button("Clone to Other View").clicked() {
            self.commands.push(TabCommand::CloneToOtherView(view));
        }
        ui.separator();
        edit_tools::tab_menu(ui, view, self.editor, self.untitled, self.commands);
    }
}

impl TabViewer for Tabs<'_> {
    type Tab = Tab;

    fn id(&mut self, tab: &mut Tab) -> Id {
        match tab {
            Tab::Editor(v) => Id::new(("ochrepad-tab", v.0)),
            Tab::Preview(b) => Id::new(("ochrepad-preview-tab", b.0)),
            Tab::Compare(c) => Id::new(("ochrepad-compare-tab", c.0)),
            Tab::Viewer(v) => Id::new(("ochrepad-viewer-tab", v.0)),
        }
    }

    fn title(&mut self, tab: &mut Tab) -> WidgetText {
        let view = match tab {
            Tab::Editor(v) => *v,
            Tab::Preview(b) => {
                return format!("Preview: {}", buffer_name(self.editor, self.untitled, *b)).into();
            }
            Tab::Compare(c) => {
                let name = |b| buffer_name(self.editor, self.untitled, b);
                return match self.compares.get(*c) {
                    Some(c) => format!(
                        "Compare: {} vs {}",
                        c.left.label(&name),
                        c.right.label(&name)
                    ),
                    None => "Compare".to_owned(),
                }
                .into();
            }
            Tab::Viewer(id) => {
                return viewer_title(self.viewers, *id).into();
            }
        };
        editor_title(self.editor, self.untitled, view).into()
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut Tab) {
        let view = match *tab {
            Tab::Editor(v) => v,
            Tab::Preview(b) => {
                if let Some(buffer) = self.editor.buffer(b)
                    && let Some(click) = self.previews.show(ui, buffer)
                {
                    self.commands.push(TabCommand::Link(click));
                }
                return;
            }
            Tab::Compare(id) => {
                let (editor, untitled) = (&*self.editor, self.untitled);
                let name = |b| buffer_name(editor, untitled, b);
                if let Some(c) = self.compares.get_mut(id) {
                    let labels = [c.left.label(&name), c.right.label(&name)];
                    c.show(
                        ui,
                        self.editor,
                        self.highlighting,
                        self.keymap,
                        labels,
                        &self.look,
                        self.find_highlight,
                    );
                }
                return;
            }
            Tab::Viewer(id) => {
                let label = format!("Viewer: {}", viewer_name(self.viewers, id));
                if let Some(v) = self.viewers.get_mut(id) {
                    let rules = v
                        .file
                        .path()
                        .file_name()
                        .is_some_and(|n| self.log_rules.applies_to(&n.to_string_lossy()))
                        .then_some(self.log_rules);
                    crate::viewer::viewer_ui(
                        ui,
                        id,
                        v,
                        &self.look,
                        self.tab_width,
                        &label,
                        self.find_highlight,
                        rules,
                    );
                }
                return;
            }
        };
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
        state.set_look(&self.look);
        state.fold_margin = self.fold_margin;
        state.word_wrap = self.word_wrap;
        state.document_map = self.document_map;
        if *self.focus_request == Some(view) {
            state.request_focus();
            *self.focus_request = None;
        }
        let mut styler = self.highlighting.styler(buffer);
        // Log highlight rules, for files they apply to. The document is
        // cloned (cheaply) because the editor is lent to the widget.
        let log_doc = self
            .editor
            .buffer(buffer)
            .filter(|b| {
                b.meta
                    .path
                    .as_deref()
                    .and_then(|p| p.file_name())
                    .is_some_and(|n| self.log_rules.applies_to(&n.to_string_lossy()))
            })
            .map(|b| b.doc().clone());
        let log_style = log_doc.as_ref().map(|doc| {
            crate::log_rules::LogStyle::new(self.log_rules, doc, ui.visuals().dark_mode)
        });
        let styled = self.token_styles.highlights(buffer, ui.visuals());
        let response = editor_view(
            ui,
            self.editor,
            view,
            state,
            self.keymap,
            &label,
            Decorations {
                token_styles: &styled,
                styler: styler.as_mut().map(|s| s as &mut dyn LineStyler),
                find_highlight: self.find_highlight,
                row_style: log_style
                    .as_ref()
                    .map(|s| s as &dyn crate::editor_view::RowStyle),
                changes: self.changes.and_then(|c| c.marks(buffer)),
            },
        );
        let mut command = None;
        let doc_dir = self
            .editor
            .buffer(buffer)
            .and_then(|b| b.meta.path.as_deref())
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        let dirs: Vec<&Path> = doc_dir
            .as_deref()
            .into_iter()
            .chain(self.workspace_root)
            .collect();
        response.context_menu(|ui| {
            command = crate::editor_menu::show(ui, self.editor, view, state, &dirs);
        });
        if let Some(command) = command {
            self.commands.push(TabCommand::Menu(view, command));
        }
    }

    fn on_tab_button(&mut self, tab: &mut Tab, response: &egui::Response) {
        self.tab_rects
            .push((*tab, response.rect, response.layer_id));
        // A middle click closes the tab, as in Notepad++ and browsers.
        if response.middle_clicked() {
            self.commands.push(match *tab {
                Tab::Editor(view) => TabCommand::Close(view),
                Tab::Viewer(id) => TabCommand::CloseViewer(id),
                other => TabCommand::CloseTab(other),
            });
        }
        // The right-click menu: ours, as egui_dock's is turned off.
        let target = *tab;
        response.context_menu(|ui| self.tab_menu(ui, target));
        // egui_dock gives tab buttons no accessibility info.
        let title = self.title(tab).text().to_owned();
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &title));
    }

    fn on_close(&mut self, tab: &mut Tab) -> OnCloseResponse {
        match tab {
            Tab::Editor(view) => {
                // The app closes it after any save prompt.
                self.commands.push(TabCommand::Close(*view));
                OnCloseResponse::Ignore
            }
            Tab::Preview(_) => OnCloseResponse::Close,
            Tab::Viewer(id) => {
                self.commands.push(TabCommand::CloseViewer(*id));
                OnCloseResponse::Ignore
            }
            Tab::Compare(id) => {
                self.compares.remove(*id, self.editor);
                OnCloseResponse::Close
            }
        }
    }

    fn allowed_in_windows(&self, _tab: &mut Tab) -> bool {
        false
    }

    fn scroll_bars(&self, _tab: &Tab) -> [bool; 2] {
        [false, false]
    }
}

/// A copied text as a menu item: its first line, cut short, and how many
/// more lines it has.
fn clip_label(text: &str) -> String {
    const MAX: usize = 40;
    let mut lines = text.lines();
    let first = lines.next().unwrap_or("").trim();
    let mut label: String = first.chars().take(MAX).collect();
    if first.chars().count() > MAX {
        label.push('…');
    }
    if label.is_empty() {
        label.push_str("(blank)");
    }
    let more = lines.count();
    if more > 0 {
        label.push_str(&format!("  (+{more} lines)"));
    }
    label
}

/// A viewer's file name.
fn viewer_name(viewers: &crate::viewer::Viewers, id: crate::viewer::ViewerId) -> String {
    viewers
        .get(id)
        .and_then(|v| v.file.path().file_name())
        .map_or_else(|| "?".to_owned(), |n| n.to_string_lossy().into_owned())
}

/// A viewer tab's title (the tab bar and the Document List).
fn viewer_title(viewers: &crate::viewer::Viewers, id: crate::viewer::ViewerId) -> String {
    format!("{} (read-only)", viewer_name(viewers, id))
}

/// An editor tab's title (the tab bar and the Document List): its
/// document's name, with " *" while it has unsaved changes and
/// " (read-only)" while it refuses edits.
fn editor_title(editor: &Editor, untitled: &HashMap<BufferId, String>, view: ViewId) -> String {
    let Some(buffer) = editor.view(view).map(|v| v.buffer()) else {
        return "?".to_owned();
    };
    let name = buffer_name(editor, untitled, buffer);
    let dirty = editor.buffer(buffer).is_some_and(Buffer::is_dirty);
    let read_only = editor.buffer(buffer).is_some_and(|b| b.read_only);
    let name = if dirty { format!("{name} *") } else { name };
    if read_only {
        format!("{name} (read-only)")
    } else {
        name
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

/// As in desktop menu bars: while one menu is open, hovering another title
/// opens that one (egui's `MenuBar` opens its menus only on click).
fn switch_menu_on_hover(ui: &Ui, titles: &[egui::Response]) {
    let ctx = ui.ctx();
    let ids: Vec<Id> = titles
        .iter()
        .map(egui::Popup::default_response_id)
        .collect();
    if !ids.iter().any(|id| egui::Popup::is_id_open(ctx, *id)) {
        return;
    }
    for (title, id) in titles.iter().zip(ids) {
        if title.hovered() && !egui::Popup::is_id_open(ctx, id) {
            egui::Popup::open_id(ctx, id);
            ctx.request_repaint();
        }
    }
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

impl OchrepadApp {
    /// Shows `error` in the status bar for a few seconds (invalid JSON in
    /// a message, with the caret moved to it).
    fn note(&mut self, error: &ochrepad_core::ActionError) {
        use ochrepad_core::ActionError;
        if self.json_error(error) {
            return;
        }
        let text = match error {
            ActionError::UnknownView(_) | ActionError::UnknownBuffer(_) => return,
            ActionError::NotFound => "Not found".to_owned(),
            e => {
                let s = e.to_string();
                let mut c = s.chars();
                c.next()
                    .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                    .unwrap_or_default()
            }
        };
        self.note_text(text);
    }

    /// Shows `text` in the status bar for a few seconds.
    pub(super) fn note_text(&mut self, text: impl Into<String>) {
        self.status_note = Some((text.into(), self.now + 4.0));
    }

    /// Ctrl+G: a line, or `line:column`, counted from 1.
    fn go_to_line_dialog(&mut self, ui: &mut Ui) {
        if self.go_to_line.is_none() {
            return;
        }
        let viewer = self.focused_viewer();
        let lines = match viewer {
            Some(id) => self.viewer_line_hint(id),
            None => format!("1–{}", self.buffer().doc().line_count()),
        };
        let Some(text) = &mut self.go_to_line else {
            return;
        };
        let resp = Modal::new(Id::new("ochrepad-go-to-line")).show(ui.ctx(), |ui| {
            ui.label(format!("Go to line ({lines}), or line:column"));
            let field = ui.add(
                egui::TextEdit::singleline(text)
                    .id(Id::new("ochrepad-go-to-line-field"))
                    .desired_width(160.0),
            );
            if !field.has_focus() && !field.lost_focus() {
                field.request_focus();
            }
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Go").clicked() || enter {
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
                let mut parts = text.trim().splitn(2, [':', ',']);
                let line = parts.next().and_then(|l| l.trim().parse::<usize>().ok());
                let column = parts
                    .next()
                    .and_then(|c| c.trim().parse::<usize>().ok())
                    .unwrap_or(1);
                if let Some(line) = line {
                    self.go_to_line = None;
                    match viewer {
                        Some(id) => self.viewer_go_to_line(id, line as u64, column),
                        None => self.apply(Action::GoToLine { line, column }),
                    }
                }
            }
            Some(false) => {
                self.go_to_line = None;
                match viewer.and_then(|id| self.viewers.get_mut(id)) {
                    Some(v) => v.state.request_focus(),
                    None => self.view_state_mut().request_focus(),
                }
            }
            None => {}
        }
    }
}
