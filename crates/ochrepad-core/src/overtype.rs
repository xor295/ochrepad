//! Overtype (the Insert key; `Settings::overtype`), as in Scintilla: a typed
//! character replaces the one after the caret instead of pushing it along.
//! At a line's end it is inserted as usual, a selection is replaced as
//! usual, and Enter and pasting always insert.

use unicode_segmentation::UnicodeSegmentation;

use crate::action::Action;
use crate::changeset::{Bias, ChangeSet, Edit};
use crate::history::EditKind;
use crate::position::next_grapheme_boundary;
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
    let mut over = false;
    let edits: Vec<Edit> = sels
        .ranges()
        .iter()
        .map(|s| {
            if !s.is_empty() {
                return Edit::new(s.start(), s.end(), text.as_str());
            }
            let line_end = doc.line_end(doc.line_of(s.head));
            let mut end = s.head;
            for _ in 0..typed {
                if end >= line_end {
                    break;
                }
                end = next_grapheme_boundary(doc, end).min(line_end);
            }
            over |= end > s.head;
            Edit::new(s.head, end, text.as_str())
        })
        .collect();
    if !over {
        return None;
    }
    let kind = if sels.ranges().iter().all(Selection::is_empty) && typed == 1 {
        EditKind::Insert
    } else {
        EditKind::Other
    };
    let ends: Vec<_> = edits.iter().map(|e| e.end).collect();
    let changes = ChangeSet::new(doc.len(), edits);
    let after = SelectionSet::new(
        ends.into_iter()
            .map(|e| Selection::caret(changes.map_position(e, Bias::After)))
            .collect(),
        sels.primary_index(),
    );
    Some(Resolved::Edit {
        changes,
        after,
        kind,
    })
}
