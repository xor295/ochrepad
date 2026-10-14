//! Smart indentation (Notepad++'s Advanced auto-indent; off by default:
//! `Settings::smart_indent`). Enter after an opening bracket, or after a
//! colon in Python, indents the new line one level more than its line;
//! Enter between a bracket pair also puts the closing bracket on a line of
//! its own, at the opening line's indentation. Typing a closing bracket on
//! a line of only blanks moves it to the indentation of the line its
//! partner is on. Brackets are matched as plain text, as `brackets` does.

use crate::action::Action;
use crate::brackets::SEARCH_LIMIT;
use crate::buffer::Indent;
use crate::changeset::{ChangeSet, Edit};
use crate::history::EditKind;
use crate::position::ByteOffset;
use crate::resolve::Resolved;
use crate::selection::{Selection, SelectionSet};
use crate::text::Document;

const PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];

/// The leading spaces and tabs of `line`.
fn leading(doc: &Document, line: usize) -> String {
    doc.line_text(line)
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

fn unit(indent: Indent) -> String {
    if indent.use_spaces {
        " ".repeat(indent.width.max(1))
    } else {
        "\t".to_owned()
    }
}

/// The keystroke with smart indentation, or `None` to type it as usual.
pub(crate) fn resolve(
    action: &Action,
    doc: &Document,
    sels: &SelectionSet,
    indent: Indent,
    language: Option<&str>,
) -> Option<Resolved> {
    if !matches!(action, Action::InsertNewline | Action::InsertText(_)) {
        return None;
    }
    // Two selections on one line would rewrite overlapping text (they are
    // sorted, so only neighbours can share one).
    let shared = sels
        .ranges()
        .windows(2)
        .any(|w| doc.line_of(w[0].end()) == doc.line_of(w[1].start()));
    if shared {
        return None;
    }
    match action {
        Action::InsertNewline => newline(doc, sels, indent, language),
        Action::InsertText(t) => {
            let mut chars = t.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if PAIRS.iter().any(|p| p.1 == c) => closer(doc, sels, c),
                _ => None,
            }
        }
        _ => None,
    }
}

fn newline(
    doc: &Document,
    sels: &SelectionSet,
    indent: Indent,
    language: Option<&str>,
) -> Option<Resolved> {
    let python = language.is_some_and(|l| l.eq_ignore_ascii_case("python"));
    let mut edits = Vec::new();
    // Where each caret goes: the edit's start plus this many bytes.
    let mut caret_in = Vec::new();
    let mut smart = false;
    for s in sels.ranges() {
        let line = doc.line_of(s.start());
        let line_start = doc.line_start(line);
        let prefix = doc.slice(line_start..s.start());
        let base: String = prefix
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let before = prefix.trim_end();
        let rest = doc.slice(s.end()..doc.line_end(doc.line_of(s.end())));
        let opener = before
            .chars()
            .last()
            .filter(|c| PAIRS.iter().any(|p| p.0 == *c) || (python && *c == ':'));
        let text = match opener {
            Some(open) => {
                smart = true;
                let inner = format!("\n{base}{}", unit(indent));
                let close = PAIRS.iter().find(|p| p.0 == open).map(|p| p.1);
                let closes = close.is_some_and(|c| rest.trim_start().starts_with(c));
                caret_in.push(inner.len());
                if closes {
                    // The closing bracket on its own line, the blanks
                    // before it dropped.
                    let blanks = rest.len() - rest.trim_start().len();
                    edits.push(Edit::new(
                        s.start(),
                        ByteOffset(s.end().0 + blanks),
                        format!("{inner}\n{base}"),
                    ));
                    continue;
                }
                inner
            }
            None => {
                let text = format!("\n{base}");
                caret_in.push(text.len());
                text
            }
        };
        edits.push(Edit::new(s.start(), s.end(), text));
    }
    if !smart {
        return None;
    }
    let starts: Vec<ByteOffset> = edits.iter().map(|e| e.start).collect();
    let changes = ChangeSet::new(doc.len(), edits);
    let after = SelectionSet::new(
        starts
            .iter()
            .zip(&caret_in)
            .map(|(&start, &n)| {
                let at = changes.map_position(start, crate::changeset::Bias::Before);
                Selection::caret(ByteOffset(at.0 + n))
            })
            .collect(),
        sels.primary_index(),
    );
    Some(Resolved::Edit {
        changes,
        after,
        kind: EditKind::Other,
    })
}

/// The unmatched `open` before `pos`, counting nesting, within the search
/// limit.
fn unmatched_opener(
    doc: &Document,
    pos: ByteOffset,
    open: char,
    close: char,
) -> Option<ByteOffset> {
    let mut start = pos.0.saturating_sub(SEARCH_LIMIT);
    while !doc.is_char_boundary(ByteOffset(start)) {
        start += 1;
    }
    let text = doc.slice(ByteOffset(start)..pos);
    let (open, close) = (open as u8, close as u8);
    let mut depth = 0usize;
    // Brackets are ASCII, never part of a multibyte character.
    for (i, b) in text.bytes().enumerate().rev() {
        if b == close {
            depth += 1;
        } else if b == open {
            if depth == 0 {
                return Some(ByteOffset(start + i));
            }
            depth -= 1;
        }
    }
    None
}

fn closer(doc: &Document, sels: &SelectionSet, c: char) -> Option<Resolved> {
    if !sels.ranges().iter().all(Selection::is_empty) {
        return None;
    }
    let open = PAIRS.iter().find(|p| p.1 == c)?.0;
    let mut edits = Vec::new();
    let mut smart = false;
    for s in sels.ranges() {
        let line = doc.line_of(s.head);
        let line_start = doc.line_start(line);
        let blanks = doc.slice(line_start..s.head);
        let after = doc.slice(s.head..doc.line_end(line));
        let blank = |t: &str| t.chars().all(|ch| ch == ' ' || ch == '\t');
        let aligned = (blank(&blanks) && blank(&after))
            .then(|| unmatched_opener(doc, s.head, open, c))
            .flatten()
            .map(|at| leading(doc, doc.line_of(at)));
        match aligned {
            Some(indent) if indent != blanks => {
                smart = true;
                edits.push(Edit::new(line_start, s.head, format!("{indent}{c}")));
            }
            _ => edits.push(Edit::new(s.head, s.head, c.to_string())),
        }
    }
    if !smart {
        return None;
    }
    let ends: Vec<ByteOffset> = edits.iter().map(|e| e.end).collect();
    let changes = ChangeSet::new(doc.len(), edits);
    let after = SelectionSet::new(
        ends.into_iter()
            .map(|e| Selection::caret(changes.map_position(e, crate::changeset::Bias::After)))
            .collect(),
        sels.primary_index(),
    );
    Some(Resolved::Edit {
        changes,
        after,
        kind: EditKind::Insert,
    })
}
