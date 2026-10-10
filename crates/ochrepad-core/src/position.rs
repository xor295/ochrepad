//! Position newtypes and the conversions between them.
//!
//! This is the only module that knows about tab stops, grapheme clusters and
//! display widths. The canonical position everywhere else is [`ByteOffset`].

use serde::{Deserialize, Serialize};
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete, UnicodeSegmentation};
use unicode_width::UnicodeWidthStr;

use crate::text::Document;

/// Byte offset into the document's UTF-8 text, always on a char boundary.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct ByteOffset(pub usize);

/// Char index; only used at the boundary with Ropey 1.x.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CharIdx(pub usize);

/// A line and a byte column within that line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LineCol {
    pub line: usize,
    pub col: ByteOffset,
}

/// On-screen cell column, after tab expansion and wide-glyph widths.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct VisualCol(pub usize);

pub fn byte_to_line_col(doc: &Document, pos: ByteOffset) -> LineCol {
    let line = doc.line_of(pos);
    LineCol {
        line,
        col: ByteOffset(pos.0 - doc.line_start(line).0),
    }
}

/// Clamps the column to the end of the line. The column must land on a char
/// boundary; out-of-range lines clamp to the end of the document.
pub fn line_col_to_byte(doc: &Document, lc: LineCol) -> ByteOffset {
    if lc.line >= doc.line_count() {
        return doc.len();
    }
    let start = doc.line_start(lc.line);
    ByteOffset((start.0 + lc.col.0).min(doc.line_end(lc.line).0))
}

/// The next grapheme boundary after `pos` (or the end of the document).
pub fn next_grapheme_boundary(doc: &Document, pos: ByteOffset) -> ByteOffset {
    let len = doc.len();
    if pos >= len {
        return len;
    }
    let (mut chunk, mut chunk_start) = doc.chunk_at(pos);
    let mut gc = GraphemeCursor::new(pos.0, len.0, true);
    loop {
        match gc.next_boundary(chunk, chunk_start.0) {
            Ok(Some(n)) => return ByteOffset(n),
            Ok(None) => return len,
            Err(GraphemeIncomplete::NextChunk) => {
                chunk_start = ByteOffset(chunk_start.0 + chunk.len());
                chunk = doc.chunk_at(chunk_start).0;
            }
            Err(GraphemeIncomplete::PreContext(n)) => provide_context(doc, &mut gc, n),
            Err(_) => return step_char_forward(doc, pos),
        }
    }
}

/// The previous grapheme boundary before `pos` (or 0).
pub fn prev_grapheme_boundary(doc: &Document, pos: ByteOffset) -> ByteOffset {
    if pos.0 == 0 {
        return pos;
    }
    let (mut chunk, mut chunk_start) = doc.chunk_at(pos);
    let mut gc = GraphemeCursor::new(pos.0, doc.len().0, true);
    loop {
        match gc.prev_boundary(chunk, chunk_start.0) {
            Ok(Some(n)) => return ByteOffset(n),
            Ok(None) => return ByteOffset(0),
            Err(GraphemeIncomplete::PrevChunk) => {
                (chunk, chunk_start) = doc.chunk_at(ByteOffset(chunk_start.0 - 1));
            }
            Err(GraphemeIncomplete::PreContext(n)) => provide_context(doc, &mut gc, n),
            Err(_) => return step_char_backward(doc, pos),
        }
    }
}

fn provide_context(doc: &Document, gc: &mut GraphemeCursor, n: usize) {
    let (ctx, ctx_start) = doc.chunk_at(ByteOffset(n - 1));
    gc.provide_context(&ctx[..n - ctx_start.0], ctx_start.0);
}

fn step_char_forward(doc: &Document, pos: ByteOffset) -> ByteOffset {
    ByteOffset(pos.0 + doc.char_at(pos).map_or(0, char::len_utf8))
}

fn step_char_backward(doc: &Document, pos: ByteOffset) -> ByteOffset {
    let mut p = pos.0.saturating_sub(1);
    while p > 0 && !doc.is_char_boundary(ByteOffset(p)) {
        p -= 1;
    }
    ByteOffset(p)
}

/// Display width of one grapheme starting at visual column `col`. Tabs expand
/// to the next tab stop; every other grapheme takes at least one cell so that
/// distinct boundaries always have distinct visual columns.
pub fn grapheme_width(g: &str, col: usize, tab_width: usize) -> usize {
    if g == "\t" {
        let tw = tab_width.max(1);
        tw - col % tw
    } else {
        g.width().max(1)
    }
}

/// Width of a string of graphemes that starts at visual column `start`.
fn advance(s: &str, start: usize, tab_width: usize) -> usize {
    s.graphemes(true)
        .fold(start, |col, g| col + grapheme_width(g, col, tab_width))
}

pub fn byte_to_visual_col(doc: &Document, pos: ByteOffset, tab_width: usize) -> VisualCol {
    let line_start = doc.line_start(doc.line_of(pos));
    VisualCol(advance(&doc.slice(line_start..pos), 0, tab_width))
}

/// The last grapheme boundary on `line` whose visual column is `<= col`, or the
/// line end if the line is shorter (no virtual space).
pub fn visual_col_to_byte(
    doc: &Document,
    line: usize,
    col: VisualCol,
    tab_width: usize,
) -> ByteOffset {
    let start = doc.line_start(line);
    let text = doc.line_text(line);
    let mut c = 0;
    for (i, g) in text.grapheme_indices(true) {
        let w = grapheme_width(g, c, tab_width);
        if c + w > col.0 {
            return ByteOffset(start.0 + i);
        }
        c += w;
    }
    ByteOffset(start.0 + text.len())
}

/// The first grapheme boundary on `line` whose visual column is `>= col`, or
/// the line end. Used for the left edge of a rectangle so that a tab or wide
/// glyph straddling the edge stays outside it.
pub fn visual_col_to_byte_ceil(
    doc: &Document,
    line: usize,
    col: VisualCol,
    tab_width: usize,
) -> ByteOffset {
    let start = doc.line_start(line);
    let text = doc.line_text(line);
    let mut c = 0;
    for (i, g) in text.grapheme_indices(true) {
        if c >= col.0 {
            return ByteOffset(start.0 + i);
        }
        c += grapheme_width(g, c, tab_width);
    }
    ByteOffset(start.0 + text.len())
}

/// Width of the whole line in visual columns.
pub fn line_visual_width(doc: &Document, line: usize, tab_width: usize) -> VisualCol {
    VisualCol(advance(&doc.line_text(line), 0, tab_width))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boundaries(text: &str) -> Vec<usize> {
        let doc = Document::new(text);
        let mut out = vec![0];
        let mut p = ByteOffset(0);
        while p < doc.len() {
            p = next_grapheme_boundary(&doc, p);
            out.push(p.0);
        }
        let mut back = vec![doc.len().0];
        let mut p = doc.len();
        while p.0 > 0 {
            p = prev_grapheme_boundary(&doc, p);
            back.push(p.0);
        }
        back.reverse();
        assert_eq!(out, back, "forward and backward stepping disagree");
        out
    }

    #[test]
    fn grapheme_stepping() {
        // e + combining acute is one grapheme.
        assert_eq!(boundaries("e\u{301}x"), vec![0, 3, 4]);
        // Family emoji (ZWJ sequence) is one grapheme of 18 bytes.
        assert_eq!(boundaries("👨‍👩‍👧!"), vec![0, 18, 19]);
        // Skin-tone modifier joins the base emoji.
        assert_eq!(boundaries("👍🏽"), vec![0, 8]);
        assert_eq!(boundaries("漢字"), vec![0, 3, 6]);
        assert_eq!(boundaries("a\nb"), vec![0, 1, 2, 3]);
        assert_eq!(boundaries(""), vec![0]);
    }

    #[test]
    fn grapheme_stepping_across_rope_chunks() {
        // Long enough for Ropey to split into many chunks; combining marks and
        // ZWJ sequences then straddle chunk boundaries somewhere.
        let unit = "ab e\u{301}👨‍👩‍👧漢\t";
        let text = unit.repeat(500);
        let b = boundaries(&text);
        let expected: Vec<usize> = std::iter::once(0)
            .chain(text.grapheme_indices(true).map(|(i, g)| i + g.len()))
            .collect();
        assert_eq!(b, expected);
    }

    #[test]
    fn visual_columns() {
        let doc = Document::new("\ta\t漢e\u{301}👨‍👩‍👧x\nnext");
        let cols: Vec<usize> = [0, 1, 2, 3, 6, 9, 27, 28]
            .iter()
            .map(|&b| byte_to_visual_col(&doc, ByteOffset(b), 4).0)
            .collect();
        // tab->4, a->5, tab->8, 漢 (2 cells)->10, é->11, family (2 cells)->13, x->14
        assert_eq!(cols, vec![0, 4, 5, 8, 10, 11, 13, 14]);
        assert_eq!(line_visual_width(&doc, 0, 4), VisualCol(14));
        assert_eq!(byte_to_visual_col(&doc, ByteOffset(29), 4), VisualCol(0));
    }

    #[test]
    fn visual_col_to_byte_snaps_left_inside_wide_glyphs_and_tabs() {
        let doc = Document::new("\ta漢b");
        let at = |c| visual_col_to_byte(&doc, 0, VisualCol(c), 4).0;
        assert_eq!(at(0), 0);
        assert_eq!(at(2), 0); // inside the tab
        assert_eq!(at(4), 1);
        assert_eq!(at(5), 2);
        assert_eq!(at(6), 2); // inside 漢
        assert_eq!(at(7), 5);
        assert_eq!(at(99), 6); // clamps to line end
    }

    #[test]
    fn line_col_round_trip() {
        let doc = Document::new("ab\ncdé\n");
        for b in [0, 1, 2, 3, 4, 5, 7, 8] {
            let lc = byte_to_line_col(&doc, ByteOffset(b));
            assert_eq!(line_col_to_byte(&doc, lc), ByteOffset(b));
        }
        assert_eq!(
            byte_to_line_col(&doc, ByteOffset(5)),
            LineCol {
                line: 1,
                col: ByteOffset(2)
            }
        );
    }
}
