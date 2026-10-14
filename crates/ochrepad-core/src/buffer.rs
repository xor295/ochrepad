//! `Buffer`: one document's text, undo history, revisions and file metadata.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::bookmarks::Bookmarks;
use crate::fold::FoldStrategy;
use crate::history::{History, Revision};
use crate::text::Document;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BufferId(pub u64);

/// How a document is indented: the Tab key's character and the tab width
/// (also how wide a tab character is drawn).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Indent {
    /// The Tab key inserts spaces instead of a tab character.
    pub use_spaces: bool,
    /// Cells per indent level and per tab stop.
    pub width: usize,
}

impl Default for Indent {
    fn default() -> Self {
        Self {
            use_spaces: false,
            width: 4,
        }
    }
}

/// A language's comment markers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentSyntax {
    /// Starts a comment that runs to the end of the line (`//`, `#`).
    pub line: Option<String>,
    /// Opens and closes a block comment (`/*`, `*/`).
    pub block: Option<(String, String)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineEnding {
    #[default]
    Lf,
    CrLf,
    Cr,
}

/// How the file was stored on disk. Filled in and used only by `fileio`; core
/// never touches the filesystem.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMeta {
    pub path: Option<PathBuf>,
    /// WHATWG encoding label, e.g. "UTF-8", "UTF-16LE", "windows-1252".
    pub encoding: String,
    pub bom: bool,
    pub line_ending: LineEnding,
    pub mixed_line_endings: bool,
}

impl Default for FileMeta {
    fn default() -> Self {
        Self {
            path: None,
            encoding: "UTF-8".to_owned(),
            bom: false,
            line_ending: LineEnding::default(),
            mixed_line_endings: false,
        }
    }
}

/// How one edit changed the lines: lines `start..=old_end` were replaced by
/// lines `start..=new_end`, and every later line moved by `new_end - old_end`.
/// `old_end == new_end == usize::MAX` means "everything from `start` on may
/// have changed" (the log overflowed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineEdit {
    pub start: usize,
    pub old_end: usize,
    pub new_end: usize,
}

impl LineEdit {
    pub fn everything_from(start: usize) -> Self {
        Self {
            start,
            old_end: usize::MAX,
            new_end: usize::MAX,
        }
    }

    pub fn is_everything(&self) -> bool {
        self.old_end == usize::MAX
    }
}

const MAX_PENDING_LINE_EDITS: usize = 1024;

#[derive(Debug)]
pub struct Buffer {
    id: BufferId,
    pub(crate) doc: Document,
    pub(crate) history: History,
    pub(crate) revision: Revision,
    pub(crate) next_revision: Revision,
    saved_revision: Revision,
    pub language: Option<String>,
    /// How fold regions are found; the app sets it from the language.
    pub fold_strategy: FoldStrategy,
    /// Edits are refused (a compare view's copy of a file on disk).
    pub read_only: bool,
    /// The language's comment markers, for toggling comments; the app sets
    /// them from the language.
    pub comments: Option<CommentSyntax>,
    /// Indentation and tab width (from the preferences, or detected).
    pub indent: Indent,
    /// `indent` was detected from the file or chosen for it, so changing
    /// the preferences leaves it alone.
    pub indent_set: bool,
    /// Some lines are indented with tabs and others with spaces.
    pub indent_mixed: bool,
    pub meta: FileMeta,
    /// Line edits since the last `take_line_edits`, for the highlighter.
    line_edits: Vec<LineEdit>,
    /// The bookmarked lines; edits move them (`Editor` maps them).
    pub(crate) bookmarks: Bookmarks,
}

impl Buffer {
    pub(crate) fn new(id: BufferId, text: &str, meta: FileMeta) -> Self {
        Self {
            id,
            doc: Document::new(text),
            history: History::default(),
            revision: 0,
            next_revision: 1,
            saved_revision: 0,
            language: None,
            fold_strategy: FoldStrategy::Indent,
            read_only: false,
            comments: None,
            indent: Indent::default(),
            indent_set: false,
            indent_mixed: false,
            meta,
            line_edits: Vec::new(),
            bookmarks: Bookmarks::default(),
        }
    }

    pub fn id(&self) -> BufferId {
        self.id
    }

    pub fn doc(&self) -> &Document {
        &self.doc
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    pub fn bookmarks(&self) -> &Bookmarks {
        &self.bookmarks
    }

    /// Replaces the bookmarks (a session restoring them); lines past the
    /// end are dropped.
    pub fn set_bookmarks(&mut self, lines: impl IntoIterator<Item = usize>) {
        let count = self.doc.line_count();
        self.bookmarks.set(lines, count);
    }

    /// Bookmarks `line`, or removes its bookmark (a click in the margin);
    /// returns whether it has one now.
    pub fn toggle_bookmark(&mut self, line: usize) -> bool {
        line < self.doc.line_count() && self.bookmarks.toggle(line)
    }

    /// Identifies the current text state; returns to an earlier value on undo.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    /// Records that the current state is what is on disk.
    pub fn mark_saved(&mut self) {
        self.mark_saved_at(self.revision);
    }

    /// Records that `revision` is what is on disk (a background save of an
    /// earlier state finished; the buffer stays dirty if it changed since).
    pub fn mark_saved_at(&mut self, revision: Revision) {
        self.saved_revision = revision;
        self.history.break_coalescing();
    }

    /// The history to keep between sessions: the newest steps within about
    /// `max_bytes`, ending at the current text.
    pub fn saved_history(&self, max_bytes: usize) -> crate::history::SavedHistory {
        let (undo, redo) = self.history.newest(max_bytes);
        crate::history::SavedHistory {
            undo,
            redo,
            revision: self.revision,
            next_revision: self.next_revision,
        }
    }

    /// Takes a history kept from an earlier session for this same text
    /// (the caller compares the text's hash first). It is checked step by
    /// step against the text ([`SavedHistory::check`]) and refused if
    /// anything does not fit. `on_disk`: the text is the file's (so undoing
    /// and redoing back to it leaves the buffer clean); otherwise the buffer
    /// stays unsaved.
    pub fn restore_history(
        &mut self,
        saved: crate::history::SavedHistory,
        on_disk: bool,
    ) -> Result<(), String> {
        saved.check(&self.doc)?;
        self.history.replace(saved.undo, saved.redo);
        self.revision = saved.revision;
        self.next_revision = self.next_revision.max(saved.next_revision);
        if on_disk {
            self.saved_revision = saved.revision;
        } else {
            self.mark_modified();
        }
        Ok(())
    }

    /// Forgets the undo and redo history.
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Marks the buffer as differing from disk, e.g. when its text was
    /// restored from a backup snapshot.
    pub fn mark_modified(&mut self) {
        // No reachable revision has this value.
        self.saved_revision = Revision::MAX;
    }

    /// How the lines changed since the last call, in the order the edits
    /// were applied (for incremental highlighting).
    pub fn take_line_edits(&mut self) -> Vec<LineEdit> {
        std::mem::take(&mut self.line_edits)
    }

    pub(crate) fn record_line_edit(&mut self, edit: LineEdit) {
        if let Some(last) = self.line_edits.last_mut()
            && last.old_end == usize::MAX
        {
            last.start = last.start.min(edit.start);
            return;
        }
        if self.line_edits.len() >= MAX_PENDING_LINE_EDITS {
            // Nobody is draining the log; keep one conservative entry. The
            // smallest start is never after the earliest changed line.
            let start = self
                .line_edits
                .iter()
                .map(|e| e.start)
                .fold(edit.start, usize::min);
            self.line_edits = vec![LineEdit::everything_from(start)];
            return;
        }
        self.line_edits.push(edit);
    }
}
