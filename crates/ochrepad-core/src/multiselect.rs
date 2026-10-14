//! Multi-select Next and Multi-select All (Notepad++'s Edit menu; Ctrl+D in
//! other editors): selections on the other occurrences of the selected
//! text, so typing edits them all. A caret first selects the word at it.

use crate::action::{Action, ActionError};
use crate::position::ByteOffset;
use crate::resolve::Resolved;
use crate::search::{SearchOptions, SearchQuery, Searcher};
use crate::selection::{Selection, SelectionSet};
use crate::text::Document;

pub(crate) fn resolve(
    action: &Action,
    doc: &Document,
    sels: &SelectionSet,
) -> Result<Resolved, ActionError> {
    let (match_case, whole_word, all) = match *action {
        Action::MultiSelectNext {
            match_case,
            whole_word,
        } => (match_case, whole_word, false),
        Action::MultiSelectAll {
            match_case,
            whole_word,
        } => (match_case, whole_word, true),
        _ => return Ok(Resolved::Selections(sels.clone())),
    };
    let primary = sels.primary();
    let range = if primary.is_empty() {
        let (start, end) = crate::resolve::word_at(doc, primary.head);
        if start == end {
            return Err(ActionError::NotFound);
        }
        if !all {
            // The first press selects the word; the next ones add.
            let mut ranges = sels.ranges().to_vec();
            ranges[sels.primary_index()] = Selection::new(start, end);
            return Ok(Resolved::Selections(SelectionSet::new(
                ranges,
                sels.primary_index(),
            )));
        }
        start..end
    } else {
        primary.range()
    };
    let text = doc.slice(range.clone());
    // Plain text: "whole word" then only asks for a word boundary where the
    // text starts or ends with a word character ($var, foo( match).
    let query = SearchQuery::new(
        text,
        SearchOptions {
            regex: false,
            match_case,
            whole_word,
            wrap: true,
            in_selection: false,
        },
    );
    let searcher = Searcher::new(&query).map_err(|e| ActionError::InvalidPattern(e.0))?;
    if all {
        let matches = searcher.find_all(doc, ByteOffset(0)..doc.len());
        if matches.is_empty() {
            return Err(ActionError::NotFound);
        }
        let primary = matches.iter().position(|m| *m == range).unwrap_or(0);
        let ranges = matches
            .into_iter()
            .map(|m| Selection::new(m.start, m.end))
            .collect();
        return Ok(Resolved::Selections(SelectionSet::new(ranges, primary)));
    }
    // The next occurrence after the newest selection that is not selected
    // yet, going round from the end to the start.
    // The selections are sorted: look a match up by its start.
    let ranges = sels.ranges();
    let taken = |m: &std::ops::Range<ByteOffset>| {
        let i = ranges.partition_point(|s| s.start() < m.start);
        ranges.get(i).is_some_and(|s| s.range() == *m)
    };
    let mut from = range.end;
    let mut wrapped = false;
    let next = loop {
        let found = searcher
            .find_next(doc, from)
            .filter(|m| m.start < m.end)
            .or_else(|| {
                if wrapped {
                    None
                } else {
                    wrapped = true;
                    searcher
                        .find_next(doc, ByteOffset(0))
                        .filter(|m| m.start < m.end)
                }
            });
        let Some(m) = found else {
            return Err(ActionError::NotFound);
        };
        if !taken(&m) {
            break m;
        }
        if wrapped && m.start >= range.start {
            return Err(ActionError::NotFound);
        }
        from = m.end;
    };
    let mut ranges = sels.ranges().to_vec();
    ranges.push(Selection::new(next.start, next.end));
    let primary = ranges.len() - 1;
    Ok(Resolved::Selections(
        SelectionSet::new(ranges, primary).with_rect(None),
    ))
}
