//! `View`: a window onto a buffer with its own selections and scroll position.

use serde::{Deserialize, Serialize};

use crate::buffer::BufferId;
use crate::selection::SelectionSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ViewId(pub u64);

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScrollPos {
    pub first_line: usize,
    /// Fraction of a line scrolled past `first_line`, in `[0, 1)`.
    pub line_fraction: f32,
    /// Horizontal offset in pixels.
    pub x_offset: f32,
}

#[derive(Clone, Debug)]
pub struct View {
    id: ViewId,
    buffer: BufferId,
    pub selections: SelectionSet,
    pub scroll: ScrollPos,
}

impl View {
    pub(crate) fn new(id: ViewId, buffer: BufferId) -> Self {
        Self {
            id,
            buffer,
            selections: SelectionSet::caret(Default::default()),
            scroll: ScrollPos::default(),
        }
    }

    pub fn id(&self) -> ViewId {
        self.id
    }

    pub fn buffer(&self) -> BufferId {
        self.buffer
    }
}

/// Maps document lines to display rows. Views never assume line N is row N;
/// word wrap and diff alignment will provide other implementations.
pub trait DisplayMap {
    fn row_count(&self, line_count: usize) -> usize;
    fn line_to_row(&self, line: usize) -> usize;
    /// The document line shown on `row`, or `None` for a filler row.
    fn row_to_line(&self, row: usize) -> Option<usize>;
}

/// v1: one row per line.
#[derive(Clone, Copy, Debug, Default)]
pub struct IdentityDisplayMap;

impl DisplayMap for IdentityDisplayMap {
    fn row_count(&self, line_count: usize) -> usize {
        line_count
    }

    fn line_to_row(&self, line: usize) -> usize {
        line
    }

    fn row_to_line(&self, row: usize) -> Option<usize> {
        Some(row)
    }
}
