//! `View`: a window onto a buffer with its own selections and scroll position.

use std::cell::RefCell;

use serde::{Deserialize, Serialize};

use crate::buffer::BufferId;
use crate::display::WrapCache;
use crate::fold::FoldSet;
use crate::selection::SelectionSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ViewId(pub u64);

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScrollPos {
    /// The top row: `first_sub` is the row within `first_line` (see
    /// [`crate::display`]).
    pub first_line: usize,
    #[serde(default)]
    pub first_sub: usize,
    /// Fraction of the top row scrolled past, in `[0, 1)`.
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
    /// Folded regions; changed only through `Editor`, which keeps them in
    /// step with the text.
    pub(crate) folds: FoldSet,
    /// Wrap width in cells, or `None` when lines do not wrap. The editor
    /// widget sets it from the pane's width.
    pub wrap_cols: Option<usize>,
    pub(crate) wrap_cache: RefCell<WrapCache>,
}

impl View {
    pub(crate) fn new(id: ViewId, buffer: BufferId) -> Self {
        Self {
            id,
            buffer,
            selections: SelectionSet::caret(Default::default()),
            scroll: ScrollPos::default(),
            folds: FoldSet::default(),
            wrap_cols: None,
            wrap_cache: RefCell::default(),
        }
    }

    pub fn id(&self) -> ViewId {
        self.id
    }

    pub fn buffer(&self) -> BufferId {
        self.buffer
    }

    pub fn folds(&self) -> &FoldSet {
        &self.folds
    }
}
