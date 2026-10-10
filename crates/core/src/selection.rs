//! Selections are always a set (PLAN 3.3). A plain cursor is a set of one empty
//! selection.

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::position::{ByteOffset, VisualCol};
use crate::text::Document;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    pub anchor: ByteOffset,
    pub head: ByteOffset,
    /// Remembered visual column for consecutive vertical movement.
    pub goal: Option<VisualCol>,
}

impl Selection {
    pub fn caret(pos: ByteOffset) -> Self {
        Self::new(pos, pos)
    }

    pub fn new(anchor: ByteOffset, head: ByteOffset) -> Self {
        Self {
            anchor,
            head,
            goal: None,
        }
    }

    pub fn with_goal(mut self, goal: Option<VisualCol>) -> Self {
        self.goal = goal;
        self
    }

    pub fn start(&self) -> ByteOffset {
        self.anchor.min(self.head)
    }

    pub fn end(&self) -> ByteOffset {
        self.anchor.max(self.head)
    }

    pub fn range(&self) -> Range<ByteOffset> {
        self.start()..self.end()
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    fn is_backward(&self) -> bool {
        self.head < self.anchor
    }
}

/// The rectangle a column selection was made from, kept alongside the set so
/// rectangular copy/paste and column-keyed sort can use it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rectangle {
    pub start_line: usize,
    pub end_line: usize,
    pub start_col: VisualCol,
    pub end_col: VisualCol,
}

/// Non-empty, sorted, non-overlapping selections plus the primary's index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionSet {
    ranges: Vec<Selection>,
    primary: usize,
    rect: Option<Rectangle>,
}

impl SelectionSet {
    pub fn single(sel: Selection) -> Self {
        Self {
            ranges: vec![sel],
            primary: 0,
            rect: None,
        }
    }

    pub fn caret(pos: ByteOffset) -> Self {
        Self::single(Selection::caret(pos))
    }

    /// Builds a normalized set. `primary` indexes into `ranges` as given.
    /// An empty `ranges` yields a caret at 0.
    pub fn new(ranges: Vec<Selection>, primary: usize) -> Self {
        if ranges.is_empty() {
            return Self::caret(ByteOffset(0));
        }
        let primary = primary.min(ranges.len() - 1);
        let mut set = Self {
            ranges,
            primary,
            rect: None,
        };
        set.normalize();
        set
    }

    pub fn ranges(&self) -> &[Selection] {
        &self.ranges
    }

    pub fn primary(&self) -> &Selection {
        &self.ranges[self.primary]
    }

    pub fn primary_index(&self) -> usize {
        self.primary
    }

    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn rect(&self) -> Option<&Rectangle> {
        self.rect.as_ref()
    }

    pub fn with_rect(mut self, rect: Option<Rectangle>) -> Self {
        self.rect = rect;
        self
    }

    /// Applies `f` to every selection, renormalizes and drops the rectangle.
    pub fn map(&self, mut f: impl FnMut(&Selection) -> Selection) -> Self {
        Self::new(self.ranges.iter().map(&mut f).collect(), self.primary)
    }

    /// Sorts, then merges overlapping or touching selections. The merged
    /// selection keeps the primary's direction if it absorbed the primary,
    /// otherwise the first one's.
    fn normalize(&mut self) {
        let primary = self.primary;
        let mut indexed: Vec<(usize, Selection)> = std::mem::take(&mut self.ranges)
            .into_iter()
            .enumerate()
            .collect();
        indexed.sort_by_key(|(_, s)| (s.start(), s.end()));

        let mut out: Vec<Selection> = Vec::with_capacity(indexed.len());
        let mut new_primary = 0;
        // Which original selection decides the direction of `out.last()`.
        let mut leader: Option<(usize, Selection)> = None;
        for (i, sel) in indexed {
            match out.last_mut() {
                Some(last) if sel.start() <= last.end() => {
                    let (start, end) = (last.start(), last.end().max(sel.end()));
                    let (lead_idx, lead) = match leader.take() {
                        Some(l) if l.0 == primary || i != primary => l,
                        _ => (i, sel),
                    };
                    *last = if lead.is_backward() {
                        Selection {
                            anchor: end,
                            head: start,
                            goal: lead.goal,
                        }
                    } else {
                        Selection {
                            anchor: start,
                            head: end,
                            goal: lead.goal,
                        }
                    };
                    if i == primary {
                        new_primary = out.len() - 1;
                    }
                    leader = Some((lead_idx, lead));
                }
                _ => {
                    if i == primary {
                        new_primary = out.len();
                    }
                    leader = Some((i, sel.clone()));
                    out.push(sel);
                }
            }
        }
        self.ranges = out;
        self.primary = new_primary;
    }

    /// Checks the invariants; used by tests and debug assertions.
    pub fn validate(&self, doc: &Document) -> Result<(), String> {
        if self.ranges.is_empty() || self.primary >= self.ranges.len() {
            return Err(format!("bad primary/len: {self:?}"));
        }
        for s in &self.ranges {
            for p in [s.anchor, s.head] {
                if p > doc.len() || !doc.is_char_boundary(p) {
                    return Err(format!("{p:?} out of range or not a char boundary"));
                }
            }
        }
        for w in self.ranges.windows(2) {
            if w[0].end() >= w[1].start() {
                return Err(format!(
                    "unsorted, overlapping or touching: {:?}",
                    self.ranges
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(a: usize, h: usize) -> Selection {
        Selection::new(ByteOffset(a), ByteOffset(h))
    }

    #[test]
    fn normalize_sorts_and_merges() {
        let set = SelectionSet::new(vec![sel(10, 12), sel(0, 2), sel(5, 5), sel(5, 5)], 0);
        assert_eq!(set.ranges(), &[sel(0, 2), sel(5, 5), sel(10, 12)]);
        assert_eq!(set.primary(), &sel(10, 12));
    }

    #[test]
    fn touching_selections_merge_keeping_primary_direction() {
        let set = SelectionSet::new(vec![sel(0, 3), sel(6, 3)], 1);
        assert_eq!(set.ranges(), &[sel(6, 0)]);
        assert_eq!(set.primary_index(), 0);

        let set = SelectionSet::new(vec![sel(0, 3), sel(6, 3), sel(9, 9)], 2);
        assert_eq!(set.ranges(), &[sel(0, 6), sel(9, 9)]);
        assert_eq!(set.primary_index(), 1);
    }

    #[test]
    fn caret_inside_selection_merges() {
        let set = SelectionSet::new(vec![sel(2, 8), sel(4, 4)], 1);
        assert_eq!(set.ranges(), &[sel(2, 8)]);
        assert_eq!(set.primary_index(), 0);
    }
}
