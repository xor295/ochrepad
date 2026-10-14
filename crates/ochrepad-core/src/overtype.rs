//! Overtype (the Insert key; `Settings::overtype`), as in Scintilla: a typed
//! character replaces the one after the caret instead of pushing it along.
//! At a line's end it is inserted as usual, a selection is replaced as
//! usual, and Enter and pasting always insert.

use unicode_segmentation::UnicodeSegmentation;

use crate::action::Action;
use crate::changeset::{ChangeSet, Edit};
use crate::history::EditKind;
use crate::position::{ByteOffset, next_grapheme_boundary};
use crate::resolve::Resolved;
use crate::selection::{Selection, SelectionSet};
use crate::text::Document;

/// The keystroke typed over the text after each caret, or `None` when no
/// caret has anything after it on its line (then it is typed as usual).
pub(crate) fn resolve(action: &Action, doc: &Document, sels: &SelectionSet) -> Option<Resolved> {
    let Action::InsertText(text) = action else {
        return None;
    };
    if text.is_empty() || text.contains('\n') {
        return None;
    }
    // One character of the text after the caret for each one typed.
    let typed = text.graphemes(true).count();
    let ranges = sels.ranges();
    let mut over = false;
    let edits: Vec<Edit> = ranges
        .iter()
        .enumerate()
        .map(|(i, s)| {
            if !s.is_empty() {
                return Edit::new(s.start(), s.end(), text.as_str());
            }
            // Up to the line's end, and never into the next selection.
            let mut limit = doc.line_end(doc.line_of(s.head));
            if let Some(next) = ranges.get(i + 1) {
                limit = limit.min(next.start());
            }
            let mut end = s.head;
            for _ in 0..typed {
                if end >= limit {
                    break;
                }
                end = next_grapheme_boundary(doc, end).min(limit);
            }
            over |= end > s.head;
            Edit::new(s.head, end, text.as_str())
        })
        .collect();
    if !over {
        return None;
    }
    let kind = if ranges.iter().all(Selection::is_empty) && typed == 1 {
        EditKind::Insert
    } else {
        EditKind::Other
    };
    // Each caret goes after its own text. (Edits can touch, and a change
    // set merges touching edits, so mapping their ends through it would
    // put two carets in one place.)
    let mut shift = 0isize;
    let carets: Vec<Selection> = edits
        .iter()
        .map(|e| {
            let start = e.start.0 as isize + shift;
            shift += e.text.len() as isize - (e.end.0 - e.start.0) as isize;
            Selection::caret(ByteOffset(start as usize + e.text.len()))
        })
        .collect();
    let changes = ChangeSet::new(doc.len(), edits);
    let after = SelectionSet::new(carets, sels.primary_index());
    Some(Resolved::Edit {
        changes,
        after,
        kind,
    })
}
