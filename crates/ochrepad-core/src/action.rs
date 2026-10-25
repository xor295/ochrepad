//! `Action`: what the user intends, independent of where the carets are
//! (PLAN 3.5). Key bindings map to actions and macros record them.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::buffer::{BufferId, Indent};
use crate::column_edit::ColumnInsert;
use crate::position::{ByteOffset, VisualCol};
use crate::search::SearchQuery;
use crate::selection::SelectionSet;
use crate::text_tools::{Codec, InvalidJson};
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
    /// Multi-select Next: add the next occurrence of the primary
    /// selection's text (after it, going round from the end) as a new
    /// primary selection. With a caret, select the word at it first.
    MultiSelectNext {
        match_case: bool,
        whole_word: bool,
    },
    /// Multi-select All: a selection on every occurrence of the primary
    /// selection's text (or of the word at the caret).
    MultiSelectAll {
        match_case: bool,
        whole_word: bool,
    },
    /// Convert each selection (or the caret's line) to a case.
    ConvertCase(CaseMode),
    /// Sort the selected lines, the rectangle's lines keyed by the text in
    /// the rectangle, or the whole document when nothing is selected.
    SortLines(SortOptions),
    /// Put the lines Sort Lines would sort in a random order (Randomize
    /// Line Order); the seed decides the order, so a macro replays it.
    ShuffleLines {
        seed: u64,
    },
    /// Remove trailing spaces and tabs from the selected lines (all lines
    /// when nothing is selected).
    TrimTrailingWhitespace,
    /// Duplicate the caret's line, or each selection.
    DuplicateLine,
    MoveLinesUp,
    MoveLinesDown,
    /// Join the selected lines (or the caret's line and the next).
    JoinLines,
    /// Split each selected line (or the caret's line) wider than `width`
    /// columns at the blanks between words, as Notepad++'s Split Lines;
    /// the new lines start with the line's indentation.
    SplitLines {
        width: usize,
    },
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
    /// Delete the lines the selections touch (the caret's line), with their
    /// line breaks: Notepad++'s Line Delete (Ctrl+Shift+L).
    DeleteLine,
    /// Swap each caret's line with the line above it (Ctrl+T); the caret
    /// goes to the start of its line, as in Notepad++.
    TransposeLines,
    /// Insert a line above each caret's line (a selection's first line),
    /// indented like it, and move the caret there (Ctrl+Alt+Enter).
    InsertLineAbove,
    /// Insert a line below each caret's line (a selection's last line),
    /// indented like it, and move the caret there (Ctrl+Alt+Shift+Enter).
    InsertLineBelow,
    /// Reverse the order of the selected lines (every line when nothing is
    /// selected).
    ReverseLines,
    /// Remove leading spaces and tabs from the selected lines (all lines
    /// when nothing is selected).
    TrimLeadingWhitespace,
    /// Remove leading and trailing spaces and tabs from the selected lines
    /// (all lines when nothing is selected).
    TrimLeadingAndTrailingWhitespace,
    /// Replace the line breaks between the selected lines (all lines) with
    /// spaces, keeping the blanks around them.
    EolToSpace,
    /// Replace every tab on the selected lines (all lines) with spaces to
    /// the same tab stop.
    TabsToSpaces,
    /// Replace spaces with tabs on the selected lines (all lines) wherever
    /// the columns stay the same: the indentation, and runs of two or more
    /// spaces reaching a tab stop.
    SpacesToTabs,
    /// The Column Editor: insert text or a number sequence at one column
    /// on many lines (see [`ColumnInsert`]).
    ColumnInsert(ColumnInsert),
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
    /// Comment the selected lines (or the caret's line) even if they
    /// already are, as Notepad++'s Single Line Comment. Languages with only
    /// block comments get a block comment around the lines unless they are
    /// in one.
    CommentLines,
    /// Remove one line comment from each selected line that has one (or
    /// the block comment around the lines), as Notepad++'s Single Line
    /// Uncomment. Other lines are left alone.
    UncommentLines,
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
    /// Pretty-print the JSON in each selection, or the whole document when
    /// nothing is selected: one member or element per line, indented with
    /// `indent` (the document's indentation when `None`). Key order,
    /// numbers and strings are kept exactly. Fails on invalid JSON.
    FormatJson {
        #[serde(default)]
        indent: Option<Indent>,
    },
    /// Remove the whitespace between the JSON tokens of each selection (or
    /// the whole document).
    MinifyJson,
    /// Encode each selection, or the whole document when nothing is
    /// selected, as UTF-8 with the document's line endings.
    Encode(Codec),
    /// Decode each selection (or the whole document). Fails when it is not
    /// valid or the bytes are not UTF-8 text.
    Decode(Codec),
    /// Bookmark each caret's line, or remove the bookmarks if every one of
    /// those lines has one (Ctrl+F2). Bookmarks belong to the document;
    /// changing them is not an undo step.
    ToggleBookmark,
    /// Move the caret to the start of the next bookmarked line, wrapping
    /// around to the first (F2).
    NextBookmark,
    /// Move the caret to the start of the previous bookmarked line,
    /// wrapping around to the last (Shift+F2).
    PrevBookmark,
    /// Remove every bookmark.
    ClearBookmarks,
    /// Bookmark every line without a bookmark, and remove the others.
    InverseBookmarks,
    /// Bookmark every line holding the start of a match (in the selections
    /// with "in selection"), keeping the bookmarks already there.
    BookmarkMatches(SearchQuery),
    /// Delete the bookmarked lines with their line breaks.
    RemoveBookmarkedLines,
    /// Delete every line without a bookmark.
    RemoveUnbookmarkedLines,
    /// Replace the text of each bookmarked line (not its line break) with
    /// this text: Paste to (Replace) Bookmarked Lines.
    ReplaceBookmarkedLines(String),
    /// Accept a completion: at each caret, the `before` characters before
    /// it and the `after` characters after it (never past its line) are
    /// replaced with `text`; a selection is replaced like typing.
    Complete {
        before: usize,
        after: usize,
        text: String,
    },
    /// Replace these byte ranges as one undo step (a language server's
    /// edits: a completion's extra edits, a rename, formatting, a code
    /// action). Refused as a whole (`InvalidEdits`) unless every range is
    /// inside the document, on character boundaries and apart from the
    /// others. Never recorded in macros: the places are absolute.
    Edits(Vec<TextEdit>),
    Undo,
    Redo,
}

/// One replacement of [`Action::Edits`]: the bytes `start..end` become
/// `text`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextEdit {
    pub start: ByteOffset,
    pub end: ByteOffset,
    pub text: String,
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
    /// The JSON is not valid (boxed to keep errors small).
    InvalidJson(Box<InvalidJson>),
    /// The text cannot be decoded; the message says why.
    Decode(String),
    /// A bookmark command found no bookmarks.
    NoBookmarks,
    /// `Action::Edits` with a range outside the document, off a character
    /// boundary, or overlapping another.
    InvalidEdits,
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActionError::UnknownView(v) => write!(f, "no view with id {}", v.0),
            ActionError::UnknownBuffer(b) => write!(f, "no buffer with id {}", b.0),
            ActionError::InvalidPattern(e) => write!(f, "invalid search pattern: {e}"),
            ActionError::NotFound => f.write_str("no match found"),
            ActionError::InvalidEdits => {
                f.write_str("the edits do not fit the document (it may have changed)")
            }
            ActionError::ReadOnly => f.write_str("the document is read-only"),
            ActionError::NoCommentSyntax => {
                f.write_str("no comment markers are known for this language")
            }
            ActionError::InvalidJson(e) => write!(
                f,
                "invalid JSON on line {}, column {}: {}",
                e.line, e.column, e.message
            ),
            ActionError::Decode(message) => f.write_str(message),
            ActionError::NoBookmarks => f.write_str("there are no bookmarks"),
        }
    }
}

impl std::error::Error for ActionError {}
