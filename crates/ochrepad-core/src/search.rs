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

/// Bytes read on demand from somewhere too large to hold in memory, such as
/// a huge file the app reads in pieces (see [`Searcher::find_next_in`]).
pub trait ByteSource: Sync {
    /// The length in bytes, fixed for the duration of a search.
    fn len(&self) -> u64;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Reads from `at` into `buf`; returns how many bytes were read: fewer
    /// at the end of the data, and 0 past it, on an error, or to stop a
    /// search early.
    fn read_at(&self, at: u64, buf: &mut [u8]) -> usize;
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

    /// All non-overlapping matches inside `span` of bytes read from `src`.
    fn find_iter_source<'a>(
        &'a self,
        src: &'a dyn ByteSource,
        span: Range<usize>,
    ) -> Box<dyn Iterator<Item = Range<usize>> + 'a>;

    /// All non-overlapping matches inside `span` of `hay`.
    fn find_iter_bytes<'a>(
        &'a self,
        hay: &'a [u8],
        span: Range<usize>,
    ) -> Box<dyn Iterator<Item = Range<usize>> + 'a>;

    /// Expands `$1`/`${name}` in `replacement` for the match at `m`.
    fn expand(&self, doc: &Document, m: Range<usize>, replacement: &str) -> String;
}

/// Bytes a [`SourceCursor`] reads at a time.
const SOURCE_CHUNK: usize = 1 << 20;

fn is_continuation(b: u8) -> bool {
    b & 0xC0 == 0x80
}

/// A [`regex_cursor::Cursor`] over a [`ByteSource`], one chunk in memory at
/// a time. Chunks start and end only where a byte is not a UTF-8
/// continuation byte (as the cursor contract asks: no character is split),
/// unless the data there is not UTF-8 anyway (four or more continuation
/// bytes in a row).
struct SourceCursor<'a> {
    src: &'a dyn ByteSource,
    len: usize,
    buf: Vec<u8>,
    /// Where the next chunk is read, swapped in when the read works.
    spare: Vec<u8>,
    /// The current chunk is `buf[..used]`, at `start` in the data.
    used: usize,
    start: usize,
}

impl<'a> SourceCursor<'a> {
    /// A cursor whose first chunk holds `offset`.
    fn at(src: &'a dyn ByteSource, offset: usize) -> Self {
        let len = usize::try_from(src.len()).unwrap_or(usize::MAX);
        let mut cursor = Self {
            src,
            len,
            buf: Vec::new(),
            spare: Vec::new(),
            used: 0,
            start: 0,
        };
        // A boundary at or before `offset`.
        let mut start = (offset.min(len) / SOURCE_CHUNK) * SOURCE_CHUNK;
        let mut probe = [0u8; 1];
        for _ in 0..3 {
            if start == 0 || src.read_at(start as u64, &mut probe) == 0 {
                break;
            }
            if !is_continuation(probe[0]) {
                break;
            }
            start -= 1;
        }
        if !cursor.load_from(start) {
            // Nothing to read: an empty haystack (a chunk may be empty only
            // then).
            cursor.len = 0;
        }
        cursor
    }

    /// Makes the chunk starting at `from` current; false (and nothing
    /// changed, as the cursor contract asks) at the end of the data.
    fn load_from(&mut self, from: usize) -> bool {
        let want = (SOURCE_CHUNK + 3).min(self.len.saturating_sub(from));
        if want == 0 {
            return false;
        }
        self.spare.resize(want, 0);
        let n = self.src.read_at(from as u64, &mut self.spare);
        if n == 0 {
            return false;
        }
        let mut end = n.min(SOURCE_CHUNK);
        if n > SOURCE_CHUNK {
            // `spare[end]` starts the next chunk: not inside a character.
            for _ in 0..3 {
                if !is_continuation(self.spare[end]) {
                    break;
                }
                end -= 1;
            }
        }
        std::mem::swap(&mut self.buf, &mut self.spare);
        self.used = end;
        self.start = from;
        true
    }

    /// Makes the chunk ending at `to` current; false (and nothing changed)
    /// at the start.
    fn load_to(&mut self, to: usize) -> bool {
        if to == 0 {
            return false;
        }
        let from = to.saturating_sub(SOURCE_CHUNK);
        self.spare.resize(to - from, 0);
        let n = self.src.read_at(from as u64, &mut self.spare);
        if n < to - from {
            // The data shrank under us. regex-cursor keeps backtracking
            // until it can, so this must not fail: the missing bytes read
            // as zeros (the results are no longer exact anyway).
            self.spare[n..].fill(0);
        }
        let mut skip = 0;
        if from > 0 {
            while skip < 3 && is_continuation(self.spare[skip]) {
                skip += 1;
            }
        }
        self.spare.copy_within(skip.., 0);
        self.spare.truncate(to - from - skip);
        std::mem::swap(&mut self.buf, &mut self.spare);
        self.used = self.buf.len();
        self.start = from + skip;
        true
    }
}

impl Cursor for SourceCursor<'_> {
    fn chunk(&self) -> &[u8] {
        &self.buf[..self.used]
    }

    fn advance(&mut self) -> bool {
        let next = self.start + self.used;
        next < self.len && self.load_from(next)
    }

    fn backtrack(&mut self) -> bool {
        self.start > 0 && self.load_to(self.start)
    }

    fn total_bytes(&self) -> Option<usize> {
        Some(self.len)
    }

    fn offset(&self) -> usize {
        self.start
    }
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

    fn find_iter_source<'a>(
        &'a self,
        src: &'a dyn ByteSource,
        span: Range<usize>,
    ) -> Box<dyn Iterator<Item = Range<usize>> + 'a> {
        let cursor = SourceCursor::at(src, span.start);
        let len = cursor.len;
        let span = span.start.min(len)..span.end.min(len);
        Box::new(
            self.re
                .find_iter(Input::new(cursor).range(span))
                .map(|m| m.range()),
        )
    }

    fn find_iter_bytes<'a>(
        &'a self,
        hay: &'a [u8],
        span: Range<usize>,
    ) -> Box<dyn Iterator<Item = Range<usize>> + 'a> {
        let span = span.start.min(hay.len())..span.end.min(hay.len());
        Box::new(
            self.re
                .find_iter(Input::new(hay).range(span))
                .map(|m| m.range()),
        )
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
        Self::build(query, false)
    }

    /// A searcher for text that keeps its CR LF and CR line endings (a file
    /// read as it is on disk): `^`, `$` and `.` treat `\r` as part of the
    /// line break, as they treat `\n`.
    pub fn new_raw(query: &SearchQuery) -> Result<Self, SearchError> {
        Self::build(query, true)
    }

    fn build(query: &SearchQuery, crlf: bool) -> Result<Self, SearchError> {
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
                    .multi_line(true)
                    .crlf(crlf),
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

    /// The matches inside `range`, found as the iterator is advanced (so a
    /// caller can stop after enough of them).
    pub fn matches<'a>(
        &'a self,
        doc: &'a Document,
        range: Range<ByteOffset>,
    ) -> impl Iterator<Item = Range<ByteOffset>> + 'a {
        self.matcher
            .find_iter(doc, range.start.0..range.end.0)
            .map(to_offsets)
    }

    /// All matches inside `range`.
    pub fn find_all(&self, doc: &Document, range: Range<ByteOffset>) -> Vec<Range<ByteOffset>> {
        self.matcher
            .find_iter(doc, range.start.0..range.end.0)
            .map(to_offsets)
            .collect()
    }

    /// Every match in `hay`: text already in memory, such as a line on
    /// screen.
    pub fn find_all_bytes(&self, hay: &[u8]) -> Vec<Range<usize>> {
        self.matcher.find_iter_bytes(hay, 0..hay.len()).collect()
    }

    /// The matches inside `range` of `src`, found as the iterator is
    /// advanced (reading `src` a chunk at a time).
    pub fn matches_in<'a>(
        &'a self,
        src: &'a dyn ByteSource,
        range: Range<u64>,
    ) -> impl Iterator<Item = Range<u64>> + 'a {
        self.matcher
            .find_iter_source(src, range.start as usize..range.end as usize)
            .map(|m| m.start as u64..m.end as u64)
    }

    /// The first match in `src` starting at or after `from`.
    pub fn find_next_in(&self, src: &dyn ByteSource, from: u64) -> Option<Range<u64>> {
        let len = src.len();
        (from <= len)
            .then(|| self.matches_in(src, from..len).next())
            .flatten()
    }

    /// The last match in `src` starting before `before`, looked for in
    /// growing windows before it. A match may run at most 64 KB past
    /// `before` (the search stops there, so a document without matches
    /// after the caret is not read to its end).
    pub fn find_prev_in(&self, src: &dyn ByteSource, before: u64) -> Option<Range<u64>> {
        let len = src.len();
        let before = before.min(len);
        let end = before.saturating_add(64 * 1024).min(len);
        let mut window: u64 = 64 * 1024;
        loop {
            let start = before.saturating_sub(window);
            let last = self
                .matches_in(src, start..end)
                .take_while(|m| m.start < before)
                .last();
            if last.is_some() || start == 0 {
                return last;
            }
            window = window.saturating_mul(4);
        }
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

    /// Bytes in memory, read like a file; with `shrink_to`, everything from
    /// there on reads as gone (a file truncated during the search).
    struct Bytes {
        data: Vec<u8>,
        shrink_to: Option<usize>,
    }

    impl ByteSource for Bytes {
        fn len(&self) -> u64 {
            self.data.len() as u64
        }

        fn read_at(&self, at: u64, buf: &mut [u8]) -> usize {
            let end = self.shrink_to.unwrap_or(self.data.len());
            let at = (at as usize).min(end);
            let n = buf.len().min(end - at);
            buf[..n].copy_from_slice(&self.data[at..at + n]);
            n
        }
    }

    fn source(text: &str) -> Bytes {
        Bytes {
            data: text.as_bytes().to_vec(),
            shrink_to: None,
        }
    }

    fn found(s: &Searcher, src: &Bytes) -> Vec<(usize, usize)> {
        s.matches_in(src, 0..src.len())
            .map(|m| (m.start as usize, m.end as usize))
            .collect()
    }

    #[test]
    fn sources_are_searched_across_chunk_boundaries() {
        // Matches straddling every alignment of the 1 MB chunk boundary,
        // with multibyte characters around it.
        for shift in 0..8 {
            let pad = SOURCE_CHUNK - 3 - shift;
            let text = format!("{}é needle éé needle", "x".repeat(pad));
            let src = source(&text);
            let s = q("needle", |_| {});
            let expected = all(&s, &text);
            assert_eq!(expected.len(), 2);
            assert_eq!(found(&s, &src), expected, "shift {shift}");
            // Whole words need the characters on both sides of the boundary.
            let w = q("é", |o| o.whole_word = true);
            assert_eq!(found(&w, &src), all(&w, &text), "shift {shift}");
        }
    }

    #[test]
    fn next_and_previous_in_a_source() {
        let text = format!("ab{}ab", "-".repeat(3 * SOURCE_CHUNK));
        let src = source(&text);
        let s = q("ab", |_| {});
        let last = (text.len() - 2) as u64;
        assert_eq!(s.find_next_in(&src, 1).map(|m| m.start), Some(last));
        assert_eq!(s.find_prev_in(&src, src.len()).map(|m| m.start), Some(last));
        assert_eq!(s.find_prev_in(&src, last).map(|m| m.start), Some(0));
        assert_eq!(s.find_prev_in(&src, 0), None);
        assert_eq!(s.find_next_in(&src, last + 1), None);
    }

    #[test]
    fn raw_searchers_treat_cr_as_part_of_the_line_break() {
        let text = "one\r\ntwo\rthree\n";
        let raw = Searcher::new_raw(&SearchQuery::new(
            r"\w+$",
            SearchOptions {
                regex: true,
                ..Default::default()
            },
        ))
        .unwrap();
        let src = source(text);
        assert_eq!(found(&raw, &src), vec![(0, 3), (5, 8), (9, 14)]);
        assert_eq!(raw.find_all_bytes(text.as_bytes()).len(), 3);
    }

    #[test]
    fn a_source_that_shrinks_mid_search_ends_it() {
        let text = format!(
            "{}needle{}needle",
            "x".repeat(SOURCE_CHUNK),
            "y".repeat(2 * SOURCE_CHUNK)
        );
        let mut src = source(&text);
        src.shrink_to = Some(SOURCE_CHUNK + 100);
        let s = q("needle", |_| {});
        assert_eq!(found(&s, &src), vec![(SOURCE_CHUNK, SOURCE_CHUNK + 6)]);
        // Backwards too (regex-cursor backtracks until it can).
        assert!(s.find_prev_in(&src, src.len()).is_some());
        let empty = source("");
        assert_eq!(found(&s, &empty), vec![]);
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
