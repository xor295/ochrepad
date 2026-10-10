//! `Buffer`: one document's text, undo history, revisions and file metadata.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::history::{History, Revision};
use crate::text::Document;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BufferId(pub u64);

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
    pub meta: FileMeta,
    /// Line edits since the last `take_line_edits`, for the highlighter.
    line_edits: Vec<LineEdit>,
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
            meta,
            line_edits: Vec::new(),
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
