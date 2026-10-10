//! Resolving an [`Action`] against a document and selection set: motions, word
//! boundaries and the edits each action produces. Pure functions; `Editor`
//! turns the result into a transaction.

use unicode_segmentation::UnicodeSegmentation;

use crate::action::{Action, Motion};
use crate::changeset::{Bias, ChangeSet, Edit};
use crate::history::EditKind;
use crate::position::{
    ByteOffset, VisualCol, byte_to_visual_col, next_grapheme_boundary, prev_grapheme_boundary,
    visual_col_to_byte,
};
use crate::selection::{Selection, SelectionSet};
use crate::text::Document;

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
pub(crate) fn resolve(
    action: &Action,
    doc: &Document,
    sels: &SelectionSet,
    tab_width: usize,
) -> Resolved {
    match action {
        Action::InsertText(text) => {
            let kind = if sels.ranges().iter().all(Selection::is_empty)
                && text != "\n"
                && text.graphemes(true).count() == 1
            {
                EditKind::Insert
            } else {
                EditKind::Other
            };
            edit_each(doc, sels, kind, |s| {
                Edit::new(s.start(), s.end(), text.as_str())
            })
        }
        Action::InsertNewline => edit_each(doc, sels, EditKind::Other, |s| {
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
            Resolved::Selections(sels.map(|s| move_selection(doc, s, *motion, *extend, tab_width)))
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
        Action::AddCursorAbove => add_cursor(doc, sels, -1, tab_width),
        Action::AddCursorBelow => add_cursor(doc, sels, 1, tab_width),
        Action::CollapseSelections => {
            Resolved::Selections(SelectionSet::caret(sels.primary().head))
        }
        Action::Undo | Action::Redo => Resolved::Selections(sels.clone()),
    }
}

/// One edit per selection; afterwards each selection becomes a caret at the
/// end of its replacement.
fn edit_each(
    doc: &Document,
    sels: &SelectionSet,
    kind: EditKind,
    f: impl Fn(&Selection) -> Edit,
) -> Resolved {
    let edits: Vec<Edit> = sels.ranges().iter().map(f).collect();
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
    edit_each(doc, sels, kind, |s| {
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
    sel: &Selection,
    motion: Motion,
    extend: bool,
    tab_width: usize,
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
        Motion::Up => vertical(doc, sel, -1, tab_width),
        Motion::Down => vertical(doc, sel, 1, tab_width),
        Motion::PageUp(n) => vertical(doc, sel, -(n as isize), tab_width),
        Motion::PageDown(n) => vertical(doc, sel, n as isize, tab_width),
    };
    Selection {
        anchor: if extend { sel.anchor } else { new_head },
        head: new_head,
        goal,
    }
}

/// Moves `delta` lines keeping the remembered visual column. On the first or
/// last line the head stays put, as in Notepad++.
fn vertical(
    doc: &Document,
    sel: &Selection,
    delta: isize,
    tab_width: usize,
) -> (ByteOffset, Option<VisualCol>) {
    let line = doc.line_of(sel.head);
    let goal = sel
        .goal
        .unwrap_or_else(|| byte_to_visual_col(doc, sel.head, tab_width));
    let last = doc.line_count() as isize - 1;
    let target = (line as isize + delta).clamp(0, last) as usize;
    if target == line {
        return (sel.head, Some(goal));
    }
    (visual_col_to_byte(doc, target, goal, tab_width), Some(goal))
}

fn add_cursor(doc: &Document, sels: &SelectionSet, delta: isize, tab_width: usize) -> Resolved {
    let from = if delta < 0 {
        &sels.ranges()[0]
    } else {
        &sels.ranges()[sels.len() - 1]
    };
    let (pos, goal) = vertical(
        doc,
        &Selection::caret(from.head).with_goal(from.goal),
        delta,
        tab_width,
    );
    if doc.line_of(pos) == doc.line_of(from.head) {
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
