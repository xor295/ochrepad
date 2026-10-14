//! Resolving an [`Action`] against a document and selection set: motions, word
//! boundaries and the edits each action produces. Pure functions; `Editor`
//! turns the result into a transaction.

use unicode_segmentation::UnicodeSegmentation;

use crate::action::{Action, ActionError, Direction, Motion};
use crate::changeset::{Bias, ChangeSet, Edit};
use crate::history::EditKind;
use crate::position::{
    ByteOffset, VisualCol, byte_to_visual_col, next_grapheme_boundary, prev_grapheme_boundary,
    visual_col_to_byte, visual_col_to_byte_ceil,
};
use std::ops::Range;

use crate::display::DisplayMap;
use crate::editor::Settings;
use crate::search::{SearchQuery, Searcher};
use crate::selection::{Rectangle, Selection, SelectionSet};
use crate::text::Document;
use crate::transform;

pub(crate) enum Resolved {
    /// Only the selections change.
    Selections(SelectionSet),
    Edit {
        changes: ChangeSet,
        after: SelectionSet,
        kind: EditKind,
    },
}

/// Resolves every action except `Undo`/`Redo`, which `Editor` handles.
/// Vertical motions move by the view's display `rows`.
pub(crate) fn resolve(
    action: &Action,
    doc: &Document,
    sels: &SelectionSet,
    settings: &Settings,
    rows: &DisplayMap,
) -> Resolved {
    let tab_width = settings.tab_width;
    match action {
        Action::InsertTab => {
            // A tab character, or spaces up to the next tab stop.
            edit_each(doc, sels, EditKind::Other, |_, s| {
                let text = if settings.use_spaces {
                    let tw = tab_width.max(1);
                    let col = byte_to_visual_col(doc, s.start(), tab_width).0;
                    " ".repeat(tw - col % tw)
                } else {
                    "\t".to_owned()
                };
                Edit::new(s.start(), s.end(), text)
            })
        }
        Action::InsertText(text) => {
            let kind = if sels.ranges().iter().all(Selection::is_empty)
                && text != "\n"
                && text.graphemes(true).count() == 1
            {
                EditKind::Insert
            } else {
                EditKind::Other
            };
            edit_each(doc, sels, kind, |_, s| {
                Edit::new(s.start(), s.end(), text.as_str())
            })
        }
        Action::Paste(text) => {
            // One line per selection when the counts match (e.g. a multi-caret
            // copy pasted back), otherwise the whole text at every selection.
            let body = text.strip_suffix('\n').unwrap_or(text);
            let lines: Vec<&str> = body.split('\n').collect();
            if sels.len() > 1 && lines.len() == sels.len() {
                edit_each(doc, sels, EditKind::Other, |i, s| {
                    Edit::new(s.start(), s.end(), lines[i])
                })
            } else {
                edit_each(doc, sels, EditKind::Other, |_, s| {
                    Edit::new(s.start(), s.end(), text.as_str())
                })
            }
        }
        Action::ColumnSelect {
            anchor_line,
            anchor_col,
            head_line,
            head_col,
        } => {
            let last = doc.line_count() - 1;
            Resolved::Selections(column_selection(
                doc,
                Rectangle {
                    start_line: (*anchor_line).min(last),
                    end_line: (*head_line).min(last),
                    start_col: *anchor_col,
                    end_col: *head_col,
                },
                tab_width,
            ))
        }
        Action::ColumnExtend(dir) => {
            let mut r = sels.rect().cloned().unwrap_or_else(|| {
                let head = sels.primary().head;
                let line = doc.line_of(head);
                let col = byte_to_visual_col(doc, head, tab_width);
                Rectangle {
                    start_line: line,
                    end_line: line,
                    start_col: col,
                    end_col: col,
                }
            });
            match dir {
                Direction::Up => r.end_line = r.end_line.saturating_sub(1),
                Direction::Down => r.end_line = (r.end_line + 1).min(doc.line_count() - 1),
                Direction::Left => {
                    r.end_col = step_col(doc, r.end_line, r.end_col, false, tab_width)
                }
                Direction::Right => {
                    r.end_col = step_col(doc, r.end_line, r.end_col, true, tab_width)
                }
            }
            Resolved::Selections(column_selection(doc, r, tab_width))
        }
        Action::ConvertCase(mode) => transform::case(doc, sels, *mode),
        Action::SortLines(options) => transform::sort(doc, sels, *options, tab_width),
        Action::TrimTrailingWhitespace => transform::trim_trailing(doc, sels),
        Action::DuplicateLine => transform::duplicate(doc, sels),
        Action::MoveLinesUp => transform::move_lines(doc, sels, true),
        Action::MoveLinesDown => transform::move_lines(doc, sels, false),
        Action::JoinLines => transform::join(doc, sels),
        Action::RemoveEmptyLines { include_whitespace } => {
            transform::remove_empty(doc, sels, *include_whitespace)
        }
        Action::RemoveDuplicateLines { consecutive_only } => {
            transform::remove_duplicates(doc, sels, *consecutive_only)
        }
        Action::PasteBlock(text) => {
            if sels.len() > 1 || !sels.primary().is_empty() {
                return resolve(&Action::Paste(text.clone()), doc, sels, settings, rows);
            }
            paste_block(doc, sels.primary().head, text, tab_width)
        }
        Action::SetSelections(set) => {
            let clamp = |p: ByteOffset| {
                let mut p = p.min(doc.len());
                while !doc.is_char_boundary(p) {
                    p.0 -= 1;
                }
                p
            };
            let ranges = set
                .ranges()
                .iter()
                .map(|s| Selection {
                    anchor: clamp(s.anchor),
                    head: clamp(s.head),
                    goal: s.goal,
                })
                .collect();
            Resolved::Selections(
                SelectionSet::new(ranges, set.primary_index()).with_rect(set.rect().cloned()),
            )
        }
        Action::InsertNewline => edit_each(doc, sels, EditKind::Other, |_, s| {
            let line = doc.line_of(s.start());
            let prefix = doc.slice(doc.line_start(line)..s.start());
            let indent: String = prefix
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            Edit::new(s.start(), s.end(), format!("\n{indent}"))
        }),
        Action::DeleteBackward => delete(doc, sels, EditKind::DeleteBackward, |p| {
            (prev_grapheme_boundary(doc, p), p)
        }),
        Action::DeleteForward => delete(doc, sels, EditKind::DeleteForward, |p| {
            (p, next_grapheme_boundary(doc, p))
        }),
        Action::DeleteWordBackward => {
            delete(doc, sels, EditKind::Other, |p| (word_left(doc, p), p))
        }
        Action::DeleteWordForward => {
            delete(doc, sels, EditKind::Other, |p| (p, word_right(doc, p)))
        }
        Action::MoveCursor { motion, extend } => {
            Resolved::Selections(sels.map(|s| move_selection(doc, rows, s, *motion, *extend)))
        }
        Action::SelectAll => Resolved::Selections(SelectionSet::single(Selection::new(
            ByteOffset(0),
            doc.len(),
        ))),
        Action::SelectWord => Resolved::Selections(sels.map(|s| {
            let (start, end) = word_at(doc, s.head);
            Selection::new(start, end)
        })),
        Action::SelectLine => Resolved::Selections(sels.map(|s| {
            let first = doc.line_of(s.start());
            let last = doc.line_of(s.end());
            let end = if last + 1 < doc.line_count() {
                doc.line_start(last + 1)
            } else {
                doc.len()
            };
            Selection::new(doc.line_start(first), end)
        })),
        Action::AddCursorAbove => add_cursor(rows, sels, -1),
        Action::AddCursorBelow => add_cursor(rows, sels, 1),
        Action::CollapseSelections => {
            Resolved::Selections(SelectionSet::caret(sels.primary().head))
        }
        // Handled by `resolve_search` / `Editor`.
        Action::Undo
        | Action::Redo
        | Action::Find { .. }
        | Action::Replace { .. }
        | Action::ReplaceAll { .. }
        | Action::SelectAllMatches(_) => Resolved::Selections(sels.clone()),
    }
}

fn searcher(query: &SearchQuery) -> Result<Searcher, ActionError> {
    Searcher::new(query).map_err(|e| ActionError::InvalidPattern(e.0))
}

/// Resolves the search actions, which can fail.
pub(crate) fn resolve_search(
    action: &Action,
    doc: &Document,
    sels: &SelectionSet,
) -> Result<Resolved, ActionError> {
    match action {
        Action::Find { query, backward } => {
            let m = searcher(query)?
                .next_from(doc, sels, *backward)
                .ok_or(ActionError::NotFound)?;
            Ok(Resolved::Selections(SelectionSet::single(Selection::new(
                m.start, m.end,
            ))))
        }
        Action::SelectAllMatches(query) => {
            let matches = searcher(query)?.find_in(doc, sels);
            if matches.is_empty() {
                return Err(ActionError::NotFound);
            }
            let ranges = matches
                .into_iter()
                .map(|m| Selection::new(m.start, m.end))
                .collect();
            Ok(Resolved::Selections(SelectionSet::new(ranges, 0)))
        }
        Action::ReplaceAll { query, replacement } => {
            let s = searcher(query)?;
            let edits: Vec<Edit> = s
                .find_in(doc, sels)
                .into_iter()
                .map(|m| {
                    let text = s.expand(doc, m.clone(), replacement);
                    Edit::new(m.start, m.end, text)
                })
                .collect();
            let changes = ChangeSet::new(doc.len(), edits);
            if changes.is_empty() {
                return Ok(Resolved::Selections(sels.clone()));
            }
            // Selections keep covering the (replaced) text they covered.
            let after = SelectionSet::new(
                sels.ranges()
                    .iter()
                    .map(|sel| {
                        let start = changes.map_position(sel.start(), Bias::Before);
                        let end = changes.map_position(sel.end(), Bias::After);
                        if sel.head < sel.anchor {
                            Selection::new(end, start)
                        } else {
                            Selection::new(start, end)
                        }
                    })
                    .collect(),
                sels.primary_index(),
            );
            Ok(Resolved::Edit {
                changes,
                after,
                kind: EditKind::Other,
            })
        }
        Action::Replace { query, replacement } => {
            let s = searcher(query)?;
            let select =
                |m: Range<ByteOffset>| SelectionSet::single(Selection::new(m.start, m.end));
            let Some((m, text)) = s.replacement_for_selection(doc, sels, replacement) else {
                // Not on a match: just find the next one.
                let m = s.next_from(doc, sels, false).ok_or(ActionError::NotFound)?;
                return Ok(Resolved::Selections(select(m)));
            };
            let changes = ChangeSet::new(doc.len(), vec![Edit::new(m.start, m.end, text)]);
            // One step: the selection afterwards is the next match, found in
            // the edited text (a rope clone is cheap).
            let caret = SelectionSet::caret(changes.map_position(m.end, Bias::After));
            let mut edited = doc.clone();
            changes.apply(&mut edited);
            let after = s.next_from(&edited, &caret, false).map_or(caret, select);
            if changes.is_empty() {
                return Ok(Resolved::Selections(after));
            }
            Ok(Resolved::Edit {
                changes,
                after,
                kind: EditKind::Other,
            })
        }
        // Only search actions are routed here.
        _ => Ok(Resolved::Selections(sels.clone())),
    }
}

/// One edit per selection (`f` gets its index); afterwards each selection becomes a caret at the
/// end of its replacement.
fn edit_each(
    doc: &Document,
    sels: &SelectionSet,
    kind: EditKind,
    mut f: impl FnMut(usize, &Selection) -> Edit,
) -> Resolved {
    let edits: Vec<Edit> = sels
        .ranges()
        .iter()
        .enumerate()
        .map(|(i, s)| f(i, s))
        .collect();
    let ends: Vec<ByteOffset> = edits.iter().map(|e| e.end).collect();
    let changes = ChangeSet::new(doc.len(), edits);
    let after = SelectionSet::new(
        ends.into_iter()
            .map(|e| Selection::caret(changes.map_position(e, Bias::After)))
            .collect(),
        sels.primary_index(),
    );
    if changes.is_empty() {
        Resolved::Selections(after)
    } else {
        Resolved::Edit {
            changes,
            after,
            kind,
        }
    }
}

/// Deletes each non-empty selection, or the range `range_at(head)` for carets.
/// Only an all-caret delete keeps the coalescible `kind`.
fn delete(
    doc: &Document,
    sels: &SelectionSet,
    kind: EditKind,
    range_at: impl Fn(ByteOffset) -> (ByteOffset, ByteOffset),
) -> Resolved {
    let kind = if sels.ranges().iter().all(Selection::is_empty) {
        kind
    } else {
        EditKind::Other
    };
    edit_each(doc, sels, kind, |_, s| {
        let (start, end) = if s.is_empty() {
            range_at(s.head)
        } else {
            (s.start(), s.end())
        };
        Edit::new(start, end, "")
    })
}

fn move_selection(
    doc: &Document,
    rows: &DisplayMap,
    sel: &Selection,
    motion: Motion,
    extend: bool,
) -> Selection {
    if !extend && !sel.is_empty() {
        match motion {
            Motion::Left => return Selection::caret(sel.start()),
            Motion::Right => return Selection::caret(sel.end()),
            _ => {}
        }
    }
    let head = sel.head;
    let line = doc.line_of(head);
    let (new_head, goal) = match motion {
        Motion::Left => (prev_grapheme_boundary(doc, head), None),
        Motion::Right => (next_grapheme_boundary(doc, head), None),
        Motion::WordLeft => (word_left(doc, head), None),
        Motion::WordRight => (word_right(doc, head), None),
        Motion::Home => {
            let first_non_blank = first_non_blank(doc, line);
            if head == first_non_blank {
                (doc.line_start(line), None)
            } else {
                (first_non_blank, None)
            }
        }
        Motion::LineStart => (doc.line_start(line), None),
        Motion::LineEnd => (doc.line_end(line), None),
        Motion::DocStart => (ByteOffset(0), None),
        Motion::DocEnd => (doc.len(), None),
        Motion::Up => vertical(rows, sel, -1),
        Motion::Down => vertical(rows, sel, 1),
        Motion::PageUp(n) => vertical(rows, sel, -(n as isize)),
        Motion::PageDown(n) => vertical(rows, sel, n as isize),
    };
    Selection {
        anchor: if extend { sel.anchor } else { new_head },
        head: new_head,
        goal,
    }
}

/// Moves `delta` display rows keeping the remembered visual column (counted
/// from the row's start). On the first or last row the head stays put, as
/// in Notepad++.
fn vertical(rows: &DisplayMap, sel: &Selection, delta: isize) -> (ByteOffset, Option<VisualCol>) {
    let row = rows.row_of(sel.head);
    let goal = sel.goal.unwrap_or_else(|| rows.col_in_row(sel.head));
    let target = rows.advance(row, delta);
    if target == row {
        return (sel.head, Some(goal));
    }
    (rows.byte_at_col(target, goal), Some(goal))
}

/// One selection per line of the rectangle, anchor column to head column;
/// the primary is on the head line. Each selection stays inside the
/// rectangle: a tab or wide glyph straddling an edge is left out.
fn column_selection(doc: &Document, r: Rectangle, tab_width: usize) -> SelectionSet {
    let ranges = (r.top()..=r.bottom())
        .map(|line| {
            let right = visual_col_to_byte(doc, line, r.right(), tab_width);
            let left = visual_col_to_byte_ceil(doc, line, r.left(), tab_width).min(right);
            if r.start_col <= r.end_col {
                Selection::new(left, right)
            } else {
                Selection::new(right, left)
            }
        })
        .collect();
    let primary = r.end_line - r.top();
    SelectionSet::new(ranges, primary).with_rect(Some(r))
}

/// The visual column one grapheme left or right of `col` on `line`; past the
/// end of the line the rectangle keeps growing one column at a time.
fn step_col(
    doc: &Document,
    line: usize,
    col: VisualCol,
    right: bool,
    tab_width: usize,
) -> VisualCol {
    let at = visual_col_to_byte(doc, line, col, tab_width);
    let at_col = byte_to_visual_col(doc, at, tab_width);
    if right {
        if at < doc.line_end(line) && at_col == col {
            byte_to_visual_col(doc, next_grapheme_boundary(doc, at), tab_width)
        } else {
            VisualCol(col.0 + 1)
        }
    } else if col.0 == 0 {
        col
    } else if at_col == col && at > doc.line_start(line) {
        byte_to_visual_col(doc, prev_grapheme_boundary(doc, at), tab_width)
    } else {
        VisualCol(col.0 - 1)
    }
}

fn paste_block(doc: &Document, head: ByteOffset, text: &str, tab_width: usize) -> Resolved {
    let line = doc.line_of(head);
    let col = byte_to_visual_col(doc, head, tab_width);
    let body = text.strip_suffix('\n').unwrap_or(text);
    let mut edits = Vec::new();
    let mut tail = String::new();
    for (i, piece) in body.split('\n').enumerate() {
        let target = line + i;
        if target < doc.line_count() {
            let pos = visual_col_to_byte(doc, target, col, tab_width);
            let have = byte_to_visual_col(doc, pos, tab_width).0;
            let pad = if pos == doc.line_end(target) {
                col.0.saturating_sub(have)
            } else {
                0
            };
            edits.push(Edit::new(pos, pos, format!("{}{piece}", " ".repeat(pad))));
        } else {
            tail.push('\n');
            tail.push_str(&" ".repeat(col.0));
            tail.push_str(piece);
        }
    }
    if !tail.is_empty() {
        edits.push(Edit::new(doc.len(), doc.len(), tail));
    }
    let changes = ChangeSet::new(doc.len(), edits);
    let after = SelectionSet::caret(changes.map_position(head, Bias::After));
    if changes.is_empty() {
        return Resolved::Selections(after);
    }
    Resolved::Edit {
        changes,
        after,
        kind: EditKind::Other,
    }
}

fn add_cursor(rows: &DisplayMap, sels: &SelectionSet, delta: isize) -> Resolved {
    let from = if delta < 0 {
        &sels.ranges()[0]
    } else {
        &sels.ranges()[sels.len() - 1]
    };
    let (pos, goal) = vertical(
        rows,
        &Selection::caret(from.head).with_goal(from.goal),
        delta,
    );
    if rows.row_of(pos) == rows.row_of(from.head) {
        return Resolved::Selections(sels.clone());
    }
    let mut ranges = sels.ranges().to_vec();
    ranges.push(Selection::caret(pos).with_goal(goal));
    let primary = ranges.len() - 1;
    Resolved::Selections(SelectionSet::new(ranges, primary))
}

fn first_non_blank(doc: &Document, line: usize) -> ByteOffset {
    let text = doc.line_text(line);
    let blank = text.len() - text.trim_start_matches([' ', '\t']).len();
    ByteOffset(doc.line_start(line).0 + blank)
}

// ---------------------------------------------------------------------------
// Word boundaries, by grapheme class.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class {
    Newline,
    Space,
    Word,
    Punct,
}

fn class_of(g: &str) -> Class {
    match g.chars().next() {
        Some('\n') => Class::Newline,
        Some(c) if c.is_whitespace() => Class::Space,
        Some(c) if c.is_alphanumeric() || c == '_' => Class::Word,
        _ => Class::Punct,
    }
}

/// The grapheme after `p`: its end and class.
fn after(doc: &Document, p: ByteOffset) -> Option<(ByteOffset, Class)> {
    (p < doc.len()).then(|| {
        let n = next_grapheme_boundary(doc, p);
        (n, class_of(&doc.slice(p..n)))
    })
}

/// The grapheme before `p`: its start and class.
fn before(doc: &Document, p: ByteOffset) -> Option<(ByteOffset, Class)> {
    (p.0 > 0).then(|| {
        let s = prev_grapheme_boundary(doc, p);
        (s, class_of(&doc.slice(s..p)))
    })
}

/// Skip the run of the current class (a newline is a run of its own), then
/// any spaces: lands on the start of the next word.
pub(crate) fn word_right(doc: &Document, mut p: ByteOffset) -> ByteOffset {
    let Some((n, class)) = after(doc, p) else {
        return p;
    };
    p = n;
    if class != Class::Newline {
        while let Some((n, c)) = after(doc, p)
            && c == class
        {
            p = n;
        }
    }
    while let Some((n, Class::Space)) = after(doc, p) {
        p = n;
    }
    p
}

/// Skip spaces backwards, then the run of the class before them.
pub(crate) fn word_left(doc: &Document, mut p: ByteOffset) -> ByteOffset {
    while let Some((s, Class::Space)) = before(doc, p) {
        p = s;
    }
    let Some((s, class)) = before(doc, p) else {
        return p;
    };
    p = s;
    if class != Class::Newline {
        while let Some((s, c)) = before(doc, p)
            && c == class
        {
            p = s;
        }
    }
    p
}

/// The run of same-class graphemes around `p`, preferring a word on either
/// side. Never crosses a newline.
fn word_at(doc: &Document, p: ByteOffset) -> (ByteOffset, ByteOffset) {
    let a = after(doc, p).map(|x| x.1).filter(|c| *c != Class::Newline);
    let b = before(doc, p).map(|x| x.1).filter(|c| *c != Class::Newline);
    let class = match (a, b) {
        (Some(Class::Word), _) => Class::Word,
        (_, Some(Class::Word)) => Class::Word,
        (Some(c), _) | (None, Some(c)) => c,
        (None, None) => return (p, p),
    };
    let mut start = p;
    while let Some((s, c)) = before(doc, start)
        && c == class
    {
        start = s;
    }
    let mut end = p;
    while let Some((n, c)) = after(doc, end)
        && c == class
    {
        end = n;
    }
    (start, end)
}
