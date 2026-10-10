//! `Action`: what the user intends, independent of where the carets are
//! (PLAN 3.5). Key bindings map to actions and macros record them.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::buffer::BufferId;
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
    Undo,
    Redo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionError {
    UnknownView(ViewId),
    UnknownBuffer(BufferId),
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActionError::UnknownView(v) => write!(f, "no view with id {}", v.0),
            ActionError::UnknownBuffer(b) => write!(f, "no buffer with id {}", b.0),
        }
    }
}

impl std::error::Error for ActionError {}
