//! Matching tags in HTML and XML: the names in the start and end tag of the
//! element at a caret, which the editor outlines as Notepad++ does. Plain
//! text, like `brackets`: tags are found by scanning the text, not from the
//! syntax highlighting, and only within [`SEARCH_LIMIT`] bytes either way of
//! the caret, so a caret in a huge file stays cheap. The window's start is
//! taken to be outside any comment, quoted value or script.
//!
//! Tags pair by name, counting the nesting of that name only (as Notepad++
//! does), so a `<li>` or `<p>` left without its end tag, as HTML allows, does
//! not stop the elements around it from pairing. Comments, CDATA,
//! declarations (`<!DOCTYPE html>`), processing instructions (`<?xml ?>`,
//! PHP's `<?php ?>`) and server code (`<% %>`) hold no tags, and in HTML
//! neither does the text of `<script>`, `<style>`, `<textarea>` and `<title>`.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;

use crate::position::ByteOffset;
use crate::text::Document;

/// How far either way of the caret to look for tags.
pub const SEARCH_LIMIT: usize = 100_000;

/// HTML's void elements: they have no end tag, so they never pair.
const VOID: [&[u8]; 14] = [
    b"area", b"base", b"br", b"col", b"embed", b"hr", b"img", b"input", b"link", b"meta", b"param",
    b"source", b"track", b"wbr",
];

/// HTML elements whose content is text up to their end tag, not markup.
const RAW_TEXT: [&[u8]; 4] = [b"script", b"style", b"textarea", b"title"];

/// What holds no tags, from its start to its end, in the order tried.
const SKIPPED: [(&[u8], &[u8]); 5] = [
    (b"<!--", b"-->"),
    (b"<![CDATA[", b"]]>"),
    (b"<?", b"?>"),
    (b"<%", b"%>"),
    // `<!DOCTYPE html>` and the declarations of a DTD.
    (b"<!", b">"),
];

/// The names in an element's start and end tag: `div` in `<div class="a">`
/// and in `</div>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagPair {
    pub open: Range<ByteOffset>,
    pub close: Range<ByteOffset>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Start,
    End,
    /// A whole element in one tag: `<x/>`, or one of HTML's void elements.
    Empty,
}

/// A tag in the scanned text: from its `<` to after its `>`, and its name.
struct Tag {
    kind: Kind,
    span: Range<usize>,
    name: Range<usize>,
}

/// The names in the start and end tag of the element at `pos`:
/// - with `pos` in a tag (after its `<`, up to its `>`), that tag and its
///   partner, or `None` if it has none (`<br>`, `<x/>`, an unclosed tag);
/// - beside a tag, that tag if it has a partner (the tag after `pos` first);
/// - otherwise the innermost element around `pos` whose end tag is found.
///
/// `html`: HTML's rules (void elements, names in any case, the text of
/// `<script>` and the like) instead of XML's.
pub fn matching_tags(doc: &Document, pos: ByteOffset, html: bool) -> Option<TagPair> {
    let pos = pos.min(doc.len());
    // The window's edges may fall inside multibyte characters.
    let mut start = pos.0.saturating_sub(SEARCH_LIMIT);
    while !doc.is_char_boundary(ByteOffset(start)) {
        start += 1;
    }
    let mut end = (pos.0 + SEARCH_LIMIT).min(doc.len().0);
    while !doc.is_char_boundary(ByteOffset(end)) {
        end -= 1;
    }
    let text = doc.slice(ByteOffset(start)..ByteOffset(end));
    let tags = scan(text.as_bytes(), html);
    // Names compare as bytes, HTML's in lower case.
    let names = if html {
        Cow::Owned(text.to_ascii_lowercase())
    } else {
        Cow::Borrowed(text.as_str())
    };
    let pairing = Pairing {
        tags: &tags,
        names: names.as_bytes(),
    };
    let (open, close) = pairing.at(pos.0 - start)?;
    let name = |i: usize| {
        let r = &tags[i].name;
        ByteOffset(start + r.start)..ByteOffset(start + r.end)
    };
    Some(TagPair {
        open: name(open),
        close: name(close),
    })
}

/// The tags in `text`, in order. A tag cut off by the end of `text` ends
/// the scan.
fn scan(text: &[u8], html: bool) -> Vec<Tag> {
    // `a < b` is text. XML names may also start with `_`, `:` or a letter
    // outside ASCII.
    let starts_name =
        |b: u8| b.is_ascii_alphabetic() || (!html && (b == b'_' || b == b':' || !b.is_ascii()));
    let mut tags = Vec::new();
    let mut i = 0;
    while let Some(lt) = find(text, i, b"<") {
        let rest = &text[lt..];
        if let Some((open, close)) = SKIPPED.iter().find(|(open, _)| rest.starts_with(open)) {
            i = find(text, lt + open.len(), close).map_or(text.len(), |e| e + close.len());
            continue;
        }
        let (kind, name_start) = match rest.get(1) {
            Some(b'/') => (Kind::End, lt + 2),
            _ => (Kind::Start, lt + 1),
        };
        if !text.get(name_start).is_some_and(|&b| starts_name(b)) {
            i = lt + 1;
            continue;
        }
        let name_end = text[name_start..]
            .iter()
            .position(|&b| !is_name_byte(b))
            .map_or(text.len(), |n| name_start + n);
        let Some((gt, self_closing)) = tag_end(text, name_end) else {
            break;
        };
        let name = &text[name_start..name_end];
        let kind = if (html && VOID.iter().any(|v| name.eq_ignore_ascii_case(v)))
            || (kind == Kind::Start && self_closing)
        {
            Kind::Empty
        } else {
            kind
        };
        tags.push(Tag {
            kind,
            span: lt..gt + 1,
            name: name_start..name_end,
        });
        i = gt + 1;
        if html && kind == Kind::Start && RAW_TEXT.iter().any(|r| name.eq_ignore_ascii_case(r)) {
            i = raw_text_end(text, i, name);
        }
    }
    tags
}

/// Where `needle` next occurs in `text`, from `from` on.
fn find(text: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    let (&first, rest) = needle.split_first()?;
    let mut i = from;
    while let Some(p) = text.get(i..)?.iter().position(|&b| b == first) {
        if text[i + p + 1..].starts_with(rest) {
            return Some(i + p);
        }
        i += p + 1;
    }
    None
}

/// Whether `b` belongs to a tag name. A name ends at a space or one of
/// `/>=<"'`, all ASCII, so never inside a character.
fn is_name_byte(b: u8) -> bool {
    !b.is_ascii_whitespace() && !matches!(b, b'/' | b'>' | b'=' | b'<' | b'"' | b'\'')
}

/// The `>` that ends a tag whose name ends at `from`, and whether the tag
/// closes itself (`/>`). A `>` in a quoted attribute value does not end the
/// tag, and a `/` ending an unquoted one (`href=/`) does not close it.
fn tag_end(text: &[u8], from: usize) -> Option<(usize, bool)> {
    let mut i = from;
    let mut slash = false;
    while let Some(&b) = text.get(i) {
        match b {
            b'>' => return Some((i, slash)),
            b'=' => {
                i += 1;
                while text.get(i).is_some_and(u8::is_ascii_whitespace) {
                    i += 1;
                }
                match text.get(i) {
                    Some(&quote @ (b'"' | b'\'')) => {
                        i += text[i + 1..].iter().position(|&c| c == quote)? + 2;
                    }
                    _ => {
                        while text
                            .get(i)
                            .is_some_and(|&c| !c.is_ascii_whitespace() && c != b'>')
                        {
                            i += 1;
                        }
                    }
                }
                slash = false;
            }
            _ => {
                slash = b == b'/';
                i += 1;
            }
        }
    }
    None
}

/// Where the text of a raw text element named `name` ends: at its end tag
/// (`</script` in any case) from `from` on, or at the end of `text`.
fn raw_text_end(text: &[u8], from: usize, name: &[u8]) -> usize {
    let mut i = from;
    while let Some(lt) = find(text, i, b"</") {
        let after = lt + 2 + name.len();
        if text
            .get(lt + 2..after)
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
            && !text.get(after).is_some_and(|&b| is_name_byte(b))
        {
            return lt;
        }
        i = lt + 2;
    }
    text.len()
}

/// Pairs the tags `scan` found. `names` is the scanned text as names
/// compare: lower-cased for HTML.
struct Pairing<'a> {
    tags: &'a [Tag],
    names: &'a [u8],
}

impl Pairing<'_> {
    fn name(&self, i: usize) -> &[u8] {
        &self.names[self.tags[i].name.clone()]
    }

    /// The start and end tag (as indices) of the element at `caret`.
    fn at(&self, caret: usize) -> Option<(usize, usize)> {
        // The tags before `before` end at or before the caret.
        let before = self.tags.partition_point(|t| t.span.end <= caret);
        let next = self.tags.get(before);
        // In a tag: its element or nothing.
        if next.is_some_and(|t| t.span.start < caret) {
            return self.partner(before);
        }
        // Beside tags: the one after the caret, then the one before, if it
        // pairs; else the element around the caret.
        let after = next.filter(|t| t.span.start == caret).map(|_| before);
        let beside = before
            .checked_sub(1)
            .filter(|&i| self.tags[i].span.end == caret);
        after
            .into_iter()
            .chain(beside)
            .find_map(|i| self.partner(i))
            .or_else(|| self.around(before))
    }

    /// The start and end tag of the element tag `i` belongs to.
    fn partner(&self, i: usize) -> Option<(usize, usize)> {
        let name = self.name(i);
        match self.tags[i].kind {
            Kind::Start => self
                .unnested(i + 1..self.tags.len(), name, Kind::Start)
                .map(|j| (i, j)),
            Kind::End => self.unnested((0..i).rev(), name, Kind::End).map(|j| (j, i)),
            Kind::Empty => None,
        }
    }

    /// The first tag named `name` in `order` that is not nested: tags of
    /// kind `nest` open a level, the others close one.
    fn unnested(
        &self,
        order: impl Iterator<Item = usize>,
        name: &[u8],
        nest: Kind,
    ) -> Option<usize> {
        let mut depth = 0usize;
        for j in order {
            let kind = self.tags[j].kind;
            if kind == Kind::Empty || self.name(j) != name {
                continue;
            }
            if kind == nest {
                depth += 1;
            } else if depth == 0 {
                return Some(j);
            } else {
                depth -= 1;
            }
        }
        None
    }

    /// The innermost element around a caret between the tags before
    /// `before` and the rest, of those whose end tag is found. Start tags
    /// that are never closed (HTML's `<li>` without `</li>`) are passed
    /// over.
    fn around(&self, before: usize) -> Option<(usize, usize)> {
        // After the caret, by name: how deep the elements opened there are,
        // and the first end tag of an element opened before it.
        let mut ends: HashMap<&[u8], (usize, Option<usize>)> = HashMap::new();
        for (j, t) in self.tags.iter().enumerate().skip(before) {
            if t.kind == Kind::Empty {
                continue;
            }
            let (depth, first) = ends.entry(self.name(j)).or_default();
            match t.kind {
                Kind::Start => *depth += 1,
                _ if *depth > 0 => *depth -= 1,
                _ => {
                    first.get_or_insert(j);
                }
            }
        }
        // Before it, nearest first, by name: how deep the elements closed
        // there are. A start tag at depth 0 is open at the caret, and ends
        // at the first end tag found for its name after the caret; without
        // one, no element further out with its name is closed either.
        let mut depths: HashMap<&[u8], usize> = HashMap::new();
        for j in (0..before).rev() {
            let kind = self.tags[j].kind;
            if kind == Kind::Empty {
                continue;
            }
            let name = self.name(j);
            let depth = depths.entry(name).or_default();
            match kind {
                Kind::End => *depth += 1,
                _ if *depth > 0 => *depth -= 1,
                _ => {
                    if let Some(&(_, Some(end))) = ends.get(name) {
                        return Some((j, end));
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HTML: bool = true;
    const XML: bool = false;

    /// Checks `matching_tags` on `marked` with its marks taken out: `|` is
    /// the caret, and `‹` `›` surround the two names expected (no names:
    /// nothing matches).
    fn check(marked: &str, html: bool) {
        let mut text = String::new();
        let mut caret = None;
        let mut marks = Vec::new();
        for c in marked.chars() {
            match c {
                '|' => caret = Some(ByteOffset(text.len())),
                '‹' | '›' => marks.push(ByteOffset(text.len())),
                _ => text.push(c),
            }
        }
        let expected = match marks[..] {
            [a, b, c, d] => Some(TagPair {
                open: a..b,
                close: c..d,
            }),
            [] => None,
            _ => panic!("two names or none: {marked}"),
        };
        let caret = caret.unwrap_or_else(|| panic!("no caret: {marked}"));
        let found = matching_tags(&Document::new(&text), caret, html);
        assert_eq!(found, expected, "{marked} (html: {html})");
    }

    #[test]
    fn a_tag_with_the_caret_in_it_matches_its_partner() {
        // Anywhere from after the `<` to the `>`, attributes included.
        for marked in [
            "<|‹div› class=\"a\">x</‹div›>",
            "<‹d|iv› class=\"a\">x</‹div›>",
            "<‹div› cl|ass=\"a\">x</‹div›>",
            "<‹div› class=\"a\"|>x</‹div›>",
            "<‹div› class=\"a\">x<|/‹div›>",
            "<‹div› class=\"a\">x</‹di|v›>",
            "<‹div› class=\"a\">x</‹div›|>",
        ] {
            check(marked, HTML);
            check(marked, XML);
        }
    }

    #[test]
    fn nesting_of_the_same_name_is_counted() {
        check("<‹div›|><div><div></div></div><p></p></‹div›>", HTML);
        check("<‹div›><div><div></div></div><p></p><|/‹div›>", HTML);
        check("<div><div><‹d|iv›></‹div›></div></div>", HTML);
        check("<div><‹div›><div></div><|/‹div›></div>", XML);
        // Other names don't count: tags pair by name.
        check("<‹a›|><b></‹a›></b>", XML);
        check("<a><‹b›|></a></‹b›>", XML);
    }

    #[test]
    fn between_tags_the_innermost_element_around_the_caret() {
        check("<‹ul›>\n  <li>a</li>\n | <li>b</li>\n</‹ul›>", HTML);
        check("<‹p›>Some <b>bold</b> te|xt</‹p›>", HTML);
        check("<p>Some <‹b›>bo|ld</‹b›> text</p>", HTML);
        check("<‹div›><p>a</p><p>b</p> | </‹div›>", XML);
        check("<p>a</p> | <p>b</p>", HTML);
        // Elements of the same name before or after the caret are skipped.
        check("<‹div›><div>a</div> | </‹div›>", HTML);
        check("<‹div›> | <div>a</div></‹div›>", HTML);
        check("<div><‹div›>a | <div>b</div></‹div›>", XML);
    }

    #[test]
    fn beside_a_tag_the_caret_matches_it_the_one_after_first() {
        check("|<‹b›>x</‹b›>", HTML);
        check("<‹b›>x</‹b›>|", HTML);
        check("<b>x</b>|<‹i›>y</‹i›>", HTML);
        check("<p><‹b›>x</‹b›>| y</p>", HTML);
        check("<‹div›>x</‹div›>|</div>", HTML);
        // A tag beside the caret without a partner is passed over.
        check("<‹p›>a<br>|b</‹p›>", HTML);
        check("<‹ul›><li>|one</‹ul›>", HTML);
        check("<‹ul›><li>one|<li>two</‹ul›>", HTML);
    }

    #[test]
    fn quoted_values_may_hold_angle_brackets() {
        check("<‹a› title=\"x > y\" data-x='<b>'>t|</‹a›>", HTML);
        check("<‹a› title = \"x >| y\">t</‹a›>", XML);
        // Only a value's quotes count, and a `/` ending an unquoted value
        // does not close the tag.
        check("<‹p› don't>x|</‹p›>", HTML);
        check("<‹a› href=/x/>t|</‹a›>", HTML);
        // A value that never closes runs past the end: no tag.
        check("<a title=\"x>y</a>|", HTML);
    }

    #[test]
    fn comments_cdata_declarations_and_code_hold_no_tags() {
        for skipped in [
            "<!-- <di|v> -->",
            "<!-- </di|v> -->",
            "<![CDATA[ </di|v> ]]>",
            "<?php if ($a <di|v) echo '</div>'; ?>",
            "<% if a <di|v %>",
            "<!ENTITY x \"<di|v>\">",
        ] {
            check(&format!("<!DOCTYPE html>\n<‹div›>{skipped}</‹div›>"), HTML);
            check(
                &format!("<?xml version=\"1.0\"?><‹div›>{skipped}</‹div›>"),
                XML,
            );
        }
        // Left open, they run to the end.
        check("<div>|<!-- </div>", HTML);
    }

    #[test]
    fn self_closing_and_void_elements_never_pair() {
        // In one, nothing matches; beside one, the element around it.
        check("<p>a<b|r>b</p>", HTML);
        check("<p>a<x/|>b</p>", XML);
        check("<B|R>x</BR>", HTML);
        check("<‹p›>a<br>b<img src=\"x\" />|c</‹p›>", HTML);
        check("<‹div›><div/>|</‹div›>", HTML);
        check("<‹div›><div />x|</‹div›>", XML);
        check("<‹p›>a</br>b|</‹p›>", HTML);
        // XML has no void elements.
        check("<‹B|R›>x</‹BR›>", XML);
    }

    #[test]
    fn html_names_match_in_any_case_xml_names_exactly() {
        check("<‹Div›>x|</‹dIV›>", HTML);
        check("<Div>x|</dIV>", XML);
        check("<‹Div›><div>x</div>|</‹Div›>", XML);
    }

    #[test]
    fn the_text_of_script_and_style_is_not_markup_in_html() {
        check("<‹script›>if (a <b) w(\"</div>\")|</‹script›>", HTML);
        check(
            "<‹div›><style>p > a {}</style><script>x = '<div>'</script>|</‹div›>",
            HTML,
        );
        check("<‹div›><SCRIPT>'</div>'</Script >|</‹div›>", HTML);
        check("<‹script›>a <b>|</b></‹script›>", HTML);
        check("<script>a <‹b›>|</‹b›></script>", XML);
    }

    #[test]
    fn unbalanced_markup_matches_nothing() {
        for marked in [
            "<div>|",
            "|</div>",
            "<d|iv><div></div>",
            "<a>|</b>",
            "<a>x|</b>",
            "</div>x|<div>",
            "<ul><l|i>one<li>two</li></ul>",
        ] {
            check(marked, HTML);
            check(marked, XML);
        }
    }

    #[test]
    fn at_the_ends_of_the_document() {
        check("|", HTML);
        check("|<‹a›>x</‹a›>", XML);
        check("<‹a›>x</‹a›>|", XML);
        // Cut off at the end: no tag.
        check("<a>x</a|", HTML);
        check("<a>x|</a", HTML);
        check("<|", HTML);
        check("x<a|", XML);
    }

    #[test]
    fn names_and_text_outside_ascii() {
        check("<‹p›>é漢😀|</‹p›>", HTML);
        check("<‹p›>é|漢😀</‹p›>", HTML);
        check("<‹café›>ü|</‹café›>", XML);
        check("<‹café›>ü|</‹CAFé›>", HTML);
        // HTML names start with an ASCII letter; XML's need not.
        check("<‹p›><é>x|</é></‹p›>", HTML);
        check("<p><‹é›>x|</‹é›></p>", XML);
    }

    #[test]
    fn the_search_is_bounded() {
        let pair =
            |text: &str, pos: usize| matching_tags(&Document::new(text), ByteOffset(pos), HTML);
        // An end tag in reach either way is found; one further away is not.
        let near = format!("<a>{}</a>", "x".repeat(SEARCH_LIMIT - 100));
        assert!(pair(&near, 1).is_some());
        assert!(pair(&near, near.len() - 2).is_some());
        let far = format!("<a>{}</a>", "x".repeat(SEARCH_LIMIT + 100));
        assert_eq!(pair(&far, 1), None);
        assert_eq!(pair(&far, far.len() - 2), None);
        // The caret in `<b>`, with multibyte text across both edges of the
        // window: every alignment of each edge.
        let fill = "é漢😀".repeat(SEARCH_LIMIT / 9 + 2);
        for pad in 0..9 {
            let x = "x".repeat(pad);
            let text = format!("<a>{fill}{x}<b>{x}{fill}</a>");
            assert_eq!(pair(&text, 3 + fill.len() + pad + 1), None);
        }
    }
}
