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
    /// First line whose text changed since the highlighter last asked.
    pub(crate) dirty_from_line: Option<usize>,
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
            dirty_from_line: None,
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
        self.saved_revision = self.revision;
        self.history.break_coalescing();
    }

    /// The first line changed since the last call, for incremental highlighting.
    pub fn take_dirty_from_line(&mut self) -> Option<usize> {
        self.dirty_from_line.take()
    }

    pub(crate) fn note_dirty_line(&mut self, line: usize) {
        self.dirty_from_line = Some(self.dirty_from_line.map_or(line, |l| l.min(line)));
    }
}
