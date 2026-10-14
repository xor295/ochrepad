//! The editor's right-click menu: Notepad++'s basics (clipboard, case,
//! comments) and the bookmark of the line clicked. The widget has already
//! moved the caret where the click was, unless it was on the selection.

use egui::{Button, Ui, ViewportCommand};
use ochrepad_core::{Action, CaseMode, Editor, ViewId};

use crate::editor_view::{EditorViewState, copy_selection};

/// Applies `action` from the menu, then gives the editor the keyboard back.
fn run(editor: &mut Editor, view: ViewId, state: &mut EditorViewState, action: Action) {
    if let Err(e) = editor.apply(view, action) {
        state.last_error = Some(e);
    }
    state.reveal_caret();
    state.request_focus();
}

/// Fills the context menu of the editor showing `view`.
pub fn show(ui: &mut Ui, editor: &mut Editor, view: ViewId, state: &mut EditorViewState) {
    let Some(buffer) = editor.view(view).map(|v| v.buffer()) else {
        return;
    };
    let Some(b) = editor.buffer(buffer) else {
        return;
    };
    let editable = !b.read_only;
    let (can_undo, can_redo) = (b.history().can_undo(), b.history().can_redo());
    let selected = editor
        .view(view)
        .is_some_and(|v| v.selections.ranges().iter().any(|s| !s.is_empty()));
    let line = state.context_line;
    let marked = line.is_some_and(|l| b.bookmarks().contains(l));

    let item = |ui: &mut Ui, enabled: bool, label: &str, shortcut: &str| {
        ui.add_enabled(enabled, Button::new(label).shortcut_text(shortcut))
            .clicked()
    };
    if item(ui, editable && can_undo, "Undo", "Ctrl+Z") {
        run(editor, view, state, Action::Undo);
    }
    if item(ui, editable && can_redo, "Redo", "Ctrl+Y") {
        run(editor, view, state, Action::Redo);
    }
    ui.separator();
    if item(ui, editable && selected, "Cut", "Ctrl+X") && copy_selection(ui.ctx(), editor, view) {
        run(editor, view, state, Action::InsertText(String::new()));
    }
    if item(ui, selected, "Copy", "Ctrl+C") {
        copy_selection(ui.ctx(), editor, view);
        state.request_focus();
    }
    if item(ui, editable, "Paste", "Ctrl+V") {
        // The integration answers with an Event::Paste, which the editor
        // (given the keyboard back) handles next frame.
        ui.ctx().send_viewport_cmd(ViewportCommand::RequestPaste);
        state.request_focus();
    }
    if item(ui, editable, "Delete", "Del") {
        run(editor, view, state, Action::DeleteForward);
    }
    if item(ui, true, "Select All", "Ctrl+A") {
        run(editor, view, state, Action::SelectAll);
    }
    ui.separator();
    if item(ui, editable, "UPPERCASE", "Ctrl+Shift+U") {
        run(editor, view, state, Action::ConvertCase(CaseMode::Upper));
    }
    if item(ui, editable, "lowercase", "Ctrl+U") {
        run(editor, view, state, Action::ConvertCase(CaseMode::Lower));
    }
    if item(ui, editable, "Toggle Comment", "Ctrl+/") {
        run(editor, view, state, Action::ToggleLineComment);
    }
    if item(ui, editable, "Toggle Block Comment", "Ctrl+Shift+/") {
        run(editor, view, state, Action::ToggleBlockComment);
    }
    ui.separator();
    let label = if marked {
        "Remove Bookmark"
    } else {
        "Add Bookmark"
    };
    if item(ui, line.is_some(), label, "Ctrl+F2")
        && let Some(line) = line
        && let Some(b) = editor.buffer_mut(buffer)
    {
        b.toggle_bookmark(line);
        state.request_focus();
    }
}
