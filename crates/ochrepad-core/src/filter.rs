//! The filter view's lines (PLAN_v0.3.0 P6): which lines a pattern shows.
//! The hiding itself is in [`crate::fold::FoldSet`], set through
//! [`crate::Editor::set_filter`].

use crate::position::ByteOffset;
use crate::search::Searcher;
use crate::text::Document;

/// How often [`shown_lines`] asks whether to stop, in lines.
const CANCEL_EVERY: usize = 4096;

/// The lines with a match of `searcher` (every line a match touches), or
/// with `keep_matching` false, the lines without one; sorted. Returns
/// `None` if `cancel` says to stop (it is asked now and then, so a search
/// on another thread can give up when the document changes).
pub fn shown_lines(
    doc: &Document,
    searcher: &Searcher,
    keep_matching: bool,
    cancel: &dyn Fn() -> bool,
) -> Option<Vec<usize>> {
    shown_lines_from(doc, searcher, keep_matching, 0, cancel)
}

/// [`shown_lines`] among the lines from `from` on (the lines a followed
/// file just gained).
pub fn shown_lines_from(
    doc: &Document,
    searcher: &Searcher,
    keep_matching: bool,
    from: usize,
    cancel: &dyn Fn() -> bool,
) -> Option<Vec<usize>> {
    let lines = doc.line_count();
    let from = from.min(lines);
    let mut matched = vec![false; lines - from];
    let mut line = from;
    let mut checked = from;
    while line < lines {
        // One search per line with a match at most: after a match, go on
        // from the start of the line after it.
        let Some(m) = searcher.find_next(doc, doc.line_start(line)) else {
            break;
        };
        let first = doc.line_of(m.start);
        let last = doc.line_of(if m.end > m.start {
            ByteOffset(m.end.0 - 1)
        } else {
            m.start
        });
        for flag in &mut matched[first.max(from) - from..=last - from] {
            *flag = true;
        }
        if last / CANCEL_EVERY != checked / CANCEL_EVERY && cancel() {
            return None;
        }
        checked = last;
        line = last + 1;
    }
    Some(
        (from..lines)
            .filter(|&l| matched[l - from] == keep_matching)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{SearchOptions, SearchQuery};

    fn lines(text: &str, pattern: &str, regex: bool, keep: bool) -> Vec<usize> {
        let q = SearchQuery::new(
            pattern,
            SearchOptions {
                regex,
                match_case: true,
                ..SearchOptions::default()
            },
        );
        let s = Searcher::new(&q).unwrap();
        shown_lines(&Document::new(text), &s, keep, &|| false).unwrap()
    }

    #[test]
    fn matching_and_other_lines() {
        let log = "INFO a\nERROR b ERROR\nINFO c\nERROR d";
        assert_eq!(lines(log, "ERROR", false, true), vec![1, 3]);
        assert_eq!(lines(log, "ERROR", false, false), vec![0, 2]);
        assert_eq!(lines(log, "nothing", false, true), Vec::<usize>::new());
        // A match over a line break counts for both lines; one matching
        // every line (empty at each start) shows them all.
        assert_eq!(lines(log, r"b ERROR\nINFO", true, true), vec![1, 2]);
        assert_eq!(lines(log, "^", true, true), vec![0, 1, 2, 3]);
    }

    #[test]
    fn searching_from_a_line_looks_at_the_lines_after_it() {
        let q = SearchQuery::new("ERROR", SearchOptions::default());
        let s = Searcher::new(&q).unwrap();
        let doc = Document::new("ERROR a\nINFO b\nERROR c\nINFO d");
        assert_eq!(
            shown_lines_from(&doc, &s, true, 1, &|| false),
            Some(vec![2])
        );
        assert_eq!(
            shown_lines_from(&doc, &s, false, 1, &|| false),
            Some(vec![1, 3])
        );
        assert_eq!(shown_lines_from(&doc, &s, true, 9, &|| false), Some(vec![]));
    }

    #[test]
    fn a_cancelled_search_returns_nothing() {
        let text = "x\n".repeat(20_000);
        let q = SearchQuery::new("x", SearchOptions::default());
        let s = Searcher::new(&q).unwrap();
        assert!(shown_lines(&Document::new(&text), &s, true, &|| true).is_none());
    }
}
