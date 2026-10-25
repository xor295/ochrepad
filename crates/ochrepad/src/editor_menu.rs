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

/// The bytes of `text` around byte `at`, up to the characters `stop`
/// matches on either side (which can be wider than a byte, as a no-break
/// space is).
fn run_around(text: &str, at: usize, stop: impl Fn(char) -> bool) -> (usize, usize) {
    let from = text[..at]
        .char_indices()
        .rev()
        .find(|&(_, c)| stop(c))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let to = text[at..].find(&stop).map_or(text.len(), |i| at + i);
    (from, to)
}

/// The web link (`http://` or `https://`) the selection is, or the caret
/// is in. Other kinds of link (`file:`, `javascript:` and the like) are
/// not offered.
pub fn url_at(doc: &Document, sels: &SelectionSet) -> Option<String> {
    let sel = sels.primary();
    if !sel.is_empty() {
        let text = doc.slice(sel.range());
        let text = text.trim();
        return (starts_link(text) && text.len() <= 4096 && !text.contains(char::is_whitespace))
            .then(|| text.to_owned());
    }
    let line = doc.line_of(sel.head);
    let text = doc.line_text(line);
    let at = sel.head.0 - doc.line_start(line).0;
    url_in(&text, at).map(|(_, link)| link)
}

/// Whether `s` starts with `http://` or `https://` and something after it.
fn starts_link(s: &str) -> bool {
    const SCHEMES: [&str; 2] = ["https://", "http://"];
    SCHEMES
        .iter()
        .any(|p| s.len() > p.len() && s.get(..p.len()).is_some_and(|h| h.eq_ignore_ascii_case(p)))
}

/// The web link around byte `at` of one line of `text`: its bytes, and
/// the link.
pub fn url_in(text: &str, at: usize) -> Option<(std::ops::Range<usize>, String)> {
    // A link runs between blanks and quotes; brackets can be part of it.
    let (from, to) = run_around(text, at, |c| c.is_whitespace() || "\"'<>`".contains(c));
    let run = &text[from..to];
    // The last link start at or before the caret ("url=https://...").
    let caret = at - from;
    let begin = run
        .char_indices()
        .rev()
        .map(|(i, _)| i)
        .find(|&i| i <= caret && starts_link(&run[i..]))?;
    let mut link = &run[begin..];
    if link.len() > 4096 {
        return None;
    }
    // Punctuation after a link ends the sentence, and a closing bracket
    // without its opening one closes the text around it. Each bracket is
    // counted once (a run of closers is common, and recounting is slow).
    const PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];
    let mut open = [0usize; 3];
    let mut close = [0usize; 3];
    for c in link.chars() {
        for (k, (o, cl)) in PAIRS.iter().enumerate() {
            open[k] += usize::from(c == *o);
            close[k] += usize::from(c == *cl);
        }
    }
    while let Some(last) = link.chars().next_back() {
        let closer = PAIRS.iter().position(|p| p.1 == last);
        let trim = match closer {
            Some(k) => close[k] > open[k],
            None => matches!(last, '.' | ',' | ';' | ':' | '!' | '?'),
        };
        if !trim {
            break;
        }
        if let Some(k) = closer {
            close[k] -= 1;
        }
        link = &link[..link.len() - last.len_utf8()];
    }
    (starts_link(link) && caret <= begin + link.len())
        .then(|| (from + begin..from + begin + link.len(), link.to_owned()))
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
        text[path_run(&text, at)].to_owned()
    } else {
        doc.slice(sel.range())
    };
    file_named(&text, dirs)
}

/// The bytes of one line of `text` around byte `at` that a path can be
/// made of.
pub fn path_run(text: &str, at: usize) -> std::ops::Range<usize> {
    let stop = |c: char| c.is_whitespace() || "\"'<>()[]{},;|`".contains(c);
    let (from, to) = run_around(text, at, stop);
    from..to
}

/// The file `text` names, if it is there: see [`file_at`].
pub fn file_named(text: &str, dirs: &[&Path]) -> Option<FileAt> {
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
    // Never a network path, written as one or reached through a link: only
    // looking at `\\host\share\...` sends Windows' credentials to that host
    // (and can hold the frame up while it does not answer).
    let local = |p: &Path| crate::safe_images::is_local_path(p) && p.is_file();
    let found = if candidate.is_absolute() {
        local(candidate).then(|| candidate.to_path_buf())
    } else {
        dirs.iter().map(|d| d.join(candidate)).find(|p| local(p))
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
    let url = state
        .context_url
        .get_or_insert_with(|| {
            editor
                .view(view)
                .and_then(|v| url_at(b.doc(), &v.selections))
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
    if let Some(url) = url {
        if ui.button("Open Link").on_hover_text(&url).clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
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

    /// A long run of closing brackets after a link is trimmed in one pass
    /// (it took seconds when each one recounted the link), and a link
    /// longer than 4096 bytes is not offered.
    #[test]
    fn many_closing_brackets_are_trimmed_quickly() {
        let url = |text: &str| url_at(&Document::new(text), &SelectionSet::caret(ByteOffset(3)));
        let t = std::time::Instant::now();
        assert_eq!(
            url(&format!("https://a.b/{}", ")".repeat(4000))),
            Some("https://a.b/".to_owned())
        );
        assert_eq!(url(&format!("https://a.b/{}", ")".repeat(200_000))), None);
        if !cfg!(debug_assertions) {
            assert!(t.elapsed() < std::time::Duration::from_secs(2));
        }
    }

    /// Spaces wider than a byte (no-break, ideographic) end a run too.
    #[test]
    fn a_wide_space_before_the_caret_is_one_character() {
        assert_eq!(at("a\u{a0}b", 3, &[]), None);
        assert_eq!(at("see\u{3000}main.rs", 8, &[]), None);
        let url = |text: &str, caret: usize| {
            url_at(
                &Document::new(text),
                &SelectionSet::caret(ByteOffset(caret)),
            )
        };
        assert_eq!(
            url("voir\u{a0}https://a.b/c\u{a0}!", 9),
            Some("https://a.b/c".to_owned())
        );
        assert_eq!(url("\u{3000}x", 3), None);
    }

    #[test]
    fn web_links_around_the_caret() {
        let url = |text: &str, caret: usize| {
            url_at(
                &Document::new(text),
                &SelectionSet::caret(ByteOffset(caret)),
            )
        };
        let some = |s: &str| Some(s.to_owned());
        assert_eq!(
            url("see https://example.com/a?b=1. Then", 10),
            some("https://example.com/a?b=1")
        );
        // Brackets of its own stay; the ones around it go.
        assert_eq!(
            url("(https://en.wikipedia.org/wiki/Rust_(language))", 5),
            some("https://en.wikipedia.org/wiki/Rust_(language)")
        );
        assert_eq!(url("<http://x.org/>", 3), some("http://x.org/"));
        assert_eq!(url("[HTTPS://X.ORG]", 3), some("HTTPS://X.ORG"));
        assert_eq!(url("url=https://a.b/c", 10), some("https://a.b/c"));
        // At its end, and before its start.
        assert_eq!(url("go https://a.b/c", 16), some("https://a.b/c"));
        assert_eq!(url("url=https://a.b/c", 2), None);
        // Only web links.
        assert_eq!(url("ftp://x.org/f", 3), None);
        assert_eq!(url("javascript:alert(1)", 3), None);
        assert_eq!(url("file:///C:/x", 3), None);
        assert_eq!(url("https://", 3), None);
        assert_eq!(url("no link here", 3), None);
        // A selected link, as it is.
        let doc = Document::new("x https://a.b y");
        let sel = SelectionSet::single(Selection::new(ByteOffset(2), ByteOffset(13)));
        assert_eq!(url_at(&doc, &sel), some("https://a.b"));
        let sel = SelectionSet::single(Selection::new(ByteOffset(0), ByteOffset(13)));
        assert_eq!(url_at(&doc, &sel), None);
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
