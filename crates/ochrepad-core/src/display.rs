//! Display rows: how a view's document lines map to rows on screen
//! (PLAN_v0.2.0 3.2). A row is addressed relative to its line, as a
//! [`RowPos`], so nothing needs a global row index: with word wrap that
//! would mean wrapping the whole file. Lines hidden by a fold have no rows;
//! with word wrap on, a line has one row per wrapped piece, otherwise one.
//!
//! The editor widget scrolls, paints and hit-tests through a [`DisplayMap`],
//! and vertical motions resolve through one, so a macro's `Up` moves one
//! row in whatever view plays it.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::buffer::Buffer;
use crate::fold::FoldSet;
use crate::history::Revision;
use crate::position::{
    ByteOffset, VisualCol, Wrapped, byte_to_visual_col, prev_grapheme_boundary, visual_col_to_byte,
    wrap_line,
};
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

/// Wrapped lines of one view, for one (width, tab width). An entry made at
/// an older revision is still good if the line's text is unchanged, so an
/// edit only rewraps the lines it touched (or shifted).
#[derive(Clone, Debug, Default)]
pub(crate) struct WrapCache {
    key: Option<(usize, usize)>,
    lines: HashMap<usize, WrapEntry>,
}

#[derive(Clone, Debug)]
struct WrapEntry {
    /// The revision this entry was last checked at.
    revision: Revision,
    text: String,
    wrapped: Arc<Wrapped>,
}

/// Lines kept in a [`WrapCache`] before it starts over.
const WRAP_CACHE_LINES: usize = 4096;

/// A view's rows over its document.
#[derive(Clone, Copy)]
pub struct DisplayMap<'a> {
    doc: &'a Document,
    revision: Revision,
    folds: &'a FoldSet,
    wrap: Option<usize>,
    cache: &'a std::cell::RefCell<WrapCache>,
    tab_width: usize,
}

impl<'a> DisplayMap<'a> {
    pub fn new(buffer: &'a Buffer, view: &'a View, tab_width: usize) -> Self {
        Self {
            doc: buffer.doc(),
            revision: buffer.revision(),
            folds: &view.folds,
            wrap: view.wrap_cols,
            cache: &view.wrap_cache,
            tab_width,
        }
    }

    pub fn doc(&self) -> &'a Document {
        self.doc
    }

    /// Whether lines wrap.
    pub fn wraps(&self) -> bool {
        self.wrap.is_some()
    }

    /// How `line` wraps, or `None` when wrap is off.
    fn wrapped(&self, line: usize) -> Option<Arc<Wrapped>> {
        let cols = self.wrap?;
        let mut cache = self.cache.borrow_mut();
        let key = (cols, self.tab_width);
        if cache.key != Some(key) || cache.lines.len() > WRAP_CACHE_LINES {
            cache.key = Some(key);
            cache.lines.clear();
        }
        if let Some(e) = cache.lines.get_mut(&line) {
            if e.revision == self.revision {
                return Some(e.wrapped.clone());
            }
            let text = self.doc.line_text(line);
            if text == e.text {
                e.revision = self.revision;
                return Some(e.wrapped.clone());
            }
        }
        let text = self.doc.line_text(line);
        let wrapped = Arc::new(wrap_line(&text, cols, self.tab_width));
        let entry = WrapEntry {
            revision: self.revision,
            text,
            wrapped: wrapped.clone(),
        };
        cache.lines.insert(line, entry);
        Some(wrapped)
    }

    /// Rows that `line` takes on screen: 0 for a hidden line.
    pub fn rows_in_line(&self, line: usize) -> usize {
        if self.folds.is_hidden(line) {
            0
        } else {
            self.wrapped(line).map_or(1, |w| w.starts.len())
        }
    }

    /// Lines that are not hidden. Sizes the scrollbar.
    pub fn visible_line_count(&self) -> usize {
        self.doc.line_count() - self.folds.hidden_count()
    }

    /// `line`, or the fold header above it if it is hidden.
    fn shown_line(&self, line: usize) -> usize {
        match self.folds.hidden_range(line) {
            Some((start, _)) => start - 1,
            None => line,
        }
    }

    /// How many visible lines come before `line` (which must be visible).
    fn visible_index(&self, line: usize) -> usize {
        line - self.folds.hidden_before(line)
    }

    fn next_visible_line(&self, line: usize) -> Option<usize> {
        let next = match self.folds.hidden_range(line + 1) {
            Some((_, end)) => end + 1,
            None => line + 1,
        };
        (next < self.doc.line_count()).then_some(next)
    }

    fn prev_visible_line(&self, line: usize) -> Option<usize> {
        let prev = line.checked_sub(1)?;
        Some(self.shown_line(prev))
    }

    pub fn first_row(&self) -> RowPos {
        RowPos::default()
    }

    pub fn last_row(&self) -> RowPos {
        let line = self.shown_line(self.doc.line_count() - 1);
        RowPos::new(line, self.rows_in_line(line) - 1)
    }

    /// The nearest real row: a line past the end becomes the last line, a
    /// hidden line its fold's header, and a row past the line's last row
    /// (after the text or the width changed) that last row.
    pub fn clamp_row(&self, row: RowPos) -> RowPos {
        let line = self.shown_line(row.line.min(self.doc.line_count() - 1));
        RowPos::new(line, row.sub.min(self.rows_in_line(line) - 1))
    }

    /// The row `n` rows after `row` (before it for negative `n`), stopping at
    /// the first and last rows, and how many rows it actually moved.
    pub fn step(&self, row: RowPos, n: isize) -> (RowPos, isize) {
        if !self.wraps() {
            // One row per visible line: count in visible lines.
            let from = self.visible_index(self.shown_line(row.line)) as isize;
            let last = self.visible_line_count() as isize - 1;
            let to = (from + n).clamp(0, last);
            let line = self.folds.visible_line(to as usize, self.doc.line_count());
            return (RowPos::new(line, 0), to - from);
        }
        let mut row = self.clamp_row(row);
        let mut moved = 0;
        while moved < n {
            if row.sub + 1 < self.rows_in_line(row.line) {
                row.sub += 1;
            } else if let Some(line) = self.next_visible_line(row.line) {
                row = RowPos::new(line, 0);
            } else {
                break;
            }
            moved += 1;
        }
        while moved > n {
            if row.sub > 0 {
                row.sub -= 1;
            } else if let Some(line) = self.prev_visible_line(row.line) {
                row = RowPos::new(line, self.rows_in_line(line) - 1);
            } else {
                break;
            }
            moved -= 1;
        }
        (row, moved)
    }

    /// The row `n` rows after `row`, clamped to the document.
    pub fn advance(&self, row: RowPos, n: isize) -> RowPos {
        self.step(row, n).0
    }

    /// The text shown on `row`, without the line break. A row that ends at a
    /// wrap point ends where the next row starts.
    pub fn row_range(&self, row: RowPos) -> Range<ByteOffset> {
        let start = self.doc.line_start(row.line);
        let end = self.doc.line_end(row.line);
        match self.wrapped(row.line) {
            None => start..end,
            Some(w) => {
                let at = |i: usize| ByteOffset(start.0 + w.starts[i]);
                let sub = row.sub.min(w.starts.len() - 1);
                at(sub)..w.starts.get(sub + 1).map_or(end, |_| at(sub + 1))
            }
        }
    }

    /// Whether `row` is its line's last row.
    pub fn is_last_in_line(&self, row: RowPos) -> bool {
        row.sub + 1 >= self.rows_in_line(row.line)
    }

    /// The last caret position on `row`. A position at a wrap point shows
    /// at the start of the next row, so a row that wraps ends one grapheme
    /// before it.
    pub fn row_end(&self, row: RowPos) -> ByteOffset {
        let range = self.row_range(row);
        if self.is_last_in_line(row) {
            range.end
        } else {
            prev_grapheme_boundary(self.doc, range.end).max(range.start)
        }
    }

    /// Cells the text of `row` is indented by (continuation rows only).
    pub fn row_indent(&self, row: RowPos) -> usize {
        match self.wrapped(row.line) {
            Some(w) if row.sub > 0 => w.indent,
            _ => 0,
        }
    }

    /// The row showing `pos`; for a position hidden by a fold, the fold's
    /// header row.
    pub fn row_of(&self, pos: ByteOffset) -> RowPos {
        let line = self.doc.line_of(pos);
        let shown = self.shown_line(line);
        if shown != line {
            return RowPos::new(shown, self.rows_in_line(shown) - 1);
        }
        match self.wrapped(line) {
            None => RowPos::new(line, 0),
            Some(w) => {
                let off = pos.0 - self.doc.line_start(line).0;
                RowPos::new(line, w.starts.partition_point(|&s| s <= off) - 1)
            }
        }
    }

    /// The on-screen column of `pos` within its row, counting a
    /// continuation row's indent, so that Up and Down keep carets above
    /// each other.
    pub fn col_in_row(&self, pos: ByteOffset) -> VisualCol {
        let col = byte_to_visual_col(self.doc, pos, self.tab_width).0;
        let row = self.row_of(pos);
        match self.wrapped(row.line) {
            Some(w) if row.sub > 0 => VisualCol(w.indent + col - w.start_cols[row.sub]),
            _ => VisualCol(col),
        }
    }

    /// The position at on-screen column `col` of `row`, or the row's end if
    /// it is shorter.
    pub fn byte_at_col(&self, row: RowPos, col: VisualCol) -> ByteOffset {
        let target = match self.wrapped(row.line) {
            Some(w) if row.sub > 0 => w.start_cols[row.sub] + col.0.saturating_sub(w.indent),
            _ => col.0,
        };
        let at = visual_col_to_byte(self.doc, row.line, VisualCol(target), self.tab_width);
        at.clamp(self.row_range(row).start, self.row_end(row))
    }

    /// A scroll position as a number of visible lines from the top, for the
    /// scrollbar: the visible lines above `row`'s line, plus the part of that
    /// line scrolled past.
    pub fn offset_of(&self, row: RowPos, frac: f32) -> f64 {
        let rows = self.rows_in_line(row.line).max(1) as f64;
        self.visible_index(self.shown_line(row.line)) as f64 + (row.sub as f64 + frac as f64) / rows
    }

    /// The inverse of [`offset_of`](Self::offset_of): the top row and fraction
    /// for a scrollbar offset.
    pub fn pos_at_offset(&self, offset: f64) -> (RowPos, f32) {
        let max = self.visible_line_count().saturating_sub(1) as f64;
        let offset = offset.clamp(0.0, max + 0.999_999);
        let index = offset.floor();
        let line = self
            .folds
            .visible_line(index as usize, self.doc.line_count());
        let rows = self.rows_in_line(line).max(1) as f64;
        let within = (offset - index) * rows;
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

    fn with_map(text: &str, wrap: Option<usize>, f: impl FnOnce(DisplayMap)) {
        let mut ed = Editor::new();
        let buffer = ed.add_buffer(text, Default::default());
        let view = ed.add_view(buffer).unwrap();
        ed.view_mut(view).unwrap().wrap_cols = wrap;
        let v = ed.view(view).unwrap();
        f(DisplayMap::new(ed.buffer(buffer).unwrap(), v, 4));
    }

    #[test]
    fn identity_rows_are_lines() {
        with_map("a\nbb\n\tc", None, |m| {
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
        with_map(&"x\n".repeat(99), None, |m| {
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

    #[test]
    fn wrapped_rows_step_and_map_positions() {
        // "  aaaa bbbb cccc" at width 10: "  aaaa " / "bbbb " / "cccc",
        // the continuation rows indented by 2.
        with_map("x\n  aaaa bbbb cccc\ny", Some(10), |m| {
            assert_eq!(m.rows_in_line(1), 3);
            let line = 2; // start of line 1
            assert_eq!(
                m.row_range(RowPos::new(1, 1)),
                ByteOffset(line + 7)..ByteOffset(line + 12)
            );
            assert_eq!(
                m.row_of(ByteOffset(line + 7)),
                RowPos::new(1, 1),
                "wrap point"
            );
            assert_eq!(m.row_end(RowPos::new(1, 1)), ByteOffset(line + 11));
            assert_eq!(m.row_end(RowPos::new(1, 2)), ByteOffset(line + 16));
            assert_eq!(m.row_indent(RowPos::new(1, 2)), 2);
            // The second "b" (line column 8) is at column 3 of its row,
            // counting the indent.
            assert_eq!(m.col_in_row(ByteOffset(line + 8)), VisualCol(3));
            assert_eq!(
                m.byte_at_col(RowPos::new(1, 2), VisualCol(3)),
                ByteOffset(line + 13)
            );
            assert_eq!(m.step(RowPos::new(0, 0), 3), (RowPos::new(1, 2), 3));
            assert_eq!(m.step(RowPos::new(2, 0), -2), (RowPos::new(1, 1), -2));
            assert_eq!(m.last_row(), RowPos::new(2, 0));
            assert_eq!(m.max_top(3), RowPos::new(1, 1));
            assert_eq!(m.clamp_row(RowPos::new(1, 9)), RowPos::new(1, 2));
        });
    }
}
