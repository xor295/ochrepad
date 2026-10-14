//! Per-line layout shared by painting and hit testing (PLAN 3.7). The x of
//! every grapheme boundary is computed here, once, from `core::position`'s
//! widths; nothing else computes x positions.

use std::collections::HashMap;
use std::sync::Arc;

use egui::Color32;
use ochrepad_core::position::grapheme_width;
use ochrepad_core::{BufferId, Document};
use ochrepad_syntax::{LineSpans, Span};
use unicode_segmentation::UnicodeSegmentation;

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

#[derive(Debug, Clone)]
pub struct LineLayout {
    pub text: String,
    /// (byte offset within the line, x) for every grapheme boundary,
    /// including 0 and the line end.
    pub boundaries: Vec<(usize, f32)>,
    pub pieces: Vec<Piece>,
    /// Special characters, which are not in `pieces`.
    pub specials: Vec<SpecialCell>,
    pub width: f32,
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
        let mut span_i = 0;
        let mut boundaries = Vec::with_capacity(text.len() + 1);
        let mut pieces: Vec<Piece> = Vec::new();
        let mut specials = Vec::new();
        let mut col = 0usize;
        for (byte, g) in text.grapheme_indices(true) {
            while span_i < spans.len() && spans[span_i].end <= byte {
                span_i += 1;
            }
            let color = match spans.get(span_i) {
                Some(s) if s.start <= byte => {
                    let [r, g, b, a] = s.color;
                    Color32::from_rgba_unmultiplied(r, g, b, a)
                }
                _ => color,
            };
            let x = col as f32 * cell_w;
            boundaries.push((byte, x));
            let w = grapheme_width(g, col, tab_width);
            col += w;
            if g == "\t" {
                continue;
            }
            // A control character inside an ASCII run would be laid out by
            // the font (as a replacement glyph of its own width) and push the
            // rest of the run off the grid.
            if crate::invisible::maybe_special(g.as_bytes()[0])
                && crate::invisible::special(g).is_some()
            {
                specials.push(SpecialCell {
                    range: byte..byte + g.len(),
                    x,
                    width: w as f32 * cell_w,
                });
                continue;
            }
            let ascii = g.len() == 1;
            match pieces.last_mut() {
                Some(p) if ascii && !p.clip && p.range.end == byte && p.color == color => {
                    p.range.end = byte + 1;
                    p.width += cell_w;
                }
                _ => pieces.push(Piece {
                    range: byte..byte + g.len(),
                    x,
                    width: w as f32 * cell_w,
                    color,
                    clip: !ascii,
                }),
            }
        }
        let width = col as f32 * cell_w;
        boundaries.push((text.len(), width));
        Self {
            text,
            boundaries,
            pieces,
            specials,
            width,
        }
    }

    /// x of a line-relative byte offset (snapped to the grapheme boundary at
    /// or before it).
    pub fn x_of(&self, byte: usize) -> f32 {
        match self.boundaries.binary_search_by_key(&byte, |b| b.0) {
            Ok(i) => self.boundaries[i].1,
            Err(i) => self.boundaries[i.saturating_sub(1)].1,
        }
    }

    /// Inverse of `x_of`: the nearest grapheme boundary to x.
    pub fn byte_at_x(&self, x: f32) -> usize {
        let i = self.boundaries.partition_point(|b| b.1 < x);
        if i == 0 {
            return self.boundaries[0].0;
        }
        let Some(&(rb, rx)) = self.boundaries.get(i) else {
            return self.text.len();
        };
        let (lb, lx) = self.boundaries[i - 1];
        if x - lx <= rx - x { lb } else { rb }
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
            for w in l.boundaries.windows(2) {
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
        let cols: Vec<f32> = l.boundaries.iter().map(|b| b.1 / CELL).collect();
        assert_eq!(cols, vec![0.0, 4.0, 5.0, 8.0, 10.0, 11.0, 13.0, 14.0]);
        // Wide and combining graphemes are clipped pieces; 'a' and 'x' are not.
        let clipped: Vec<bool> = l.pieces.iter().map(|p| p.clip).collect();
        assert_eq!(clipped, vec![false, true, true, true, false]);
    }
}
