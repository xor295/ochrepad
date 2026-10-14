//! The editor's right-click menu: Notepad++'s basics (clipboard, case,
//! comments) and the bookmark of the line clicked. The widget has already
//! moved the caret where the click was, unless it was on the selection.

use std::path::{Path, PathBuf};

use egui::{Button, Ui, ViewportCommand};
use ochrepad_core::{Action, CaseMode, Document, Editor, SelectionSet, ViewId};

use crate::editor_view::{EditorViewState, copy_selection};

/// Applies `action` from the menu, then gives the editor the keyboard back.
fn run(editor: &mut Editor, view: ViewId, state: &mut EditorViewState, action: Action) {
    if let Err(e) = editor.apply(view, action) {
        state.last_error = Some(e);
    }
    state.reveal_caret();
    state.request_focus();
}

/// What the menu asks of the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuCommand {
    /// Style All Occurrences of Token with this style.
    StyleToken(usize),
    /// Clear this style, or every one.
    ClearStyle(Option<usize>),
    /// Open a file named in the text, at a line and column if it said.
    OpenFile(PathBuf, Option<(usize, usize)>),
}

/// A file named in the text, and the line and column it gave.
pub type FileAt = (PathBuf, Option<(usize, usize)>);

/// The file the selection, or the text around the caret, names, if it is
/// there: a path as such (on this machine), or relative to one of `dirs`
/// (the document's folder, the open folder). A `:line` or `:line:column`
/// after it, as in a compiler's or a log's messages, is the place to go to.
pub fn file_at(doc: &Document, sels: &SelectionSet, dirs: &[&Path]) -> Option<FileAt> {
    let sel = sels.primary();
    let text = if sel.is_empty() {
        // The run of characters a path can have around the caret.
        let line = doc.line_of(sel.head);
        let start = doc.line_start(line);
        let text = doc.line_text(line);
        let at = sel.head.0 - start.0;
        let stop = |c: char| c.is_whitespace() || "\"'<>()[]{},;|`".contains(c);
        let from = text[..at].rfind(stop).map_or(0, |i| i + 1);
        let to = text[at..].find(stop).map_or(text.len(), |i| at + i);
        text[from..to].to_owned()
    } else {
        doc.slice(sel.range())
    };
    let text = text.trim().trim_end_matches(['.', ',', ':']);
    if text.is_empty() || text.contains('\n') || text.len() > 1000 {
        return None;
    }
    // "path:12" or "path:12:5"; a drive letter's colon is not followed by
    // digits.
    let mut place = None;
    let mut path = text;
    let mut parts = text.rsplitn(3, ':');
    let last = parts.next();
    let middle = parts.next();
    let first = parts.next();
    let number = |s: Option<&str>| {
        s.filter(|s| !s.is_empty())
            .and_then(|s| s.parse::<usize>().ok())
    };
    match (number(middle), number(last), first) {
        (Some(line), Some(col), Some(p)) if !p.is_empty() => {
            place = Some((line, col));
            path = p;
        }
        _ => {
            let mut two = text.rsplitn(2, ':');
            let (last, first) = (two.next(), two.next());
            if let (Some(line), Some(p)) = (number(last), first)
                && !p.is_empty()
            {
                place = Some((line, 1));
                path = p;
            }
        }
    }
    let candidate = Path::new(path);
    let found = if candidate.is_absolute() {
        // Never a network path: only looking at `\\host\share\...` sends
        // Windows' credentials to that host.
        (crate::safe_images::is_local_path(candidate) && candidate.is_file())
            .then(|| candidate.to_path_buf())
    } else {
        dirs.iter().map(|d| d.join(candidate)).find(|p| p.is_file())
    }?;
    Some((found, place))
}

/// Fills the context menu of the editor showing `view`. `dirs` are where a
/// relative path in the text is looked for (Open File).
pub fn show(
    ui: &mut Ui,
    editor: &mut Editor,
    view: ViewId,
    state: &mut EditorViewState,
    dirs: &[&Path],
) -> Option<MenuCommand> {
    let mut command = None;
    let buffer = editor.view(view).map(|v| v.buffer())?;
    let b = editor.buffer(buffer)?;
    let editable = !b.read_only;
    let (can_undo, can_redo) = (b.history().can_undo(), b.history().can_redo());
    let selected = editor
        .view(view)
        .is_some_and(|v| v.selections.ranges().iter().any(|s| !s.is_empty()));
    let line = state.context_line;
    let marked = line.is_some_and(|l| b.bookmarks().contains(l));
    // Looked for once, when the menu opens: it is drawn every frame.
    let file = state
        .context_file
        .get_or_insert_with(|| {
            editor
                .view(view)
                .and_then(|v| file_at(b.doc(), &v.selections, dirs))
        })
        .clone();

    let item = |ui: &mut Ui, enabled: bool, label: &str, shortcut: &str| {
        ui.add_enabled(enabled, Button::new(label).shortcut_text(shortcut))
            .clicked()
    };
    if let Some((path, place)) = file {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let label = match place {
            Some((line, _)) => format!("Open {name} at Line {line}"),
            None => format!("Open {name}"),
        };
        if ui
            .button(label)
            .on_hover_text(path.display().to_string())
            .clicked()
        {
            command = Some(MenuCommand::OpenFile(path, place));
        }
        ui.separator();
    }
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
    ui.menu_button("Style All Occurrences of Token", |ui| {
        for (i, name) in crate::token_styles::NAMES.iter().enumerate() {
            if ui.button(format!("Using {name} Style")).clicked() {
                command = Some(MenuCommand::StyleToken(i));
            }
        }
    });
    ui.menu_button("Clear Style", |ui| {
        for (i, name) in crate::token_styles::NAMES.iter().enumerate() {
            if ui.button(format!("Clear {name} Style")).clicked() {
                command = Some(MenuCommand::ClearStyle(Some(i)));
            }
        }
        if ui.button("Clear All Styles").clicked() {
            command = Some(MenuCommand::ClearStyle(None));
        }
    });
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
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use ochrepad_core::{ByteOffset, Selection};

    fn at(text: &str, caret: usize, dirs: &[&Path]) -> Option<FileAt> {
        let doc = Document::new(text);
        file_at(&doc, &SelectionSet::caret(ByteOffset(caret)), dirs)
    }

    #[test]
    fn paths_in_the_text_with_a_line_and_column() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        let main = dir.path().join("src").join("main.rs");
        std::fs::write(&main, "fn main() {}").unwrap();
        let dirs = [dir.path()];
        let line = "error: src/main.rs:12:5: oops";
        assert_eq!(at(line, 10, &dirs), Some((main.clone(), Some((12, 5)))));
        assert_eq!(
            at("see (src/main.rs:3).", 8, &dirs),
            Some((main.clone(), Some((3, 1))))
        );
        assert_eq!(
            at("  File \"src/main.rs\", line 4", 12, &dirs),
            Some((main.clone(), None))
        );
        // An absolute path needs no folder.
        let abs = main.display().to_string();
        assert_eq!(at(&abs, 2, &[]), Some((main.clone(), None)));
        // Nothing there, or no such file.
        assert_eq!(at("no path here", 3, &dirs), None);
        assert_eq!(at("src/missing.rs:1", 3, &dirs), None);
        // A selection is taken as it is.
        let doc = Document::new("x src/main.rs y");
        let sel = SelectionSet::single(Selection::new(ByteOffset(2), ByteOffset(13)));
        assert_eq!(file_at(&doc, &sel, &dirs), Some((main, None)));
    }

    /// Only looking at a network path sends Windows' credentials to its
    /// host, so a path in the text never makes the menu look at one, even
    /// for a file that is there.
    #[cfg(windows)]
    #[test]
    fn network_paths_are_never_looked_at() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.txt");
        std::fs::write(&file, "x").unwrap();
        let local = file.display().to_string();
        assert!(at(&local, 2, &[]).is_some());
        // The same file through the drive's administrative share.
        let (drive, rest) = local.split_once(":\\").unwrap();
        for path in [
            format!(r"\\localhost\{drive}$\{rest}"),
            format!("//localhost/{drive}$/{}", rest.replace('\\', "/")),
            format!(r"\\?\UNC\localhost\{drive}$\{rest}"),
        ] {
            assert_eq!(at(&path, 3, &[]), None, "{path}");
        }
    }
}
