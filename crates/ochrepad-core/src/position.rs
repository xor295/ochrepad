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

/// What a language server's columns count (LSP's position encodings).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnUnits {
    Utf8,
    /// The protocol's default.
    Utf16,
    Utf32,
}

impl ColumnUnits {
    fn of(self, c: char) -> usize {
        match self {
            ColumnUnits::Utf8 => c.len_utf8(),
            ColumnUnits::Utf16 => c.len_utf16(),
            ColumnUnits::Utf32 => 1,
        }
    }
}

/// `pos` as a language server's (line, column), the column in `units`
/// from the line's start. Numbers past `u32` saturate.
pub fn to_lsp_position(doc: &Document, pos: ByteOffset, units: ColumnUnits) -> (u32, u32) {
    let line = doc.line_of(pos);
    let start = doc.line_start(line);
    let column = match units {
        ColumnUnits::Utf8 => pos.0 - start.0,
        _ => doc.slice(start..pos).chars().map(|c| units.of(c)).sum(),
    };
    (sat(line), sat(column))
}

/// The offset a language server's (line, column) names. Clamped, since a
/// server's numbers are not to be trusted: a line past the end is the end
/// of the document, a column past the line's end is the line's end, and a
/// column inside a character is that character's start.
pub fn from_lsp_position(doc: &Document, line: u32, column: u32, units: ColumnUnits) -> ByteOffset {
    let line = line as usize;
    if line >= doc.line_count() {
        return doc.len();
    }
    let (start, end) = (doc.line_start(line), doc.line_end(line));
    let mut left = column as usize;
    let mut at = start.0;
    for c in doc.slice(start..end).chars() {
        let n = units.of(c);
        if left < n {
            break;
        }
        left -= n;
        at += c.len_utf8();
    }
    ByteOffset(at)
}

fn sat(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
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

/// (byte, visual column, width in cells) of each grapheme of one line's
/// `text`.
pub fn cells(text: &str, tab_width: usize) -> Vec<(usize, usize, usize)> {
    let mut col = 0;
    let mut out = Vec::with_capacity(text.len());
    if text.is_ascii() {
        // Every byte is a grapheme (a line holds no CR LF pair).
        let tw = tab_width.max(1);
        for (i, b) in text.bytes().enumerate() {
            let w = if b == b'\t' { tw - col % tw } else { 1 };
            out.push((i, col, w));
            col += w;
        }
    } else {
        for (i, g) in text.grapheme_indices(true) {
            let w = grapheme_width(g, col, tab_width);
            out.push((i, col, w));
            col += w;
        }
    }
    out
}

/// The narrowest wrap width, in cells.
pub const MIN_WRAP_WIDTH: usize = 10;

/// How one line wraps into display rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wrapped {
    /// Byte offset (within the line) where each row starts; the first is 0.
    pub starts: Vec<usize>,
    /// The visual column (of the line) where each row starts.
    pub start_cols: Vec<usize>,
    /// Cells that rows after the first are indented by.
    pub indent: usize,
}

/// Wraps `text` (one line, without its line break) to rows of `width`
/// cells. Rows break after a run of whitespace when there is one in the
/// row, otherwise before the grapheme that does not fit; a grapheme is never
/// split, and one wider than a row gets a row of its own. Tab stops count
/// from the start of the line, so wrapping never changes a grapheme's
/// visual column. Rows after the first are indented like the line, by at
/// most half the width.
pub fn wrap_line(text: &str, width: usize, tab_width: usize) -> Wrapped {
    let width = width.max(MIN_WRAP_WIDTH);
    // Most lines fit; for plain ASCII that is a length check.
    if text.len() <= width && text.bytes().all(|b| b.is_ascii() && b != b'\t') {
        let lead = text.len() - text.trim_start_matches(' ').len();
        return Wrapped {
            starts: vec![0],
            start_cols: vec![0],
            indent: lead.min(width / 2),
        };
    }
    let lead = text.len() - text.trim_start_matches([' ', '\t']).len();
    let indent = advance(&text[..lead], 0, tab_width).min(width / 2);
    let mut out = Wrapped {
        starts: vec![0],
        start_cols: vec![0],
        indent,
    };
    // (byte, column, width, is whitespace) of each grapheme.
    let mut col = 0;
    let gs: Vec<(usize, usize, usize, bool)> = if text.is_ascii() {
        // Most lines: every byte is a grapheme one cell wide (a line holds
        // no `\n`, so there is no CR LF pair), except tabs.
        text.bytes()
            .enumerate()
            .map(|(i, b)| {
                let w = if b == b'\t' {
                    let tw = tab_width.max(1);
                    tw - col % tw
                } else {
                    1
                };
                col += w;
                (i, col - w, w, b == b' ' || b == b'\t')
            })
            .collect()
    } else {
        text.grapheme_indices(true)
            .map(|(i, g)| {
                let w = grapheme_width(g, col, tab_width);
                col += w;
                (i, col - w, w, g == " " || g == "\t")
            })
            .collect()
    };
    // Never break right after the leading indentation.
    let lead_graphemes = gs.iter().take_while(|g| g.3).count();
    let (mut row_start, mut cap) = (0, width);
    // The last place this row could break: just after whitespace.
    let mut candidate = None;
    let mut j = 0;
    while j < gs.len() {
        let (_, col, w, space) = gs[j];
        if j > row_start.max(lead_graphemes) && gs[j - 1].3 && !space {
            candidate = Some(j);
        }
        let row_col = gs[row_start].1;
        if col + w - row_col > cap && j > row_start {
            let at = candidate.unwrap_or(j);
            out.starts.push(gs[at].0);
            out.start_cols.push(gs[at].1);
            row_start = at;
            cap = width - indent;
            candidate = None;
            // Look at the new row from its start.
            j = at;
            continue;
        }
        j += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsp_positions_count_the_servers_units_and_clamp() {
        let doc = Document::new(
            "a😀b
xy",
        );
        let b = ByteOffset(5);
        assert_eq!(to_lsp_position(&doc, b, ColumnUnits::Utf8), (0, 5));
        assert_eq!(to_lsp_position(&doc, b, ColumnUnits::Utf16), (0, 3));
        assert_eq!(to_lsp_position(&doc, b, ColumnUnits::Utf32), (0, 2));
        // Inside the emoji's surrogate pair: its start.
        assert_eq!(
            from_lsp_position(&doc, 0, 2, ColumnUnits::Utf16),
            ByteOffset(1)
        );
        assert_eq!(
            from_lsp_position(&doc, 0, 3, ColumnUnits::Utf8),
            ByteOffset(1)
        );
        // Past the line's end, past the last line.
        assert_eq!(
            from_lsp_position(&doc, 0, 99, ColumnUnits::Utf16),
            ByteOffset(6)
        );
        assert_eq!(from_lsp_position(&doc, 9, 0, ColumnUnits::Utf16), doc.len());
        assert_eq!(
            from_lsp_position(&doc, 1, u32::MAX, ColumnUnits::Utf8),
            doc.len()
        );
    }

    /// The rows of `text` wrapped at `width` (tab width 4).
    fn rows(text: &str, width: usize) -> Vec<&str> {
        let w = wrap_line(text, width, 4);
        let mut ends = w.starts[1..].to_vec();
        ends.push(text.len());
        w.starts
            .iter()
            .zip(ends)
            .map(|(&a, b)| &text[a..b])
            .collect()
    }

    #[test]
    fn wrapping_breaks_after_whitespace_or_inside_long_words() {
        assert_eq!(rows("short", 10), vec!["short"]);
        assert_eq!(rows("aaaa bbbb cccc", 10), vec!["aaaa bbbb ", "cccc"]);
        assert_eq!(
            rows("abcdefghijklmnopqrstuvwxy", 10),
            vec!["abcdefghij", "klmnopqrst", "uvwxy"]
        );
        // Indented by 4: continuation rows hold 6 cells; the indentation
        // itself is never a row on its own.
        assert_eq!(
            rows("    xxxxxxxxxxxxxxxx", 10),
            vec!["    xxxxxx", "xxxxxx", "xxxx"]
        );
        assert_eq!(wrap_line("    x x", 10, 4).indent, 4);
        // Wide glyphs are not split; a tab keeps its line column.
        assert_eq!(rows("漢字漢字漢字", 10), vec!["漢字漢字漢", "字"]);
        assert_eq!(rows("ab\tcdefghijk", 10), vec!["ab\t", "cdefghijk"]);
        assert_eq!(wrap_line("ab\tcdefghijk", 10, 4).start_cols, vec![0, 4]);
        // Combining marks and emoji stay whole.
        let marks = "e\u{301}e\u{301}e\u{301}👨\u{200d}👩\u{200d}👧xxxxxxx";
        assert_eq!(
            rows(marks, 10),
            vec!["e\u{301}e\u{301}e\u{301}👨\u{200d}👩\u{200d}👧xxxxx", "xx"]
        );
        // Very narrow widths are raised to the minimum.
        assert_eq!(rows("aaaaaaaaaaaa", 3), vec!["aaaaaaaaaa", "aa"]);
    }

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
