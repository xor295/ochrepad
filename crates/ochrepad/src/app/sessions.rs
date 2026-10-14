//! File > Save Session… and Load Session…, as in Notepad++: a file of the
//! user's that lists the open files (each with its caret and first line on
//! screen), the active one and the open folder, to open them together
//! again later. It is separate from the session ochrepad restores by itself
//! (`autosave.rs`).
//!
//! A session file may come from someone else. It is read only if it is
//! small, each entry is read on its own, and only files on this computer
//! are opened: on Windows a network path (`\\host\share\x`) would send the
//! user's credentials to that host. Writing it goes through the worker, as
//! for every user file.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ochrepad_core::{Action, ByteOffset, Document, FileMeta, SelectionSet};
use ochrepad_fileio as fileio;
use ochrepad_fileio::worker::Job;
use serde::{Deserialize, Serialize};

use super::autosave::PendingJob;
use super::{OchrepadApp, Tab};

/// The format Save Session writes. A file in a later one is refused rather
/// than half understood.
const SESSION_VERSION: u32 = 1;

/// Session files larger than this are not read.
const MAX_SESSION_FILE: u64 = 1024 * 1024;

/// A session file from someone else could hold any number of entries, of
/// any length: Load Session's message lists at most this many problems,
const MAX_LISTED: usize = 20;
/// and lines that quote the file are cut to this many characters.
const MAX_LINE: usize = 300;

/// A session file. As written, each entry is a [`SessionEntry`]; as read by
/// [`parse_session`], an entry or why it is not one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionFile<F = SessionEntry> {
    /// The format, 1. Files in a later one are refused.
    pub version: u32,
    /// The open files, in tab order.
    #[serde(default)]
    pub files: Vec<F>,
    /// Index into `files` of the active one.
    #[serde(default)]
    pub active: Option<usize>,
    /// The open folder.
    #[serde(default)]
    pub folder: Option<PathBuf>,
}

/// A file in a session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionEntry {
    /// A relative path is taken from the session file's folder.
    pub path: PathBuf,
    /// The primary caret, a byte offset into the text.
    #[serde(default)]
    pub caret: usize,
    /// The first line on screen, counted from 0.
    #[serde(default)]
    pub first_line: usize,
}

/// A session file as read: each entry, or why it is not a file.
pub type ParsedSession = SessionFile<Result<SessionEntry, String>>;

/// Reads a session file's JSON. Each entry is read on its own, so one that
/// is not a file (no path, a caret that is not a number) is kept as the
/// reason and does not stop the others.
pub fn parse_session(bytes: &[u8]) -> Result<ParsedSession, String> {
    // Some editors start a UTF-8 file with a BOM.
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    // serde's message may quote the file.
    let file: SessionFile<serde_json::Value> = serde_json::from_slice(bytes)
        .map_err(|e| shorten(format!("it is not a session file ({e})")))?;
    if file.version > SESSION_VERSION {
        return Err(format!(
            "it was saved by a newer version of ochrepad (session format {})",
            file.version
        ));
    }
    Ok(SessionFile {
        version: file.version,
        files: file
            .files
            .into_iter()
            .map(|entry| serde_json::from_value(entry).map_err(|e| e.to_string()))
            .collect(),
        active: file.active,
        folder: file.folder,
    })
}

/// Reads the session file at `file`, if it is small enough to be one.
fn read_session(file: &Path) -> Result<ParsedSession, String> {
    let bytes = match fileio::read_file_up_to(file, MAX_SESSION_FILE) {
        Ok(bytes) => bytes,
        Err(fileio::LoadError::TooLarge(_)) => {
            return Err(format!(
                "it is over {} MB, too large for a session file",
                MAX_SESSION_FILE / (1024 * 1024)
            ));
        }
        Err(e) => return Err(e.to_string()),
    };
    parse_session(&bytes)
}

/// `path` from a session file in `dir`, where a relative path starts; or,
/// for a path that is not on this computer, the line saying so.
fn resolve(dir: &Path, path: &Path) -> Result<PathBuf, String> {
    let joined = dir.join(path);
    // As a file dialog gives it (on Windows also without `..`), and checked
    // as it will be opened.
    let path = std::path::absolute(&joined).unwrap_or(joined);
    if !crate::safe_images::is_local_path(&path) {
        return Err(format!("{}: not on this computer", path.display()));
    }
    Ok(path)
}

/// The lines of Load Session's message, at most [`MAX_LISTED`] of them.
fn problem_list(problems: Vec<String>) -> String {
    let more = problems.len().saturating_sub(MAX_LISTED);
    let mut lines: Vec<String> = problems.into_iter().take(MAX_LISTED).map(shorten).collect();
    if more > 0 {
        lines.push(format!("and {more} more"));
    }
    lines.join("\n")
}

/// `line` cut to [`MAX_LINE`] characters.
fn shorten(line: String) -> String {
    match line.char_indices().nth(MAX_LINE) {
        Some((end, _)) => format!("{}…", &line[..end]),
        None => line,
    }
}

/// A file's name, for the status bar.
fn name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

impl OchrepadApp {
    /// File > Save Session…: asks where to save the open files as a
    /// session, suggesting `session.json`.
    pub fn save_session(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Session", &["json"])
            .set_file_name("session.json")
            .save_file()
        else {
            return;
        };
        self.save_session_to(&path);
    }

    /// Writes the open files to `path` as a session file (Save Session…
    /// without the dialog); the worker writes it. Documents never saved
    /// have no file to list: the status bar says how many were left out.
    pub fn save_session_to(&mut self, path: &Path) {
        let (session, unnamed) = self.session_file();
        let json = match serde_json::to_string_pretty(&session) {
            Ok(json) => json,
            Err(e) => {
                self.message = Some(format!("Could not save the session:\n{e}"));
                return;
            }
        };
        // UTF-8 without a BOM, with LF line breaks, replaced atomically.
        let tag = self.persist.worker.submit(Job::Save {
            path: path.to_path_buf(),
            doc: Document::new(&(json + "\n")),
            meta: FileMeta::default(),
        });
        self.persist.pending.insert(
            tag,
            PendingJob::Session {
                path: path.to_path_buf(),
                files: session.files.len(),
                unnamed,
            },
        );
    }

    /// The open files as a session file, and how many documents were left
    /// out for having no file.
    fn session_file(&mut self) -> (SessionFile, usize) {
        let viewer = self.focused_viewer();
        let active_buffer = self.editor.view(self.active).map(|v| v.buffer());
        let mut session = SessionFile {
            version: SESSION_VERSION,
            files: Vec::new(),
            active: None,
            folder: self.tree.as_ref().map(|t| t.root().to_path_buf()),
        };
        let mut seen = HashSet::new();
        let mut unnamed = 0;
        for (_, tab) in self.dock.iter_all_tabs() {
            let entry = match *tab {
                Tab::Editor(view) => {
                    let Some(v) = self.editor.view(view) else {
                        continue;
                    };
                    let Some(b) = self.editor.buffer(v.buffer()) else {
                        continue;
                    };
                    // A second view of a document adds nothing.
                    if !seen.insert(b.id()) {
                        continue;
                    }
                    let Some(path) = b.meta.path.clone() else {
                        unnamed += 1;
                        continue;
                    };
                    if viewer.is_none() && active_buffer == Some(b.id()) {
                        session.active = Some(session.files.len());
                    }
                    SessionEntry {
                        path,
                        caret: v.selections.primary().head.0,
                        first_line: v.scroll.first_line,
                    }
                }
                Tab::Viewer(id) => {
                    let Some(v) = self.viewers.get(id) else {
                        continue;
                    };
                    if viewer == Some(id) {
                        session.active = Some(session.files.len());
                    }
                    // A viewer's caret is a line and column, not an offset
                    // into the text.
                    SessionEntry {
                        path: v.file.path().to_path_buf(),
                        caret: 0,
                        first_line: usize::try_from(v.state.top_line()).unwrap_or(usize::MAX),
                    }
                }
                Tab::Preview(_) | Tab::Compare(_) => continue,
            };
            // A file opened from the command line may have a relative path,
            // which loading would take from the session file's folder.
            let path = std::path::absolute(&entry.path).unwrap_or(entry.path);
            session.files.push(SessionEntry { path, ..entry });
        }
        (session, unnamed)
    }

    /// The worker finished writing a session file.
    pub(super) fn session_saved(
        &mut self,
        path: &Path,
        files: usize,
        unnamed: usize,
        result: Result<(), String>,
    ) {
        if let Err(e) = result {
            self.message = Some(format!(
                "Could not save the session to {}:\n{e}",
                path.display()
            ));
            return;
        }
        let mut note = format!(
            "Saved {} ({})",
            name(path),
            crate::search_results::files(files)
        );
        if unnamed > 0 {
            note.push_str(&format!(
                "; {unnamed} new {} without a name left out",
                if unnamed == 1 {
                    "document"
                } else {
                    "documents"
                }
            ));
        }
        self.note_text(note);
    }

    /// File > Load Session…: asks for a session file and opens its files.
    pub fn load_session(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Session", &["json"])
            .pick_file()
        else {
            return;
        };
        self.load_session_from(&path);
    }

    /// Opens the files the session file `file` lists (Load Session…
    /// without the dialog), in its order and beside the open ones, each as
    /// `open_path` opens it (one already open is switched to, a large one
    /// goes to the viewer), with its caret and first line back. The folder
    /// opens too, and the session's active document is activated. What
    /// could not be opened is listed in one message; nothing is closed.
    pub fn load_session_from(&mut self, file: &Path) {
        let file = std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf());
        let session = match read_session(&file) {
            Ok(session) => session,
            Err(e) => {
                self.message = Some(format!(
                    "Could not load the session {}:\n{e}",
                    file.display()
                ));
                return;
            }
        };
        let dir = file.parent().unwrap_or(Path::new(""));
        let mut problems = Vec::new();
        if let Some(folder) = &session.folder {
            match resolve(dir, folder) {
                Ok(folder) if folder.is_dir() => self.open_folder(&folder),
                Ok(folder) => problems.push(format!("{}: no such folder", folder.display())),
                Err(e) => problems.push(e),
            }
        }
        let (mut opened, mut active) = (0, None);
        for (i, entry) in session.files.into_iter().enumerate() {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    problems.push(format!("Entry {}: {e}", i + 1));
                    continue;
                }
            };
            let path = match resolve(dir, &entry.path) {
                Ok(path) => path,
                Err(e) => {
                    problems.push(e);
                    continue;
                }
            };
            match self.open_tab(&path) {
                Ok(tab) => {
                    self.put_back(tab, &entry);
                    opened += 1;
                    if session.active == Some(i) {
                        active = Some(tab);
                    }
                }
                Err(e) => problems.push(format!("{}: {e}", path.display())),
            }
        }
        match active {
            Some(Tab::Editor(view)) => self.activate(view),
            Some(Tab::Viewer(id)) => self.activate_viewer(id),
            _ => {}
        }
        self.note_text(format!(
            "Loaded {} ({})",
            name(&file),
            crate::search_results::files(opened)
        ));
        if !problems.is_empty() {
            self.message = Some(format!(
                "Some files in {} could not be opened:\n{}",
                name(&file),
                problem_list(problems)
            ));
        }
    }

    /// Puts a document just opened from a session where it was: its caret
    /// (which `SetSelections` clamps into the text and onto a char
    /// boundary) and its first line on screen.
    fn put_back(&mut self, tab: Tab, entry: &SessionEntry) {
        match tab {
            Tab::Editor(view) => {
                let caret = SelectionSet::caret(ByteOffset(entry.caret));
                let _ = self.editor.apply(view, Action::SetSelections(caret));
                let lines = self
                    .editor
                    .view(view)
                    .and_then(|v| self.editor.buffer(v.buffer()))
                    .map_or(1, |b| b.doc().line_count());
                if let Some(v) = self.editor.view_mut(view) {
                    v.scroll.first_line = entry.first_line.min(lines.saturating_sub(1));
                    v.scroll.first_sub = 0;
                    v.scroll.line_fraction = 0.0;
                }
                // Drawn with the caret in view, as a new tab is.
                if let Some(state) = self.view_states.get_mut(&view) {
                    state.reveal_caret();
                }
            }
            Tab::Viewer(id) => {
                if let Some(v) = self.viewers.get_mut(id) {
                    v.state.scroll_to_line(entry.first_line as u64);
                }
            }
            Tab::Preview(_) | Tab::Compare(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, caret: usize, first_line: usize) -> SessionEntry {
        SessionEntry {
            path: PathBuf::from(path),
            caret,
            first_line,
        }
    }

    #[test]
    fn a_written_session_reads_back() {
        let session = SessionFile {
            version: SESSION_VERSION,
            files: vec![entry("a.txt", 3, 10), entry("b.txt", 0, 0)],
            active: Some(1),
            folder: Some(PathBuf::from("project")),
        };
        let json = serde_json::to_vec_pretty(&session).unwrap();
        let read = parse_session(&json).unwrap();
        assert_eq!(
            read.files,
            vec![Ok(entry("a.txt", 3, 10)), Ok(entry("b.txt", 0, 0))]
        );
        assert_eq!(read.active, Some(1));
        assert_eq!(read.folder, Some(PathBuf::from("project")));
    }

    #[test]
    fn an_entry_that_is_not_a_file_does_not_stop_the_others() {
        let json = br#"{"version": 1, "files": [
            {"path": "a.txt"}, {"caret": 2}, {"path": 42}, "b.txt", {"path": "c.txt", "caret": -1}
        ]}"#;
        let read = parse_session(json).unwrap();
        // The caret and first line may be left out.
        assert_eq!(read.files[0], Ok(entry("a.txt", 0, 0)));
        assert!(read.files[1].as_ref().is_err_and(|e| e.contains("path")));
        assert!(read.files[2..].iter().all(Result::is_err), "{read:?}");
        assert_eq!((read.active, read.folder), (None, None));
    }

    #[test]
    fn other_files_and_later_formats_are_refused() {
        for json in [&b"{ not json"[..], b"[]", br#"{"files": []}"#, b""] {
            let e = parse_session(json).unwrap_err();
            assert!(e.contains("not a session file"), "{e}");
        }
        let e = parse_session(br#"{"version": 2, "files": []}"#).unwrap_err();
        assert!(e.contains("newer version"), "{e}");
        // serde quotes the string; the message stays short.
        let long = format!(r#"{{"version": "{}"}}"#, "x".repeat(10_000));
        let e = parse_session(long.as_bytes()).unwrap_err();
        assert!(e.chars().count() <= MAX_LINE + 1, "{e}");
        // As some editors save it.
        assert!(parse_session(b"\xEF\xBB\xBF{\"version\": 1}").is_ok());
    }

    #[test]
    fn relative_paths_start_in_the_session_files_folder() {
        let dir = std::env::temp_dir().join("sessions");
        assert_eq!(
            resolve(&dir, Path::new("notes.txt")),
            Ok(dir.join("notes.txt"))
        );
        let elsewhere = std::env::temp_dir().join("other").join("a.txt");
        assert_eq!(resolve(&dir, &elsewhere), Ok(elsewhere.clone()));
    }

    #[test]
    fn a_long_list_of_problems_is_cut_short() {
        let problems = (1..=25)
            .map(|i| format!("Entry {i}: {}", "x".repeat(i * 20)))
            .collect();
        let list = problem_list(problems);
        let lines: Vec<&str> = list.lines().collect();
        assert_eq!(lines.len(), MAX_LISTED + 1);
        assert_eq!(lines[0], format!("Entry 1: {}", "x".repeat(20)));
        assert!(lines[MAX_LISTED - 1].ends_with('…'));
        assert!(lines.iter().all(|l| l.chars().count() <= MAX_LINE + 1));
        assert_eq!(lines[MAX_LISTED], "and 5 more");
    }

    #[cfg(windows)]
    #[test]
    fn network_paths_are_refused() {
        let dir = std::env::temp_dir();
        for path in [
            r"\\host\share\x.txt",
            r"\\?\UNC\host\share\x.txt",
            r"\\.\pipe\x",
        ] {
            let e = resolve(&dir, Path::new(path)).unwrap_err();
            assert!(e.ends_with("not on this computer"), "{e}");
        }
    }
}
