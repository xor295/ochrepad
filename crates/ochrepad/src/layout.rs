//! Per-line layout shared by painting and hit testing (PLAN 3.7). The x of
//! every grapheme boundary is computed here, once, from `core::position`'s
//! widths; nothing else computes x positions. Runs of printable ASCII are
//! stored as one entry each, so a line of megabytes costs little more than
//! its text.

use std::collections::HashMap;
use std::sync::Arc;

use egui::Color32;
use ochrepad_core::position::grapheme_width;
use ochrepad_core::{BufferId, Document};
use ochrepad_syntax::{LineSpans, Span};

/// A run drawn as one galley at a fixed grid x. ASCII runs of one color are
/// grouped; any other grapheme is its own piece, clipped to its cells so
/// fallback-font and emoji glyphs stay on the grid.
#[derive(Debug, Clone)]
pub struct Piece {
    /// Byte range within the line.
    pub range: std::ops::Range<usize>,
    pub x: f32,
    /// Width of the cells the piece occupies.
    pub width: f32,
    pub color: Color32,
    pub clip: bool,
}

/// The cells of a control, format or unusual space character
/// (`crate::invisible`): drawn as a labelled box, never as a glyph.
#[derive(Debug, Clone)]
pub struct SpecialCell {
    /// Byte range of the grapheme within the line.
    pub range: std::ops::Range<usize>,
    pub x: f32,
    pub width: f32,
}

/// A stretch of a line's graphemes and where they are: either `uniform`, a
/// run of printable ASCII where every byte is a one-cell grapheme (most of
/// most lines, stored in one entry however long), or a single grapheme of
/// any size. Offsets fit in `u32`: lines that long are not editable anyway.
#[derive(Debug, Clone, Copy)]
struct Run {
    byte: u32,
    len: u32,
    x: f32,
    width: f32,
    uniform: bool,
}

#[derive(Debug, Clone)]
pub struct LineLayout {
    pub text: String,
    /// The grapheme runs, in order, covering the whole line.
    runs: Vec<Run>,
    pub pieces: Vec<Piece>,
    /// Special characters, which are not in `pieces`.
    pub specials: Vec<SpecialCell>,
    pub width: f32,
    cell_w: f32,
}

/// The end of the grapheme starting at `at`, a grapheme boundary. From a
/// boundary the text after it segments on its own; a cursor placed there
/// would look back over everything before it (a long run of flag letters
/// made laying a line out quadratic).
fn next_grapheme(text: &str, at: usize) -> usize {
    use unicode_segmentation::UnicodeSegmentation;
    at + text[at..].graphemes(true).next().map_or(1, str::len)
}

/// Adds ASCII text drawn as one run, joining the previous piece when it is
/// ASCII of the same colour and ends where this starts.
fn push_ascii(
    pieces: &mut Vec<Piece>,
    range: std::ops::Range<usize>,
    x: f32,
    color: Color32,
    cell_w: f32,
) {
    match pieces.last_mut() {
        Some(p) if !p.clip && p.range.end == range.start && p.color == color => {
            p.width += range.len() as f32 * cell_w;
            p.range.end = range.end;
        }
        _ => pieces.push(Piece {
            width: range.len() as f32 * cell_w,
            range,
            x,
            color,
            clip: false,
        }),
    }
}

impl LineLayout {
    /// `spans` (sorted, from the highlighter) color the text; gaps and
    /// unhighlighted lines use `color`.
    pub fn build(
        text: String,
        cell_w: f32,
        tab_width: usize,
        color: Color32,
        spans: Option<&[Span]>,
    ) -> Self {
        let spans = spans.unwrap_or(&[]);
        let bytes = text.as_bytes();
        let len = bytes.len();
        let mut span_i = 0;
        // The colour at `byte` and the byte where it next may change.
        let mut color_at = |byte: usize| -> (Color32, usize) {
            while span_i < spans.len() && spans[span_i].end <= byte {
                span_i += 1;
            }
            match spans.get(span_i) {
                Some(s) if s.start <= byte => {
                    let [r, g, b, a] = s.color;
                    (Color32::from_rgba_unmultiplied(r, g, b, a), s.end)
                }
                Some(s) => (color, s.start),
                None => (color, usize::MAX),
            }
        };
        let mut runs: Vec<Run> = Vec::new();
        let mut pieces: Vec<Piece> = Vec::new();
        let mut specials = Vec::new();
        let mut col = 0usize;
        let mut i = 0;
        while i < len {
            // Printable ASCII is one byte and one cell per grapheme, except
            // a last character followed by non-ASCII, which may combine
            // with it (e + U+0301).
            let mut j = i;
            while j < len && (0x20..0x7F).contains(&bytes[j]) {
                j += 1;
            }
            if j < len && bytes[j] >= 0x80 && j > i {
                j -= 1;
            }
            if j > i {
                let x = col as f32 * cell_w;
                runs.push(Run {
                    byte: i as u32,
                    len: (j - i) as u32,
                    x,
                    width: (j - i) as f32 * cell_w,
                    uniform: true,
                });
                let mut a = i;
                while a < j {
                    let (c, change) = color_at(a);
                    let b = change.clamp(a + 1, j);
                    push_ascii(&mut pieces, a..b, (col + a - i) as f32 * cell_w, c, cell_w);
                    a = b;
                }
                col += j - i;
                i = j;
                continue;
            }
            let end = next_grapheme(&text, i);
            let g = &text[i..end];
            let x = col as f32 * cell_w;
            let w = grapheme_width(g, col, tab_width);
            runs.push(Run {
                byte: i as u32,
                len: (end - i) as u32,
                x,
                width: w as f32 * cell_w,
                uniform: false,
            });
            col += w;
            if g == "\t" {
                // Drawn as nothing (or Show Symbol's arrow).
            } else if crate::invisible::maybe_special(bytes[i])
                && crate::invisible::special(g).is_some()
            {
                // A control character inside an ASCII run would be laid out
                // by the font (as a replacement glyph of its own width) and
                // push the rest of the run off the grid.
                specials.push(SpecialCell {
                    range: i..end,
                    x,
                    width: w as f32 * cell_w,
                });
            } else if g.len() == 1 {
                let (c, _) = color_at(i);
                push_ascii(&mut pieces, i..end, x, c, cell_w);
            } else {
                let (c, _) = color_at(i);
                pieces.push(Piece {
                    range: i..end,
                    x,
                    width: w as f32 * cell_w,
                    color: c,
                    clip: true,
                });
            }
            i = end;
        }
        Self {
            text,
            runs,
            pieces,
            specials,
            width: col as f32 * cell_w,
            cell_w,
        }
    }

    /// x of a line-relative byte offset (snapped to the grapheme boundary at
    /// or before it).
    pub fn x_of(&self, byte: usize) -> f32 {
        let i = self.runs.partition_point(|r| r.byte as usize <= byte);
        let Some(r) = i.checked_sub(1).map(|i| self.runs[i]) else {
            return 0.0;
        };
        let off = byte - r.byte as usize;
        if off >= r.len as usize {
            r.x + r.width
        } else if r.uniform {
            r.x + off as f32 * self.cell_w
        } else {
            r.x
        }
    }

    /// Inverse of `x_of`: the nearest grapheme boundary to x (the left one
    /// on a tie).
    pub fn byte_at_x(&self, x: f32) -> usize {
        let i = self.runs.partition_point(|r| r.x < x);
        let Some(r) = i.checked_sub(1).map(|i| self.runs[i]) else {
            return 0;
        };
        let (start, len) = (r.byte as usize, r.len as usize);
        if r.uniform {
            let t = (x - r.x) / self.cell_w;
            let k = if t - t.floor() <= 0.5 {
                t.floor()
            } else {
                t.ceil()
            };
            start + (k.max(0.0) as usize).min(len)
        } else if x - r.x <= r.x + r.width - x {
            start
        } else {
            start + len
        }
    }

    /// Every grapheme that overlaps `lo..hi` (x within the line), as
    /// (byte offset, x, x of its end).
    pub fn graphemes_between(
        &self,
        lo: f32,
        hi: f32,
    ) -> impl Iterator<Item = (usize, f32, f32)> + '_ {
        let first = self.runs.partition_point(|r| r.x + r.width <= lo);
        let cell_w = self.cell_w;
        self.runs[first..]
            .iter()
            .take_while(move |r| r.x < hi)
            .flat_map(move |r| {
                let (start, len) = (r.byte as usize, r.len as usize);
                let (from, to) = if r.uniform {
                    let from = (((lo - r.x) / cell_w).floor().max(0.0) as usize).min(len);
                    let to = (((hi - r.x) / cell_w).ceil().max(0.0) as usize).min(len);
                    (from, to.max(from))
                } else {
                    (0, 1)
                };
                (from..to).map(move |k| {
                    if r.uniform {
                        let x = r.x + k as f32 * cell_w;
                        (start + k, x, x + cell_w)
                    } else {
                        (start, r.x, r.x + r.width)
                    }
                })
            })
    }

    /// Every grapheme boundary as (byte offset, x), from 0 to the line end.
    #[cfg(test)]
    fn boundaries(&self) -> Vec<(usize, f32)> {
        let mut out: Vec<(usize, f32)> = self
            .graphemes_between(f32::MIN, f32::MAX)
            .map(|(b, x, _)| (b, x))
            .collect();
        out.push((self.text.len(), self.width));
        out
    }
}

/// The visual column nearest to `x` on the cell grid, including columns past
/// the end of the line (rectangular selection works in visual columns).
/// This is the inverse of the `col * cell_w` placement in `LineLayout::build`.
pub fn visual_col_at_x(x: f32, cell_w: f32) -> ochrepad_core::VisualCol {
    ochrepad_core::VisualCol((x / cell_w).round().max(0.0) as usize)
}

/// Which styling a cached layout must have.
#[derive(Clone, Copy)]
pub enum Styling<'a> {
    /// Any: positions do not depend on styling (hit testing).
    Any,
    /// These spans (compared by pointer), or none.
    Exact(Option<&'a LineSpans>),
}

/// Layouts of recently shown lines, valid for one buffer revision and one set
/// of metrics. Any edit invalidates all of them; rebuilding the visible lines
/// takes well under a millisecond (M0). A line is also rebuilt when the
/// highlighter hands it different spans.
#[derive(Default)]
pub struct LayoutCache {
    key: Option<(BufferId, u64, u32, usize, Color32)>,
    lines: HashMap<usize, (Option<LineSpans>, Arc<LineLayout>)>,
}

impl LayoutCache {
    #[allow(clippy::too_many_arguments)]
    pub fn get(
        &mut self,
        doc: &Document,
        key: (BufferId, u64),
        line: usize,
        cell_w: f32,
        tab_width: usize,
        color: Color32,
        styling: Styling<'_>,
    ) -> Arc<LineLayout> {
        let full_key = (key.0, key.1, cell_w.to_bits(), tab_width, color);
        if self.key != Some(full_key) {
            self.lines.clear();
            self.key = Some(full_key);
        }
        let reusable = match (self.lines.get(&line), styling) {
            (Some(_), Styling::Any) => true,
            (Some((have, _)), Styling::Exact(want)) => match (have, want) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            },
            (None, _) => false,
        };
        if let (true, Some((_, layout))) = (reusable, self.lines.get(&line)) {
            return layout.clone();
        }
        let spans = match styling {
            Styling::Exact(s) => s.cloned(),
            Styling::Any => None,
        };
        let layout = Arc::new(LineLayout::build(
            doc.line_text(line),
            cell_w,
            tab_width,
            color,
            spans.as_ref().map(|s| &s[..]),
        ));
        self.lines.insert(line, (spans, layout.clone()));
        layout
    }

    /// Drops layouts far from the visible range.
    pub fn retain_near(&mut self, first: usize, last: usize) {
        if self.lines.len() > 4 * (last - first + 1) + 256 {
            let span = last - first + 1;
            self.lines
                .retain(|&l, _| l + span >= first && l <= last + span);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CELL: f32 = 8.0;

    #[test]
    fn hit_test_inverts_layout() {
        for line in ["\ta\t漢e\u{301}👨‍👩‍👧x", "plain ascii", "", "\t\t"] {
            let l = LineLayout::build(line.to_owned(), CELL, 4, Color32::WHITE, None);
            for w in l.boundaries().windows(2) {
                let ((lb, lx), (rb, rx)) = (w[0], w[1]);
                assert_eq!(l.byte_at_x(lx), lb);
                assert_eq!(l.x_of(lb), lx);
                assert_eq!(l.byte_at_x(lx + (rx - lx) * 0.4), lb);
                assert_eq!(l.byte_at_x(lx + (rx - lx) * 0.6), rb);
            }
            assert_eq!(l.byte_at_x(-3.0), 0);
            assert_eq!(l.byte_at_x(1e9), line.len());
        }
    }

    #[test]
    fn spans_split_pieces_by_color() {
        let span = |start, end| Span {
            start,
            end,
            color: [255, 0, 0, 255],
            bold: false,
            italic: false,
            underline: false,
        };
        let l = LineLayout::build(
            "let x = 1;".to_owned(),
            CELL,
            4,
            Color32::WHITE,
            Some(&[span(0, 3), span(8, 9)]),
        );
        let runs: Vec<(&str, bool)> = l
            .pieces
            .iter()
            .map(|p| (&l.text[p.range.clone()], p.color == Color32::RED))
            .collect();
        assert_eq!(
            runs,
            vec![("let", true), (" x = ", false), ("1", true), (";", false)]
        );
    }

    /// The positions the layout must give: every grapheme boundary at its
    /// visual column (the straightforward per-grapheme computation).
    fn reference_boundaries(text: &str) -> Vec<(usize, f32)> {
        use unicode_segmentation::UnicodeSegmentation;
        let mut col = 0;
        let mut out = Vec::new();
        for (byte, g) in text.grapheme_indices(true) {
            out.push((byte, col as f32 * CELL));
            col += grapheme_width(g, col, 4);
        }
        out.push((text.len(), col as f32 * CELL));
        out
    }

    #[test]
    fn runs_give_the_same_positions_as_each_grapheme_would() {
        for text in [
            "",
            "plain",
            "e\u{301}x",
            "ab\u{301}cd",
            "a\u{200D}b",
            "x🇺🇸🇬🇧y",
            "\u{0600}a b",
            "\u{1100}\u{1161}\u{11A8}z",
            "tab\there\t\t!",
            "mixed 漢字 and 👨‍👩‍👧 and é",
            "ctrl\u{1b}[0m\0end",
            "a漢",
        ] {
            let l = LineLayout::build(text.to_owned(), CELL, 4, Color32::WHITE, None);
            assert_eq!(l.boundaries(), reference_boundaries(text), "{text:?}");
            for (b, x) in reference_boundaries(text) {
                assert_eq!(l.x_of(b), x, "{text:?} at {b}");
            }
        }
    }

    #[test]
    fn a_long_ascii_line_is_one_run() {
        let text = "x".repeat(4_000_000);
        let l = LineLayout::build(text, CELL, 4, Color32::WHITE, None);
        assert_eq!(l.runs.len(), 1);
        assert_eq!(l.pieces.len(), 1);
        assert_eq!(l.x_of(3_000_000), 3_000_000.0 * CELL);
        assert_eq!(l.byte_at_x(1234.4 * CELL), 1234);
        assert_eq!(l.byte_at_x(1234.6 * CELL), 1235);
        let visible: Vec<usize> = l
            .graphemes_between(10.0 * CELL, 13.0 * CELL)
            .map(|g| g.0)
            .collect();
        assert_eq!(visible, vec![10, 11, 12]);
    }

    #[test]
    fn special_characters_get_their_own_cells_and_no_glyphs() {
        let l = LineLayout::build(
            "a\u{1b}b\u{200B}c".to_owned(),
            CELL,
            4,
            Color32::WHITE,
            None,
        );
        let drawn: Vec<&str> = l.pieces.iter().map(|p| &l.text[p.range.clone()]).collect();
        assert_eq!(drawn, vec!["a", "b", "c"]);
        let specials: Vec<(&str, f32)> = l
            .specials
            .iter()
            .map(|s| (&l.text[s.range.clone()], s.x / CELL))
            .collect();
        assert_eq!(specials, vec![("\u{1b}", 1.0), ("\u{200B}", 3.0)]);
        // The text after each one stays on the grid.
        assert_eq!(l.pieces[1].x, 2.0 * CELL);
        assert_eq!(l.pieces[2].x, 4.0 * CELL);
    }

    #[test]
    fn columns_match_core_positions() {
        let l = LineLayout::build(
            "\ta\t漢e\u{301}👨‍👩‍👧x".to_owned(),
            CELL,
            4,
            Color32::WHITE,
            None,
        );
        let cols: Vec<f32> = l.boundaries().iter().map(|b| b.1 / CELL).collect();
        assert_eq!(cols, vec![0.0, 4.0, 5.0, 8.0, 10.0, 11.0, 13.0, 14.0]);
        // Wide and combining graphemes are clipped pieces; 'a' and 'x' are not.
        let clipped: Vec<bool> = l.pieces.iter().map(|p| p.clip).collect();
        assert_eq!(clipped, vec![false, true, true, true, false]);
    }
}
