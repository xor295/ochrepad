//! `Action`: what the user intends, independent of where the carets are
//! (PLAN 3.5). Key bindings map to actions and macros record them.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::buffer::BufferId;
use crate::position::VisualCol;
use crate::search::SearchQuery;
use crate::selection::SelectionSet;
use crate::transform::{CaseMode, SortOptions};
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
    /// Home with word wrap: to the start of the caret's display row, or
    /// `Home` when already there or on a line's first row. Without wrap it
    /// is `Home`.
    RowHome,
    /// End with word wrap: to the end of the caret's display row, or the
    /// line end when already there. Without wrap it is `LineEnd`.
    RowEnd,
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
    /// Tab key: a tab character, or spaces to the next tab stop when the
    /// editor is set to indent with spaces.
    InsertTab,
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
    /// Convert each selection (or the caret's line) to a case.
    ConvertCase(CaseMode),
    /// Sort the selected lines, the rectangle's lines keyed by the text in
    /// the rectangle, or the whole document when nothing is selected.
    SortLines(SortOptions),
    /// Remove trailing spaces and tabs from the selected lines (all lines
    /// when nothing is selected).
    TrimTrailingWhitespace,
    /// Duplicate the caret's line, or each selection.
    DuplicateLine,
    MoveLinesUp,
    MoveLinesDown,
    /// Join the selected lines (or the caret's line and the next).
    JoinLines,
    /// Remove empty lines from the selected lines (all lines when nothing
    /// is selected); with `include_whitespace`, also lines of only spaces
    /// and tabs.
    RemoveEmptyLines {
        include_whitespace: bool,
    },
    /// Remove lines equal to an earlier selected line (all lines when
    /// nothing is selected), keeping the order; with `consecutive_only`,
    /// only lines equal to the line right before them.
    RemoveDuplicateLines {
        consecutive_only: bool,
    },
    /// Fold the region holding the caret: the caret's line if it starts a
    /// region, else the innermost region around it. Fold actions change the
    /// view only (no undo step) and are not recorded in macros.
    Fold,
    /// Unfold the caret's line, or every fold inside the region around it.
    Unfold,
    /// Unfold the caret's line if it is folded, else `Fold`.
    ToggleFold,
    FoldAll,
    UnfoldAll,
    /// Fold every region of this level: the nesting depth for indentation
    /// (1 is outermost), the heading level in Markdown.
    FoldLevel(usize),
    UnfoldLevel(usize),
    /// Move the caret to a line and visual column, both counted from 1 as
    /// in the status bar (column 1 is the line start).
    GoToLine {
        line: usize,
        column: usize,
    },
    /// Indent the selected lines, or the caret's line, by one level (Tab
    /// does this only when a selection spans lines).
    Indent,
    /// Shift+Tab: remove one level of indentation from the selected lines,
    /// or the caret's line. (Tab with a selection over several lines
    /// indents them.)
    Unindent,
    /// Comment the selected lines (or the caret's line) with the language's
    /// line comment, or uncomment them if they all are. Languages with only
    /// block comments get a block comment around the lines.
    ToggleLineComment,
    /// Wrap each selection (or the caret's line) in a block comment, or
    /// unwrap it.
    ToggleBlockComment,
    /// Rewrite every line's leading whitespace as spaces or as tabs, keeping
    /// its width, and indent with that from now on.
    ConvertIndentation {
        to_spaces: bool,
    },
    /// Move the caret to the bracket matching the one beside it.
    GoToMatchingBracket,
    /// Select from the bracket beside the caret to its match, both included.
    SelectToMatchingBracket,
    Undo,
    Redo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionError {
    UnknownView(ViewId),
    UnknownBuffer(BufferId),
    InvalidPattern(String),
    NotFound,
    /// The buffer is read-only.
    ReadOnly,
    /// The language has no comment markers ochrepad knows.
    NoCommentSyntax,
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActionError::UnknownView(v) => write!(f, "no view with id {}", v.0),
            ActionError::UnknownBuffer(b) => write!(f, "no buffer with id {}", b.0),
            ActionError::InvalidPattern(e) => write!(f, "invalid search pattern: {e}"),
            ActionError::NotFound => f.write_str("no match found"),
            ActionError::ReadOnly => f.write_str("the document is read-only"),
            ActionError::NoCommentSyntax => {
                f.write_str("no comment markers are known for this language")
            }
        }
    }
}

impl std::error::Error for ActionError {}
