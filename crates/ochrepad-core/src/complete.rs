//! Completion (PLAN_completion): matching what was typed against
//! candidates, ranking them, and the words of a text. Where candidates come
//! from (documents, keywords, a folder index, language servers) is the
//! app's business; this decides which match and in what order.
//!
//! A candidate matches the typed pattern as a prefix (best), by the
//! initials of its parts (`gTL`, `gtl` for `getTextLength` or
//! `get_text_length`), or, for patterns of 3 characters or more, as any
//! subsequence that starts at a part. Matching ignores case; matching case
//! scores a little more.

use crate::position::ByteOffset;
use crate::text::Document;

/// What a candidate names (shown as a marker in the popup).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Keyword,
    Function,
    Type,
    Module,
    Variable,
    Field,
    Constant,
    Snippet,
    Path,
    /// A word found in some text.
    Word,
    Other,
}

/// Where a candidate came from, best first (ties in matching are broken by
/// this).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Source {
    Server,
    Definition,
    Keyword,
    Word,
    Path,
}

impl Source {
    fn weight(self) -> i64 {
        match self {
            Source::Server => 40,
            Source::Definition => 30,
            Source::Keyword => 20,
            Source::Path => 20,
            Source::Word => 0,
        }
    }
}

/// Something the popup can offer.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    /// What is shown, and matched against.
    pub label: String,
    pub kind: Kind,
    pub source: Source,
    /// Shown dimmed after the label (where a name was found, a type).
    pub detail: Option<String>,
    /// Inserted instead of the label, when different.
    pub insert: Option<String>,
    /// How likely the source thinks it is, 0.0 to 1.0 (a word seen near the
    /// caret, used often, or accepted recently).
    pub boost: f32,
}

impl Candidate {
    pub fn new(label: impl Into<String>, kind: Kind, source: Source) -> Self {
        Self {
            label: label.into(),
            kind,
            source,
            detail: None,
            insert: None,
            boost: 0.0,
        }
    }

    /// The text accepting it inserts.
    pub fn text(&self) -> &str {
        self.insert.as_deref().unwrap_or(&self.label)
    }
}

/// A candidate that matched, and how well.
#[derive(Clone, Debug, PartialEq)]
pub struct Ranked {
    pub candidate: Candidate,
    pub score: i64,
}

/// Whether `c` starts a part of `label` at char index `i` (`prev` is the
/// char before it): the label's start, after a separator, a lower-case
/// letter or digit followed by an upper-case one (camelCase), a letter
/// followed by a digit, or the last capital of a run before lower case
/// (`HTTPServer`: H, S).
fn part_start(prev: Option<char>, c: char, next: Option<char>) -> bool {
    let Some(p) = prev else {
        return true;
    };
    if !p.is_alphanumeric() {
        return c.is_alphanumeric();
    }
    (c.is_uppercase() && (p.is_lowercase() || p.is_numeric()))
        || (c.is_numeric() && !p.is_numeric())
        || (c.is_uppercase() && p.is_uppercase() && next.is_some_and(char::is_lowercase))
}

/// Points per matched character, and bonuses.
const CHAR: i64 = 1;
const SAME_CASE: i64 = 1;
const AT_PART: i64 = 8;
const RUN: i64 = 8;
const AT_START: i64 = 12;
const PREFIX: i64 = 30;
const LEADING: i64 = 2;
const GAP: i64 = 1;
const MAX_GAP_PENALTY: i64 = 10;

/// How well `pattern` matches `label` (higher is better), or `None`. Both
/// are compared as chars; `pattern` must not be empty.
pub fn match_score(pattern: &str, label: &str) -> Option<i64> {
    match_positions(pattern, label, false).map(|(score, _)| score)
}

/// [`match_score`], with the char indices of `label` the pattern matched
/// (for drawing them).
pub fn match_with_positions(pattern: &str, label: &str) -> Option<(i64, Vec<usize>)> {
    match_positions(pattern, label, true)
}

fn lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn match_positions(pattern: &str, label: &str, want: bool) -> Option<(i64, Vec<usize>)> {
    let p: Vec<char> = pattern.chars().collect();
    let l: Vec<char> = label.chars().collect();
    if p.is_empty() || p.len() > l.len() || l.len() > 256 {
        return None;
    }
    // Quick refusal: the pattern must be a subsequence, ignoring case, and
    // its first char must be the label's first or a part's start.
    let pl: Vec<char> = p.iter().map(|&c| lower(c)).collect();
    let ll: Vec<char> = l.iter().map(|&c| lower(c)).collect();
    {
        let mut i = 0;
        for &c in &ll {
            if i < pl.len() && c == pl[i] {
                i += 1;
            }
        }
        if i < pl.len() {
            return None;
        }
    }
    let starts: Vec<bool> = (0..l.len())
        .map(|j| part_start(j.checked_sub(1).map(|k| l[k]), l[j], l.get(j + 1).copied()))
        .collect();
    // A prefix, ignoring case: the best kind of match.
    if ll[..pl.len()] == pl[..] {
        let same = p.iter().zip(&l).filter(|(a, b)| a == b).count() as i64;
        let score = PREFIX
            + AT_START
            + pl.len() as i64 * (CHAR + RUN)
            + same * SAME_CASE
            - (l.len() - p.len()) as i64 / 4;
        return Some((score, if want { (0..p.len()).collect() } else { Vec::new() }));
    }
    // Otherwise the best placement: dp[i][j] is the best score with
    // pattern char i at label char j. Short patterns only match at part
    // starts or right after the previous match (initials, not scattered
    // letters).
    let short = p.len() < 3;
    const NONE: i64 = i64::MIN / 4;
    let (n, m) = (p.len(), l.len());
    let mut dp = vec![NONE; n * m];
    let mut from = vec![usize::MAX; if want { n * m } else { 0 }];
    for i in 0..n {
        for j in i..m {
            if ll[j] != pl[i] {
                continue;
            }
            let mut gain = CHAR + i64::from(l[j] == p[i]) * SAME_CASE;
            if starts[j] {
                gain += AT_PART;
            }
            // Still on the label's unbroken start.
            if j == i {
                gain += LEADING;
            }
            if i == 0 {
                if !starts[j] {
                    continue;
                }
                if j == 0 {
                    gain += AT_START;
                }
                dp[j] = gain;
                continue;
            }
            // Right after the previous match: a run.
            let mut best = NONE;
            let mut came = usize::MAX;
            let prev = dp[(i - 1) * m + j - 1];
            if prev > NONE {
                best = prev + gain + RUN;
                came = j - 1;
            }
            // After a gap: only at a part start for short patterns. Labels
            // are short, so every earlier place is tried.
            if !short || starts[j] {
                for k in (i - 1)..j.saturating_sub(1) {
                    let s = dp[(i - 1) * m + k];
                    if s == NONE {
                        continue;
                    }
                    let gap = (j - k - 1) as i64;
                    let s = s + gain - (gap * GAP).min(MAX_GAP_PENALTY);
                    if s > best {
                        best = s;
                        came = k;
                    }
                }
            }
            if best > NONE {
                dp[i * m + j] = best;
                if want {
                    from[i * m + j] = came;
                }
            }
        }
    }
    let (end, score) = (0..m)
        .map(|j| (j, dp[(n - 1) * m + j]))
        .max_by_key(|&(_, s)| s)
        .filter(|&(_, s)| s > NONE)?;
    let mut positions = Vec::new();
    if want {
        positions.resize(n, 0);
        let mut j = end;
        for i in (0..n).rev() {
            positions[i] = j;
            if i > 0 {
                j = from[i * m + j];
            }
        }
    }
    // Longer labels rank a little lower.
    Some((score - (m - n) as i64 / 4, positions))
}

/// The candidates that match `pattern`, best first, without duplicate
/// labels (the best-ranked one is kept). A candidate equal to the pattern
/// is left out: there is nothing to complete. At most `limit` are kept.
pub fn rank(pattern: &str, candidates: impl IntoIterator<Item = Candidate>, limit: usize) -> Vec<Ranked> {
    let mut ranked: Vec<Ranked> = candidates
        .into_iter()
        .filter(|c| c.label != pattern)
        .filter_map(|c| {
            let quality = match_score(pattern, &c.label)?;
            let score =
                quality * 100 + c.source.weight() * 10 + (c.boost.clamp(0.0, 1.0) * 100.0) as i64;
            Some(Ranked {
                candidate: c,
                score,
            })
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.candidate.source.cmp(&b.candidate.source))
            .then_with(|| a.candidate.label.len().cmp(&b.candidate.label.len()))
            .then_with(|| a.candidate.label.cmp(&b.candidate.label))
    });
    let mut seen = std::collections::HashSet::new();
    ranked.retain(|r| seen.insert(r.candidate.label.clone()));
    ranked.truncate(limit);
    ranked
}

/// Whether `c` belongs in a word: a letter, a digit, `_`, or one of
/// `extra` (`-` in CSS and Lisp, `$` in PHP and shell).
pub fn is_word_char(c: char, extra: &[char]) -> bool {
    c.is_alphanumeric() || c == '_' || extra.contains(&c)
}

/// The start of the word fragment that ends at `caret`, on the caret's
/// line: the text between it and the caret is what was typed so far.
pub fn word_start(doc: &Document, caret: ByteOffset, extra: &[char]) -> ByteOffset {
    let line = doc.line_of(caret);
    let start = doc.line_start(line);
    let text = doc.slice(start..caret);
    let back: usize = text
        .chars()
        .rev()
        .take_while(|&c| is_word_char(c, extra))
        .map(char::len_utf8)
        .sum();
    ByteOffset(caret.0 - back)
}

/// The words of `text`: runs of word chars at least `min_len` chars long
/// that do not start with a digit.
pub fn words<'a>(text: &'a str, extra: &'a [char], min_len: usize) -> impl Iterator<Item = &'a str> + 'a {
    text.split(move |c: char| !is_word_char(c, extra))
        .filter(move |w| {
            !w.is_empty()
                && !w.starts_with(|c: char| c.is_numeric())
                && w.chars().nth(min_len.saturating_sub(1)).is_some()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn best(pattern: &str, labels: &[&str]) -> Vec<String> {
        rank(
            pattern,
            labels.iter().map(|l| Candidate::new(*l, Kind::Word, Source::Word)),
            50,
        )
        .into_iter()
        .map(|r| r.candidate.label)
        .collect()
    }

    #[test]
    fn prefixes_win_then_initials() {
        let labels = [
            "getTextLength",
            "get_text_length",
            "gutter",
            "settle",
            "GTLookup",
            "getter",
        ];
        assert_eq!(best("get", &labels)[..3], ["getter", "getTextLength", "get_text_length"]);
        let r = best("gtl", &labels);
        assert!(r.contains(&"getTextLength".to_owned()));
        assert!(r.contains(&"get_text_length".to_owned()));
        assert!(r.contains(&"GTLookup".to_owned()));
        assert!(!r.contains(&"settle".to_owned()), "{r:?}");
        assert_eq!(r[0], "GTLookup", "a prefix, ignoring case");
    }

    #[test]
    fn short_patterns_match_initials_not_scattered_letters() {
        assert!(match_score("gt", "getText").is_some());
        assert!(match_score("gt", "gutter").is_none(), "t is not a part start");
        assert!(match_score("gt", "giraffe_tail").is_some());
        assert!(match_score("gr", "magnifier").is_none(), "not at a part start");
        assert!(match_score("xy", "xylophone").is_some());
        // Three or more: any subsequence from a part start.
        assert!(match_score("gtt", "gutter").is_some());
        assert!(match_score("utt", "gutter").is_none(), "must start at a part");
    }

    #[test]
    fn parts_of_names() {
        assert!(part_start(Some('t'), 'L', Some('e')));
        assert!(part_start(Some('_'), 'l', Some('e')));
        assert!(part_start(Some('P'), 'S', Some('e')), "HTTPServer: S");
        assert!(!part_start(Some('H'), 'T', Some('T')));
        assert!(part_start(Some('v'), '2', None));
        let (_, at) = match_with_positions("hs", "HTTPServer").unwrap();
        assert_eq!(at, [0, 4]);
        let (_, at) = match_with_positions("gtl", "get_text_length").unwrap();
        assert_eq!(at, [0, 4, 9]);
        let (_, at) = match_with_positions("getl", "getTextLength").unwrap();
        assert_eq!(at, [0, 1, 2, 7]);
    }

    #[test]
    fn matching_case_and_sources_break_ties() {
        let r = rank(
            "Str",
            [
                Candidate::new("string", Kind::Word, Source::Word),
                Candidate::new("String", Kind::Type, Source::Definition),
            ],
            10,
        );
        assert_eq!(r[0].candidate.label, "String");
        let r = rank(
            "fo",
            [
                Candidate::new("foo", Kind::Word, Source::Word),
                Candidate::new("for", Kind::Keyword, Source::Keyword),
            ],
            10,
        );
        assert_eq!(r[0].candidate.label, "for");
    }

    #[test]
    fn duplicates_and_the_typed_word_are_dropped() {
        let r = rank(
            "count",
            [
                Candidate::new("count", Kind::Word, Source::Word),
                Candidate::new("counter", Kind::Word, Source::Word),
                Candidate::new("counter", Kind::Variable, Source::Definition),
            ],
            10,
        );
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].candidate.source, Source::Definition);
    }

    #[test]
    fn multibyte_labels_and_patterns() {
        assert!(match_score("ü", "über").is_some());
        assert!(match_score("Ü", "über").is_some());
        let (_, at) = match_with_positions("éa", "été_août").unwrap();
        assert_eq!(at, [0, 4]);
        assert!(match_score("日本", "日本語").is_some());
    }

    #[test]
    fn the_word_before_the_caret() {
        let doc = Document::new("let some_value = x.ab\nnext");
        let caret = ByteOffset(21);
        let start = word_start(&doc, caret, &[]);
        assert_eq!(doc.slice(start..caret), "ab");
        let caret = ByteOffset(14);
        assert_eq!(doc.slice(word_start(&doc, caret, &[])..caret), "some_value");
        // Not across lines.
        let caret = ByteOffset(22);
        assert_eq!(word_start(&doc, caret, &[]), caret);
        let doc = Document::new("color: var(--main-bg");
        let caret = doc.len();
        assert_eq!(doc.slice(word_start(&doc, caret, &['-'])..caret), "--main-bg");
    }

    #[test]
    fn words_of_a_text() {
        let w: Vec<&str> = words("fn main() { let x2 = 42; über_alles(x2) }", &[], 2).collect();
        assert_eq!(w, ["fn", "main", "let", "x2", "über_alles", "x2"]);
    }
}
