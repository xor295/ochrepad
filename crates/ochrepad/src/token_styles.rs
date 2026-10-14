//! Search > Style All Occurrences of Token, as in Notepad++: the selected
//! text (or the word at the caret) is highlighted everywhere in the
//! document in one of five colours, until that style is cleared. Unlike
//! Notepad++, occurrences typed later are highlighted too: each style keeps
//! its tokens and the editor finds them on the lines it draws, as it does
//! for find matches. Jump Up and Jump Down go to the previous or next
//! occurrence of a style's tokens.

use std::collections::HashMap;
use std::ops::Range;

use egui::{Color32, Visuals};
use ochrepad_core::{
    BufferId, ByteOffset, Document, SearchOptions, SearchQuery, Searcher, SelectionSet,
};

/// How many styles there are.
pub const STYLES: usize = 5;

/// The menus' names for the styles.
pub const NAMES: [&str; STYLES] = ["1st", "2nd", "3rd", "4th", "5th"];

/// Tokens longer than this are not styled (a token is for a word or so).
const MAX_TOKEN: usize = 1000;

/// One styled token.
struct Token {
    text: String,
    style: usize,
    searcher: Searcher,
}

#[derive(Default)]
pub struct TokenStyles {
    docs: HashMap<BufferId, Vec<Token>>,
}

/// The colour of style `style` (0-based): sky, violet, yellow, rose and
/// green, none of them ochre (the selection) or teal (find matches).
pub fn color(style: usize, visuals: &Visuals) -> Color32 {
    let (r, g, b) = [
        (80, 165, 240),
        (160, 110, 230),
        (225, 200, 60),
        (230, 100, 150),
        (90, 185, 100),
    ][style % STYLES];
    let alpha = if visuals.dark_mode { 80 } else { 95 };
    Color32::from_rgba_unmultiplied(r, g, b, alpha)
}

/// The token a style command works on: the primary selection if it is
/// on one line, else the word at the caret.
pub fn token_at(doc: &Document, sels: &SelectionSet) -> Option<String> {
    let sel = sels.primary();
    let range = if sel.is_empty() {
        let (start, end) = ochrepad_core::word_at(doc, sel.head);
        start..end
    } else {
        sel.range()
    };
    let text = doc.slice(range);
    let ok = !text.trim().is_empty() && !text.contains('\n') && text.len() <= MAX_TOKEN;
    ok.then_some(text)
}

impl TokenStyles {
    /// Styles every occurrence of `token` in `buffer` with `style`
    /// (replacing another style it had). Whole words match only whole
    /// words; case always matters.
    pub fn add(&mut self, buffer: BufferId, token: String, style: usize) {
        let whole_word = token.chars().all(|c| c.is_alphanumeric() || c == '_');
        let query = SearchQuery::new(
            ochrepad_core::search::escape(&token),
            SearchOptions {
                regex: true,
                match_case: true,
                whole_word,
                wrap: true,
                in_selection: false,
            },
        );
        let Ok(searcher) = Searcher::new(&query) else {
            return;
        };
        let tokens = self.docs.entry(buffer).or_default();
        tokens.retain(|t| t.text != token);
        tokens.push(Token {
            text: token,
            style: style % STYLES,
            searcher,
        });
    }

    /// Clears style `style` in `buffer`, or every style with `None`.
    pub fn clear(&mut self, buffer: BufferId, style: Option<usize>) {
        if let Some(tokens) = self.docs.get_mut(&buffer) {
            tokens.retain(|t| style.is_some_and(|s| t.style != s));
        }
    }

    /// Whether `buffer` has tokens of `style`.
    pub fn has(&self, buffer: BufferId, style: usize) -> bool {
        self.docs
            .get(&buffer)
            .is_some_and(|t| t.iter().any(|t| t.style == style))
    }

    /// The tokens of `buffer`: (text, style).
    pub fn tokens(&self, buffer: BufferId) -> Vec<(&str, usize)> {
        self.docs.get(&buffer).map_or_else(Vec::new, |t| {
            t.iter().map(|t| (t.text.as_str(), t.style)).collect()
        })
    }

    /// Searchers and colours for the editor to highlight.
    pub fn highlights(&self, buffer: BufferId, visuals: &Visuals) -> Vec<(&Searcher, Color32)> {
        self.docs.get(&buffer).map_or_else(Vec::new, |t| {
            t.iter()
                .map(|t| (&t.searcher, color(t.style, visuals)))
                .collect()
        })
    }

    /// Forgets the styles of buffers that are gone.
    pub fn retain(&mut self, alive: impl Fn(BufferId) -> bool) {
        self.docs.retain(|b, t| alive(*b) && !t.is_empty());
    }

    /// Jump Down/Up: the next occurrence of a `style` token after `from`
    /// (or the previous one before it), going round the document.
    pub fn jump(
        &self,
        buffer: BufferId,
        doc: &Document,
        style: usize,
        from: Range<ByteOffset>,
        backward: bool,
    ) -> Option<Range<ByteOffset>> {
        let tokens: Vec<&Token> = self
            .docs
            .get(&buffer)?
            .iter()
            .filter(|t| t.style == style)
            .collect();
        let pick = |found: Vec<Range<ByteOffset>>| {
            if backward {
                found.into_iter().max_by_key(|m| m.start)
            } else {
                found.into_iter().min_by_key(|m| m.start)
            }
        };
        let near = |at: ByteOffset| -> Vec<Range<ByteOffset>> {
            tokens
                .iter()
                .filter_map(|t| {
                    if backward {
                        t.searcher.find_prev(doc, at)
                    } else {
                        t.searcher.find_next(doc, at)
                    }
                })
                .filter(|m| *m != from)
                .collect()
        };
        let (here, round) = if backward {
            (from.start, ByteOffset(doc.len().0 + 1))
        } else {
            (from.end, ByteOffset(0))
        };
        pick(near(here)).or_else(|| pick(near(round)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jumping_goes_round_the_tokens_of_a_style() {
        let doc = Document::new("alpha beta alpha gamma beta");
        let b = BufferId(1);
        let mut s = TokenStyles::default();
        s.add(b, "alpha".into(), 0);
        s.add(b, "beta".into(), 0);
        s.add(b, "gamma".into(), 1);
        let at = |a, z| ByteOffset(a)..ByteOffset(z);
        assert_eq!(s.jump(b, &doc, 0, at(0, 5), false), Some(at(6, 10)));
        assert_eq!(s.jump(b, &doc, 0, at(6, 10), false), Some(at(11, 16)));
        assert_eq!(s.jump(b, &doc, 0, at(23, 27), false), Some(at(0, 5)));
        assert_eq!(s.jump(b, &doc, 0, at(0, 5), true), Some(at(23, 27)));
        assert_eq!(s.jump(b, &doc, 1, at(0, 0), false), Some(at(17, 22)));
        s.clear(b, Some(0));
        assert!(!s.has(b, 0) && s.has(b, 1));
        s.clear(b, None);
        assert!(s.tokens(b).is_empty());
    }

    #[test]
    fn a_token_styled_again_changes_style() {
        let b = BufferId(1);
        let mut s = TokenStyles::default();
        s.add(b, "x".into(), 0);
        s.add(b, "x".into(), 3);
        assert_eq!(s.tokens(b), vec![("x", 3)]);
    }
}
