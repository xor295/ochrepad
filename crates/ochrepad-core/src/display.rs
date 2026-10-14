//! Display rows: how a view's document lines map to rows on screen
//! (PLAN_v0.2.0 3.2). A row is addressed relative to its line, as a
//! [`RowPos`], so nothing needs a global row index: with word wrap that
//! would mean wrapping the whole file. Folded lines will have no rows and
//! wrapped lines several; for now every line is exactly one row.
//!
//! The editor widget scrolls, paints and hit-tests through a [`DisplayMap`],
//! and vertical motions resolve through one, so a macro's `Up` moves one
//! row in whatever view plays it.

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::position::{ByteOffset, VisualCol, byte_to_visual_col, visual_col_to_byte};
use crate::text::Document;
use crate::view::View;

/// One display row: the `sub`-th row of document line `line`. Rows order by
/// line, then by row within the line.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct RowPos {
    pub line: usize,
    pub sub: usize,
}

impl RowPos {
    pub fn new(line: usize, sub: usize) -> Self {
        Self { line, sub }
    }
}

/// A view's rows over its document.
#[derive(Clone, Copy)]
pub struct DisplayMap<'a> {
    doc: &'a Document,
    tab_width: usize,
}

impl<'a> DisplayMap<'a> {
    pub fn new(doc: &'a Document, _view: &View, tab_width: usize) -> Self {
        Self { doc, tab_width }
    }

    pub fn doc(&self) -> &'a Document {
        self.doc
    }

    /// Rows that `line` takes on screen: 0 for a hidden line.
    pub fn rows_in_line(&self, _line: usize) -> usize {
        1
    }

    /// Lines that are not hidden. Sizes the scrollbar.
    pub fn visible_line_count(&self) -> usize {
        self.doc.line_count()
    }

    pub fn first_row(&self) -> RowPos {
        RowPos::default()
    }

    pub fn last_row(&self) -> RowPos {
        let line = self.doc.line_count() - 1;
        RowPos::new(line, self.rows_in_line(line) - 1)
    }

    /// The row `n` rows after `row` (before it for negative `n`), stopping at
    /// the first and last rows, and how many rows it actually moved.
    pub fn step(&self, row: RowPos, n: isize) -> (RowPos, isize) {
        let last = self.doc.line_count() as isize - 1;
        let line = (row.line as isize + n).clamp(0, last);
        (RowPos::new(line as usize, 0), line - row.line as isize)
    }

    /// The row `n` rows after `row`, clamped to the document.
    pub fn advance(&self, row: RowPos, n: isize) -> RowPos {
        self.step(row, n).0
    }

    /// The text shown on `row`, without the line break.
    pub fn row_range(&self, row: RowPos) -> Range<ByteOffset> {
        self.doc.line_start(row.line)..self.doc.line_end(row.line)
    }

    /// The row showing `pos`.
    pub fn row_of(&self, pos: ByteOffset) -> RowPos {
        RowPos::new(self.doc.line_of(pos), 0)
    }

    /// The visual column of `pos` counted from the start of its row.
    pub fn col_in_row(&self, pos: ByteOffset) -> VisualCol {
        byte_to_visual_col(self.doc, pos, self.tab_width)
    }

    /// The position at visual column `col` of `row` (counted from the row's
    /// start), or the row's end if it is shorter.
    pub fn byte_at_col(&self, row: RowPos, col: VisualCol) -> ByteOffset {
        visual_col_to_byte(self.doc, row.line, col, self.tab_width)
    }

    /// A scroll position as a number of visible lines from the top, for the
    /// scrollbar: the visible lines above `row`'s line, plus the part of that
    /// line scrolled past.
    pub fn offset_of(&self, row: RowPos, frac: f32) -> f64 {
        let rows = self.rows_in_line(row.line).max(1) as f64;
        row.line as f64 + (row.sub as f64 + frac as f64) / rows
    }

    /// The inverse of [`offset_of`](Self::offset_of): the top row and fraction
    /// for a scrollbar offset.
    pub fn pos_at_offset(&self, offset: f64) -> (RowPos, f32) {
        let max = self.visible_line_count().saturating_sub(1) as f64;
        let offset = offset.clamp(0.0, max + 0.999_999);
        let line = (offset.floor() as usize).min(self.doc.line_count() - 1);
        let rows = self.rows_in_line(line).max(1) as f64;
        let within = (offset - line as f64) * rows;
        let sub = (within.floor() as usize).min(self.rows_in_line(line).max(1) - 1);
        (RowPos::new(line, sub), (within - sub as f64) as f32)
    }

    /// Scrolls a (top row, fraction) position by `rows`, which may be
    /// fractional. Hitting the first row leaves no fraction; the caller
    /// clamps the bottom with [`max_top`](Self::max_top).
    pub fn scroll_by(&self, row: RowPos, frac: f32, rows: f64) -> (RowPos, f32) {
        let total = frac as f64 + rows;
        let whole = total.floor();
        let (to, moved) = self.step(row, whole as isize);
        if moved as f64 != whole {
            // Stopped at the first or last row.
            (to, 0.0)
        } else {
            (to, (total - whole) as f32)
        }
    }

    /// The lowest top row that still fills a viewport of `rows_full` rows
    /// (the first row when the document is shorter).
    pub fn max_top(&self, rows_full: usize) -> RowPos {
        self.advance(self.last_row(), -(rows_full.saturating_sub(1) as isize))
    }

    /// Rows from `top` (inclusive) down, at most `limit`.
    pub fn rows_from(&self, top: RowPos, limit: usize) -> impl Iterator<Item = RowPos> + '_ {
        let last = self.last_row();
        std::iter::successors(Some(top), move |&r| (r < last).then(|| self.advance(r, 1)))
            .take(limit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Editor;

    fn with_map(text: &str, f: impl FnOnce(DisplayMap)) {
        let mut ed = Editor::new();
        let buffer = ed.add_buffer(text, Default::default());
        let view = ed.add_view(buffer).unwrap();
        let v = ed.view(view).unwrap();
        let doc = ed.buffer(v.buffer()).unwrap().doc();
        f(DisplayMap::new(doc, v, 4));
    }

    #[test]
    fn identity_rows_are_lines() {
        with_map("a\nbb\n\tc", |m| {
            assert_eq!(m.last_row(), RowPos::new(2, 0));
            assert_eq!(m.row_of(ByteOffset(3)), RowPos::new(1, 0));
            assert_eq!(m.row_range(RowPos::new(1, 0)), ByteOffset(2)..ByteOffset(4));
            assert_eq!(m.step(RowPos::new(1, 0), 5), (RowPos::new(2, 0), 1));
            assert_eq!(m.step(RowPos::new(1, 0), -5), (RowPos::new(0, 0), -1));
            assert_eq!(m.col_in_row(ByteOffset(6)), VisualCol(4));
            assert_eq!(
                m.byte_at_col(RowPos::new(2, 0), VisualCol(2)),
                ByteOffset(5)
            );
            let rows: Vec<_> = m.rows_from(RowPos::new(1, 0), 10).collect();
            assert_eq!(rows, vec![RowPos::new(1, 0), RowPos::new(2, 0)]);
        });
    }

    #[test]
    fn scrolling_clamps_and_round_trips_offsets() {
        with_map(&"x\n".repeat(99), |m| {
            assert_eq!(m.max_top(10), RowPos::new(90, 0));
            let (row, frac) = m.scroll_by(RowPos::new(5, 0), 0.25, 2.5);
            assert_eq!((row, frac), (RowPos::new(7, 0), 0.75));
            assert_eq!(
                m.scroll_by(RowPos::new(1, 0), 0.5, -3.0),
                (RowPos::new(0, 0), 0.0)
            );
            assert_eq!(m.offset_of(row, frac), 7.75);
            assert_eq!(m.pos_at_offset(7.75), (row, frac));
            assert_eq!(m.pos_at_offset(-1.0), (RowPos::new(0, 0), 0.0));
        });
    }
}
