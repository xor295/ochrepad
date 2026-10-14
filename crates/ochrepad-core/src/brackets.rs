//! Matching brackets: `()`, `[]` and `{}`. Plain text: a bracket inside a
//! string or comment counts like any other. The search is bounded so a caret
//! next to an unmatched bracket in a huge file stays cheap.

use crate::position::ByteOffset;
use crate::text::Document;

/// How far either way to look for the match.
pub const SEARCH_LIMIT: usize = 100_000;

const PAIRS: [(u8, u8); 3] = [(b'(', b')'), (b'[', b']'), (b'{', b'}')];

/// A bracket next to a caret, and its match if there is one in reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BracketPair {
    /// The bracket at the caret.
    pub at: ByteOffset,
    pub matching: Option<ByteOffset>,
    /// Whether the bracket is after the caret (else before it).
    pub after_caret: bool,
}

fn byte_at(doc: &Document, pos: ByteOffset) -> Option<u8> {
    doc.char_at(pos).filter(char::is_ascii).map(|c| c as u8)
}

/// The bracket just after `caret`, or else just before it, and its match.
pub fn bracket_at(doc: &Document, caret: ByteOffset) -> Option<BracketPair> {
    let is_bracket = |b: u8| PAIRS.iter().any(|&(o, c)| b == o || b == c);
    let (at, after_caret) = match byte_at(doc, caret).filter(|&b| is_bracket(b)) {
        Some(_) => (caret, true),
        None if caret.0 > 0 => {
            // A bracket is one byte; inside a longer character there is none.
            let before = ByteOffset(caret.0 - 1);
            if !doc.is_char_boundary(before) {
                return None;
            }
            byte_at(doc, before).filter(|&b| is_bracket(b))?;
            (before, false)
        }
        None => return None,
    };
    Some(BracketPair {
        at,
        matching: find_match(doc, at),
        after_caret,
    })
}

/// The bracket matching the one at `at`, counting nesting of the same kind.
pub fn find_match(doc: &Document, at: ByteOffset) -> Option<ByteOffset> {
    let b = byte_at(doc, at)?;
    // Brackets are ASCII, which never occurs inside a UTF-8 multibyte
    // sequence, so scanning bytes is safe.
    if let Some(&(open, close)) = PAIRS.iter().find(|p| p.0 == b) {
        // The window's edge may fall inside a multibyte character.
        let mut end = (at.0 + 1 + SEARCH_LIMIT).min(doc.len().0);
        while !doc.is_char_boundary(ByteOffset(end)) {
            end -= 1;
        }
        let text = doc.slice(ByteOffset(at.0 + 1)..ByteOffset(end));
        let mut depth = 0usize;
        for (i, c) in text.bytes().enumerate() {
            if c == open {
                depth += 1;
            } else if c == close {
                if depth == 0 {
                    return Some(ByteOffset(at.0 + 1 + i));
                }
                depth -= 1;
            }
        }
        None
    } else {
        let &(open, close) = PAIRS.iter().find(|p| p.1 == b)?;
        let mut start = at.0.saturating_sub(SEARCH_LIMIT);
        while !doc.is_char_boundary(ByteOffset(start)) {
            start += 1;
        }
        let text = doc.slice(ByteOffset(start)..at);
        let mut depth = 0usize;
        for (i, c) in text.bytes().enumerate().rev() {
            if c == close {
                depth += 1;
            } else if c == open {
                if depth == 0 {
                    return Some(ByteOffset(start + i));
                }
                depth -= 1;
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(text: &str, caret: usize) -> Option<(usize, Option<usize>, bool)> {
        bracket_at(&Document::new(text), ByteOffset(caret))
            .map(|p| (p.at.0, p.matching.map(|m| m.0), p.after_caret))
    }

    #[test]
    fn brackets_match_with_nesting_and_either_side_of_the_caret() {
        let t = "f(a[1], (b))";
        assert_eq!(pair(t, 1), Some((1, Some(11), true)));
        assert_eq!(pair(t, 12), Some((11, Some(1), false)));
        assert_eq!(pair(t, 8), Some((8, Some(10), true)));
        assert_eq!(pair(t, 4), Some((3, Some(5), false)), "after the [");
        assert_eq!(
            pair(t, 5),
            Some((5, Some(3), true)),
            "] wins when right after the caret"
        );
        assert_eq!(pair("x(é)", 1), Some((1, Some(4), true)));
        assert_eq!(
            pair("(]", 0),
            Some((0, None, true)),
            "other kinds do not close it"
        );
        assert_eq!(pair("abc", 1), None);
        assert_eq!(pair("İx", 2), None, "after a two-byte character");
    }

    #[test]
    fn the_search_window_never_splits_a_character() {
        // Unmatched brackets with more than the search limit of multibyte
        // text beside them: every alignment of the window's edge.
        for pad in 0..4 {
            let filler = "é".repeat(SEARCH_LIMIT / 2 + 3);
            let open = format!("{}{{{filler}", "x".repeat(pad));
            assert_eq!(pair(&open, pad), Some((pad, None, true)));
            let close = format!("{filler}{}}}", "x".repeat(pad));
            let at = close.len() - 1;
            assert_eq!(pair(&close, at), Some((at, None, true)));
            // 3- and 4-byte characters too.
            let wide = format!("{}[{}", "x".repeat(pad), "漢😀".repeat(SEARCH_LIMIT / 6));
            assert_eq!(pair(&wide, pad), Some((pad, None, true)));
        }
    }
}
