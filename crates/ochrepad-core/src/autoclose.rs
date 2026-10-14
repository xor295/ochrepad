//! Auto-closing brackets and quotes, as Notepad++'s Auto-Insert (off by
//! default; `Settings::auto_close`). Typing an opening bracket or a quote
//! where a partner helps (before a space, a line end or a closing bracket)
//! inserts the partner after the caret; typing a closing character where
//! the same one already follows steps over it; Backspace between an empty
//! pair deletes both. Only with carets: typing over a selection replaces it
//! as usual. Each caret must agree, or the keystroke is typed as usual.

use crate::action::Action;
use crate::changeset::{ChangeSet, Edit};
use crate::history::EditKind;
use crate::position::ByteOffset;
use crate::resolve::Resolved;
use crate::selection::{Selection, SelectionSet};
use crate::text::Document;

const PAIRS: [(char, char); 5] = [('(', ')'), ('[', ']'), ('{', '}'), ('"', '"'), ('\'', '\'')];

fn is_closer(c: char) -> bool {
    PAIRS.iter().any(|p| p.1 == c)
}

fn char_before(doc: &Document, pos: ByteOffset) -> Option<char> {
    let mut p = pos.0.checked_sub(1)?;
    while !doc.is_char_boundary(ByteOffset(p)) {
        p = p.checked_sub(1)?;
    }
    doc.char_at(ByteOffset(p))
}

/// The keystroke with auto-closing applied, or `None` to type it as usual.
pub(crate) fn resolve(action: &Action, doc: &Document, sels: &SelectionSet) -> Option<Resolved> {
    if !sels.ranges().iter().all(Selection::is_empty) {
        return None;
    }
    match action {
        Action::InsertText(text) => typed(text, doc, sels),
        Action::DeleteBackward => backspace(doc, sels),
        _ => None,
    }
}

fn typed(text: &str, doc: &Document, sels: &SelectionSet) -> Option<Resolved> {
    let mut chars = text.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    let heads: Vec<ByteOffset> = sels.ranges().iter().map(|s| s.head).collect();
    // The closing character is already there: step over it.
    if is_closer(c) && heads.iter().all(|&h| doc.char_at(h) == Some(c)) {
        return Some(Resolved::Selections(
            sels.map(|s| Selection::caret(ByteOffset(s.head.0 + c.len_utf8()))),
        ));
    }
    let close = PAIRS.iter().find(|p| p.0 == c)?.1;
    let quote = c == close;
    let helps = heads.iter().all(|&h| {
        let next_ok = doc
            .char_at(h)
            .is_none_or(|n| n.is_whitespace() || (is_closer(n) && n != c));
        // A quote after a letter is an apostrophe ("don't").
        let prev_ok = !quote
            || char_before(doc, h).is_none_or(|p| !(p.is_alphanumeric() || p == '_' || p == c));
        next_ok && prev_ok
    });
    if !helps {
        return None;
    }
    let pair = format!("{c}{close}");
    let edits: Vec<Edit> = heads
        .iter()
        .map(|&h| Edit::new(h, h, pair.as_str()))
        .collect();
    let changes = ChangeSet::new(doc.len(), edits);
    // Each caret between its pair: where it was, plus everything inserted
    // before it, plus the opening character.
    let after = SelectionSet::new(
        heads
            .iter()
            .enumerate()
            .map(|(i, &h)| Selection::caret(ByteOffset(h.0 + i * pair.len() + c.len_utf8())))
            .collect(),
        sels.primary_index(),
    );
    Some(Resolved::Edit {
        changes,
        after,
        kind: EditKind::Insert,
    })
}

fn backspace(doc: &Document, sels: &SelectionSet) -> Option<Resolved> {
    let heads: Vec<ByteOffset> = sels.ranges().iter().map(|s| s.head).collect();
    let pairs: Vec<(char, char)> = heads
        .iter()
        .map(|&h| {
            let open = char_before(doc, h)?;
            let close = doc.char_at(h)?;
            PAIRS.contains(&(open, close)).then_some((open, close))
        })
        .collect::<Option<Vec<_>>>()?;
    let edits: Vec<Edit> = heads
        .iter()
        .zip(&pairs)
        .map(|(&h, (open, close))| {
            Edit::new(
                ByteOffset(h.0 - open.len_utf8()),
                ByteOffset(h.0 + close.len_utf8()),
                "",
            )
        })
        .collect();
    // Carets on both sides of one quote (`"|"|"`) would delete it twice.
    if edits.windows(2).any(|w| w[0].end > w[1].start) {
        return None;
    }
    let removed: Vec<usize> = pairs
        .iter()
        .map(|(o, c)| o.len_utf8() + c.len_utf8())
        .collect();
    let changes = ChangeSet::new(doc.len(), edits);
    let after = SelectionSet::new(
        heads
            .iter()
            .zip(&pairs)
            .enumerate()
            .map(|(i, (&h, (open, _)))| {
                let before: usize = removed[..i].iter().sum();
                Selection::caret(ByteOffset(h.0 - open.len_utf8() - before))
            })
            .collect(),
        sels.primary_index(),
    );
    Some(Resolved::Edit {
        changes,
        after,
        kind: EditKind::DeleteBackward,
    })
}
