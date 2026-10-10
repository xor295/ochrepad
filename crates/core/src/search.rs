//! Find and replace over the document without copying it into a `String`.
//!
//! Regexes run on `regex-cursor` (a prototype-stage fork of
//! `regex-automata` that searches discontiguous text) through a cursor over
//! the document's chunks. It is wrapped behind [`Matcher`] so another engine
//! — for example a `fancy-regex` fallback that materializes the text, for
//! lookaround and backreferences — can be added later.
//!
//! Dialect: Rust `regex` syntax, not Notepad++'s Boost regex. There is no
//! lookaround and no backreferences; `^`/`$` match at line boundaries.

use std::fmt;
use std::ops::Range;

use regex_cursor::engines::meta::Regex;
use regex_cursor::regex_automata::util::captures::Captures;
use regex_cursor::regex_automata::util::interpolate;
use regex_cursor::regex_automata::util::syntax;
use regex_cursor::regex_automata::{Anchored, PatternID};
use regex_cursor::{Cursor, Input};
use serde::{Deserialize, Serialize};

use crate::position::{ByteOffset, next_grapheme_boundary};
use crate::selection::SelectionSet;
use crate::text::Document;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchOptions {
    pub regex: bool,
    pub match_case: bool,
    pub whole_word: bool,
    pub wrap: bool,
    /// Count, select all and replace all only look inside the selections.
    pub in_selection: bool,
}

/// What the find panel searches for; part of the recorded find actions.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchQuery {
    pub pattern: String,
    pub options: SearchOptions,
}

impl SearchQuery {
    pub fn new(pattern: impl Into<String>, options: SearchOptions) -> Self {
        Self {
            pattern: pattern.into(),
            options,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchError(pub String);

impl fmt::Display for SearchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A compiled pattern that can search a document.
pub trait Matcher: Send + Sync {
    /// The leftmost match inside `span`, using the text around `span` as
    /// context for anchors and word boundaries.
    fn find(&self, doc: &Document, span: Range<usize>) -> Option<Range<usize>>;

    /// All non-overlapping matches inside `span`, in order.
    fn find_iter<'a>(
        &'a self,
        doc: &'a Document,
        span: Range<usize>,
    ) -> Box<dyn Iterator<Item = Range<usize>> + 'a>;

    /// Expands `$1`/`${name}` in `replacement` for the match at `m`.
    fn expand(&self, doc: &Document, m: Range<usize>, replacement: &str) -> String;
}

/// A [`regex_cursor::Cursor`] over the document's chunks.
struct DocCursor<'a> {
    doc: &'a Document,
    chunk: &'a [u8],
    start: usize,
}

impl<'a> DocCursor<'a> {
    fn at(doc: &'a Document, offset: usize) -> Self {
        let (chunk, start) = doc.chunk_at(ByteOffset(offset.min(doc.len().0)));
        Self {
            doc,
            chunk: chunk.as_bytes(),
            start: start.0,
        }
    }
}

impl Cursor for DocCursor<'_> {
    fn chunk(&self) -> &[u8] {
        self.chunk
    }

    fn advance(&mut self) -> bool {
        let next = self.start + self.chunk.len();
        if next >= self.doc.len().0 {
            return false;
        }
        let (chunk, start) = self.doc.chunk_at(ByteOffset(next));
        self.chunk = chunk.as_bytes();
        self.start = start.0;
        true
    }

    fn backtrack(&mut self) -> bool {
        if self.start == 0 {
            return false;
        }
        let (chunk, start) = self.doc.chunk_at(ByteOffset(self.start - 1));
        self.chunk = chunk.as_bytes();
        self.start = start.0;
        true
    }

    fn total_bytes(&self) -> Option<usize> {
        Some(self.doc.len().0)
    }

    fn offset(&self) -> usize {
        self.start
    }
}

struct CursorRegex {
    re: Regex,
    /// Literal mode: `$` in the replacement is not special.
    expand_captures: bool,
}

fn input(doc: &Document, span: Range<usize>) -> Input<DocCursor<'_>> {
    Input::new(DocCursor::at(doc, span.start)).range(span)
}

impl Matcher for CursorRegex {
    fn find(&self, doc: &Document, span: Range<usize>) -> Option<Range<usize>> {
        self.re.find(input(doc, span)).map(|m| m.range())
    }

    fn find_iter<'a>(
        &'a self,
        doc: &'a Document,
        span: Range<usize>,
    ) -> Box<dyn Iterator<Item = Range<usize>> + 'a> {
        Box::new(self.re.find_iter(input(doc, span)).map(|m| m.range()))
    }

    fn expand(&self, doc: &Document, m: Range<usize>, replacement: &str) -> String {
        if !self.expand_captures {
            return replacement.to_owned();
        }
        let mut caps: Captures = self.re.create_captures();
        let mut inp = input(doc, m.start..doc.len().0);
        inp.anchored(Anchored::Yes);
        self.re.captures(inp, &mut caps);
        let replacement = unescape(replacement);
        let mut out = String::new();
        interpolate::string(
            &replacement,
            |i, dst| {
                if let Some(span) = caps.get_group(i) {
                    dst.push_str(&doc.slice(ByteOffset(span.start)..ByteOffset(span.end)));
                }
            },
            |name| caps.group_info().to_index(PatternID::ZERO, name),
            &mut out,
        );
        out
    }
}

/// `\n`, `\t` and `\\` in a regex-mode replacement, as in Notepad++.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Escapes regex metacharacters so `s` matches literally.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        if r"\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// A query compiled for searching.
pub struct Searcher {
    matcher: Box<dyn Matcher>,
    options: SearchOptions,
}

impl Searcher {
    pub fn new(query: &SearchQuery) -> Result<Self, SearchError> {
        let o = &query.options;
        if query.pattern.is_empty() {
            return Err(SearchError("nothing to find".to_owned()));
        }
        let pattern = if o.regex {
            if o.whole_word {
                format!(r"\b(?:{})\b", query.pattern)
            } else {
                query.pattern.clone()
            }
        } else {
            let mut p = escape(&query.pattern);
            // Word boundaries only where the text starts or ends with a word
            // character, so "foo(" can still match as a whole word.
            if o.whole_word {
                if query.pattern.chars().next().is_some_and(is_word_char) {
                    p.insert_str(0, r"\b");
                }
                if query.pattern.chars().last().is_some_and(is_word_char) {
                    p.push_str(r"\b");
                }
            }
            p
        };
        let re = Regex::builder()
            .syntax(
                syntax::Config::new()
                    .case_insensitive(!o.match_case)
                    .multi_line(true),
            )
            .build(&pattern)
            .map_err(|e| SearchError(e.to_string()))?;
        Ok(Self {
            matcher: Box::new(CursorRegex {
                re,
                expand_captures: o.regex,
            }),
            options: o.clone(),
        })
    }

    pub fn options(&self) -> &SearchOptions {
        &self.options
    }

    /// The first match starting at or after `from`.
    pub fn find_next(&self, doc: &Document, from: ByteOffset) -> Option<Range<ByteOffset>> {
        let len = doc.len().0;
        (from.0 <= len)
            .then(|| self.matcher.find(doc, from.0..len))
            .flatten()
            .map(to_offsets)
    }

    /// The last match starting before `before`. Searches backwards in
    /// growing windows, so a match near the caret is found quickly.
    pub fn find_prev(&self, doc: &Document, before: ByteOffset) -> Option<Range<ByteOffset>> {
        let len = doc.len().0;
        let mut window = 64 * 1024;
        loop {
            let start = before.0.saturating_sub(window);
            let last = self
                .matcher
                .find_iter(doc, start..len)
                .take_while(|m| m.start < before.0)
                .last();
            if last.is_some() || start == 0 {
                return last.map(to_offsets);
            }
            window *= 4;
        }
    }

    /// All matches inside `range`.
    pub fn find_all(&self, doc: &Document, range: Range<ByteOffset>) -> Vec<Range<ByteOffset>> {
        self.matcher
            .find_iter(doc, range.start.0..range.end.0)
            .map(to_offsets)
            .collect()
    }

    /// The replacement text for the match at `m`.
    pub fn expand(&self, doc: &Document, m: Range<ByteOffset>, replacement: &str) -> String {
        self.matcher.expand(doc, m.start.0..m.end.0, replacement)
    }

    /// The ranges to search: the non-empty selections with "in selection"
    /// on, otherwise the whole document.
    pub fn ranges(&self, doc: &Document, sels: &SelectionSet) -> Vec<Range<ByteOffset>> {
        let selected: Vec<_> = sels
            .ranges()
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.range())
            .collect();
        if self.options.in_selection && !selected.is_empty() {
            selected
        } else {
            vec![ByteOffset(0)..doc.len()]
        }
    }

    /// Matches in the ranges `ranges` selects.
    pub fn find_in(&self, doc: &Document, sels: &SelectionSet) -> Vec<Range<ByteOffset>> {
        self.ranges(doc, sels)
            .into_iter()
            .flat_map(|r| self.find_all(doc, r))
            .collect()
    }

    /// The match to select for Find Next/Previous from the current primary
    /// selection, wrapping if enabled.
    pub fn next_from(
        &self,
        doc: &Document,
        sels: &SelectionSet,
        backward: bool,
    ) -> Option<Range<ByteOffset>> {
        let cur = sels.primary().range();
        if backward {
            self.find_prev(doc, cur.start)
                .filter(|m| *m != cur)
                .or_else(|| {
                    self.options
                        .wrap
                        .then(|| self.find_prev(doc, ByteOffset(doc.len().0 + 1)))
                        .flatten()
                })
        } else {
            let mut found = self.find_next(doc, cur.end);
            // An empty match at the caret is where we already are: step on.
            if found.as_ref() == Some(&cur) && cur.end < doc.len() {
                found = self.find_next(doc, next_grapheme_boundary(doc, cur.end));
            } else if found.as_ref() == Some(&cur) {
                found = None;
            }
            found.or_else(|| {
                self.options
                    .wrap
                    .then(|| self.find_next(doc, ByteOffset(0)))
                    .flatten()
            })
        }
    }

    /// If the primary selection is exactly a match, the range and its
    /// replacement text (for Replace).
    pub fn replacement_for_selection(
        &self,
        doc: &Document,
        sels: &SelectionSet,
        replacement: &str,
    ) -> Option<(Range<ByteOffset>, String)> {
        let cur = sels.primary().range();
        let m = self.find_next(doc, cur.start)?;
        (m == cur).then(|| {
            let text = self.expand(doc, m.clone(), replacement);
            (m, text)
        })
    }
}

fn to_offsets(r: Range<usize>) -> Range<ByteOffset> {
    ByteOffset(r.start)..ByteOffset(r.end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(pattern: &str, f: impl FnOnce(&mut SearchOptions)) -> Searcher {
        let mut o = SearchOptions::default();
        f(&mut o);
        Searcher::new(&SearchQuery::new(pattern, o)).expect("valid pattern")
    }

    fn all(s: &Searcher, text: &str) -> Vec<(usize, usize)> {
        let doc = Document::new(text);
        s.find_all(&doc, ByteOffset(0)..doc.len())
            .into_iter()
            .map(|r| (r.start.0, r.end.0))
            .collect()
    }

    #[test]
    fn literal_case_and_whole_word() {
        let text = "Foo foo food (foo) foo(";
        assert_eq!(all(&q("foo", |_| {}), text).len(), 5);
        assert_eq!(all(&q("foo", |o| o.match_case = true), text).len(), 4);
        assert_eq!(
            all(&q("foo", |o| o.whole_word = true), text),
            vec![(0, 3), (4, 7), (14, 17), (19, 22)]
        );
        // A literal ending in a non-word char still matches as a word.
        assert_eq!(
            all(&q("foo(", |o| o.whole_word = true), text),
            vec![(19, 23)]
        );
        // Metacharacters are literal unless regex is on.
        assert_eq!(all(&q("(foo)", |_| {}), text), vec![(13, 18)]);
    }

    #[test]
    fn regex_lines_and_invalid_patterns() {
        let s = q(r"^\w+", |o| o.regex = true);
        assert_eq!(all(&s, "ab cd\nef"), vec![(0, 2), (6, 8)]);
        let err = Searcher::new(&SearchQuery::new(
            "(unclosed",
            SearchOptions {
                regex: true,
                ..Default::default()
            },
        ));
        assert!(err.is_err());
        // No lookaround in this dialect.
        assert!(
            Searcher::new(&SearchQuery::new(
                "a(?=b)",
                SearchOptions {
                    regex: true,
                    ..Default::default()
                }
            ))
            .is_err()
        );
    }

    #[test]
    fn matches_across_rope_chunks() {
        let text = format!("{}needle{}", "x".repeat(5000), "y".repeat(5000)).repeat(20);
        let s = q("needle", |_| {});
        let found = all(&s, &text);
        assert_eq!(found.len(), 20);
        let doc = Document::new(&text);
        for (a, b) in found {
            assert_eq!(doc.slice(ByteOffset(a)..ByteOffset(b)), "needle");
        }
    }

    #[test]
    fn previous_and_next() {
        let doc = Document::new(&format!("ab{}ab", "-".repeat(200_000)));
        let s = q("ab", |_| {});
        let end = doc.len();
        assert_eq!(s.find_prev(&doc, end).map(|r| r.start.0), Some(200_002));
        assert_eq!(
            s.find_prev(&doc, ByteOffset(200_002)).map(|r| r.start.0),
            Some(0)
        );
        assert_eq!(s.find_prev(&doc, ByteOffset(0)), None);
        assert_eq!(
            s.find_next(&doc, ByteOffset(1)).map(|r| r.start.0),
            Some(200_002)
        );
    }

    #[test]
    fn expansion() {
        let doc = Document::new("John Smith");
        let s = q(r"(?<first>\w+) (\w+)", |o| o.regex = true);
        let m = s.find_next(&doc, ByteOffset(0)).unwrap();
        assert_eq!(s.expand(&doc, m.clone(), "$2, ${first}"), "Smith, John");
        assert_eq!(s.expand(&doc, m.clone(), r"$0\n\t$$"), "John Smith\n\t$");
        // Literal mode leaves the replacement alone.
        let lit = q("John", |_| {});
        let m = lit.find_next(&doc, ByteOffset(0)).unwrap();
        assert_eq!(lit.expand(&doc, m, "$1 \\n"), "$1 \\n");
    }
}
