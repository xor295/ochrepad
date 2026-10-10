//! Line and word diff for the compare view (PLAN M12), using `similar`.
//!
//! The result is a list of aligned rows: each row shows a line of the left
//! document, a line of the right one, or both. A missing side is a filler
//! row, so equal text stays side by side. [`SideMap`] exposes one side's
//! rows as a [`DisplayMap`].

use std::ops::Range;
use std::time::Duration;

use similar::{Algorithm, ChangeTag, DiffOp, TextDiff};

use crate::text::Document;
use crate::view::DisplayMap;

/// Give up refining a diff after this long and report a coarser one.
pub const DIFF_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Equal,
    /// Only on the right.
    Added,
    /// Only on the left.
    Removed,
    /// On both sides, with different text.
    Changed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffRow {
    pub kind: RowKind,
    pub left: Option<usize>,
    pub right: Option<usize>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiffStats {
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineDiff {
    pub rows: Vec<DiffRow>,
    /// Row ranges of consecutive non-equal rows, for next/previous change.
    pub hunks: Vec<Range<usize>>,
    pub stats: DiffStats,
}

fn lines(doc: &Document) -> Vec<String> {
    (0..doc.line_count()).map(|l| doc.line_text(l)).collect()
}

/// Aligns the lines of `left` and `right`. Replaced blocks are paired line
/// by line as `Changed`; the extra lines of the longer side are `Removed` or
/// `Added`.
pub fn diff_lines(left: &Document, right: &Document) -> LineDiff {
    let a = lines(left);
    let b = lines(right);
    let a_refs: Vec<&str> = a.iter().map(String::as_str).collect();
    let b_refs: Vec<&str> = b.iter().map(String::as_str).collect();
    let diff = TextDiff::configure()
        .algorithm(Algorithm::Myers)
        .timeout(DIFF_TIMEOUT)
        .diff_slices(&a_refs, &b_refs);
    let mut out = LineDiff::default();
    let push = |out: &mut LineDiff, kind, left, right| {
        if kind != RowKind::Equal {
            let row = out.rows.len();
            match out.hunks.last_mut() {
                Some(h) if h.end == row => h.end += 1,
                _ => out.hunks.push(row..row + 1),
            }
            match kind {
                RowKind::Added => out.stats.added += 1,
                RowKind::Removed => out.stats.removed += 1,
                RowKind::Changed => out.stats.changed += 1,
                RowKind::Equal => {}
            }
        }
        out.rows.push(DiffRow { kind, left, right });
    };
    for op in diff.ops() {
        match *op {
            DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => {
                for i in 0..len {
                    push(
                        &mut out,
                        RowKind::Equal,
                        Some(old_index + i),
                        Some(new_index + i),
                    );
                }
            }
            DiffOp::Delete {
                old_index, old_len, ..
            } => {
                for i in 0..old_len {
                    push(&mut out, RowKind::Removed, Some(old_index + i), None);
                }
            }
            DiffOp::Insert {
                new_index, new_len, ..
            } => {
                for i in 0..new_len {
                    push(&mut out, RowKind::Added, None, Some(new_index + i));
                }
            }
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                for i in 0..old_len.max(new_len) {
                    let l = (i < old_len).then_some(old_index + i);
                    let r = (i < new_len).then_some(new_index + i);
                    let kind = match (l, r) {
                        (Some(_), Some(_)) => RowKind::Changed,
                        (Some(_), None) => RowKind::Removed,
                        _ => RowKind::Added,
                    };
                    push(&mut out, kind, l, r);
                }
            }
        }
    }
    out
}

/// Byte ranges of the words that differ between two changed lines, on each
/// side. Adjacent ranges are merged.
pub fn diff_words(left: &str, right: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let diff = TextDiff::configure()
        .timeout(DIFF_TIMEOUT)
        .diff_words(left, right);
    let (mut a, mut b) = (Vec::new(), Vec::new());
    let (mut ia, mut ib) = (0, 0);
    let add = |ranges: &mut Vec<Range<usize>>, start: usize, len: usize| match ranges.last_mut() {
        Some(last) if last.end == start => last.end += len,
        _ => ranges.push(start..start + len),
    };
    for change in diff.iter_all_changes() {
        let len = change.value().len();
        match change.tag() {
            ChangeTag::Equal => {
                ia += len;
                ib += len;
            }
            ChangeTag::Delete => {
                add(&mut a, ia, len);
                ia += len;
            }
            ChangeTag::Insert => {
                add(&mut b, ib, len);
                ib += len;
            }
        }
    }
    (a, b)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// One side of a diff as a display map: row → line, with filler rows.
pub struct SideMap<'a> {
    pub diff: &'a LineDiff,
    pub side: Side,
}

impl DisplayMap for SideMap<'_> {
    fn row_count(&self, _line_count: usize) -> usize {
        self.diff.rows.len()
    }

    fn line_to_row(&self, line: usize) -> usize {
        let of = |r: &DiffRow| match self.side {
            Side::Left => r.left,
            Side::Right => r.right,
        };
        self.diff
            .rows
            .iter()
            .position(|r| of(r) == Some(line))
            .unwrap_or(self.diff.rows.len())
    }

    fn row_to_line(&self, row: usize) -> Option<usize> {
        let r = self.diff.rows.get(row)?;
        match self.side {
            Side::Left => r.left,
            Side::Right => r.right,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(d: &LineDiff) -> Vec<(RowKind, Option<usize>, Option<usize>)> {
        d.rows.iter().map(|r| (r.kind, r.left, r.right)).collect()
    }

    #[test]
    fn rows_align_with_fillers() {
        let left = Document::new("a\nb\nc\nd");
        let right = Document::new("a\nB\nc\nnew\nd");
        let d = diff_lines(&left, &right);
        use RowKind::*;
        assert_eq!(
            kinds(&d),
            vec![
                (Equal, Some(0), Some(0)),
                (Changed, Some(1), Some(1)),
                (Equal, Some(2), Some(2)),
                (Added, None, Some(3)),
                (Equal, Some(3), Some(4)),
            ]
        );
        assert_eq!(d.hunks, vec![1..2, 3..4]);
        assert_eq!(
            d.stats,
            DiffStats {
                added: 1,
                removed: 0,
                changed: 1
            }
        );
    }

    #[test]
    fn uneven_replacements_pair_then_spill() {
        let left = Document::new("x\none\ntwo\nthree\ny");
        let right = Document::new("x\nuno\ny");
        let d = diff_lines(&left, &right);
        use RowKind::*;
        assert_eq!(
            kinds(&d),
            vec![
                (Equal, Some(0), Some(0)),
                (Changed, Some(1), Some(1)),
                (Removed, Some(2), None),
                (Removed, Some(3), None),
                (Equal, Some(4), Some(2)),
            ]
        );
        assert_eq!(d.hunks, vec![1..4]);
    }

    #[test]
    fn identical_documents_have_no_hunks() {
        let doc = Document::new("same\ntext\n");
        let d = diff_lines(&doc, &doc.clone());
        assert!(d.hunks.is_empty());
        assert_eq!(d.rows.len(), 3);
    }

    #[test]
    fn word_ranges() {
        let (a, b) = diff_words("let total = price * qty;", "let total = cost * qty + tax;");
        let left = "let total = price * qty;";
        let right = "let total = cost * qty + tax;";
        let words = |s: &str, r: &[Range<usize>]| -> Vec<String> {
            r.iter().map(|r| s[r.clone()].to_owned()).collect()
        };
        assert_eq!(words(left, &a), vec!["price", "qty;"]);
        assert_eq!(words(right, &b), vec!["cost", "qty + tax;"]);
    }

    #[test]
    fn side_maps_are_display_maps() {
        let d = diff_lines(&Document::new("a\nb"), &Document::new("a\nnew\nb"));
        let left = SideMap {
            diff: &d,
            side: Side::Left,
        };
        let right = SideMap {
            diff: &d,
            side: Side::Right,
        };
        assert_eq!(left.row_count(2), 3);
        assert_eq!(
            (0..3).map(|r| left.row_to_line(r)).collect::<Vec<_>>(),
            vec![Some(0), None, Some(1)]
        );
        assert_eq!(left.line_to_row(1), 2);
        assert_eq!(right.line_to_row(1), 1);
    }
}
