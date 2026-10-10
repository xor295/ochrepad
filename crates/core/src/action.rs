//! `Action`: what the user intends, independent of where the carets are
//! (PLAN 3.5). Key bindings map to actions and macros record them.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::buffer::BufferId;
use crate::position::VisualCol;
use crate::search::SearchQuery;
use crate::selection::SelectionSet;
use crate::view::ViewId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Motion {
    /// One grapheme cluster.
    Left,
    Right,
    WordLeft,
    WordRight,
    /// Smart home: first non-blank of the line, or column 0 if already there.
    Home,
    LineStart,
    LineEnd,
    DocStart,
    DocEnd,
    /// Vertical moves keep a remembered visual column.
    Up,
    Down,
    /// Page moves by the given number of lines (the view's page height).
    PageUp(usize),
    PageDown(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    InsertText(String),
    /// Like `InsertText`, but when there are several selections and the text
    /// has exactly that many lines, each selection gets one line.
    Paste(String),
    /// Newline plus the current line's leading whitespace (auto-indent).
    InsertNewline,
    DeleteBackward,
    DeleteForward,
    DeleteWordBackward,
    DeleteWordForward,
    MoveCursor {
        motion: Motion,
        extend: bool,
    },
    SelectAll,
    SelectWord,
    SelectLine,
    AddCursorAbove,
    AddCursorBelow,
    /// Collapse to a caret at the primary selection's head.
    CollapseSelections,
    /// Replace the selections (mouse clicks and drags). Positions are clamped
    /// into the document and onto char boundaries.
    SetSelections(SelectionSet),
    /// Rectangular selection in visual columns: one selection per line from
    /// the anchor corner to the head corner. Lines shorter than the rectangle
    /// get an empty selection at their end (no virtual space).
    ColumnSelect {
        anchor_line: usize,
        anchor_col: VisualCol,
        head_line: usize,
        head_col: VisualCol,
    },
    /// Grow or shrink the rectangle by moving its head corner (Alt+Shift+
    /// arrows); starts a rectangle at the primary caret if there is none.
    ColumnExtend(Direction),
    /// Paste a rectangular clip: line `i` goes to the caret's visual column
    /// on the `i`-th line below it, padding short lines with spaces and
    /// adding lines at the end of the document as needed. With several
    /// selections or a non-empty one it behaves like `Paste`.
    PasteBlock(String),
    /// Select the next (or previous) match after the primary selection.
    /// Fails with `NotFound` when there is none (macros stop there).
    Find {
        query: SearchQuery,
        backward: bool,
    },
    /// Replace the primary selection if it is exactly a match, then find
    /// the next match.
    Replace {
        query: SearchQuery,
        replacement: String,
    },
    /// Replace every match (inside the selections with "in selection") as
    /// one undo step.
    ReplaceAll {
        query: SearchQuery,
        replacement: String,
    },
    /// One selection per match.
    SelectAllMatches(SearchQuery),
    Undo,
    Redo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionError {
    UnknownView(ViewId),
    UnknownBuffer(BufferId),
    InvalidPattern(String),
    NotFound,
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActionError::UnknownView(v) => write!(f, "no view with id {}", v.0),
            ActionError::UnknownBuffer(b) => write!(f, "no buffer with id {}", b.0),
            ActionError::InvalidPattern(e) => write!(f, "invalid search pattern: {e}"),
            ActionError::NotFound => f.write_str("no match found"),
        }
    }
}

impl std::error::Error for ActionError {}
