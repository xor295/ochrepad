//! Code folding (PLAN_v0.2.0 N3): where regions are, and which are folded
//! in a view.
//!
//! Regions come from indentation, or from headings in Markdown. A line is a
//! fold header when the next non-blank line is indented deeper; its region
//! runs to the last line before the next non-blank line indented no deeper,
//! leaving trailing blank lines visible (so a closing `}` at the header's
//! indent stays visible, as in VS Code). Detection never scans the whole
//! document except for the fold-all and level commands.
//!
//! A [`FoldSet`] is per view. Each fold is stored as a byte range so it maps
//! through edits like selections do; the rules for edits and carets live in
//! `Editor` (PLAN_v0.2.0 3.6).

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::changeset::{Bias, ChangeSet};
use crate::position::{ByteOffset, byte_to_visual_col};
use crate::text::Document;

/// How a buffer's fold regions are found. Chosen by the app from the
/// buffer's language.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FoldStrategy {
    #[default]
    Indent,
    /// Headings fold to the next heading of the same or a higher level.
    Markdown,
}

/// Blank lines skipped when looking for the line after a header; past this
/// a line does not count as a header (keeps the gutter cheap).
const MAX_BLANK_RUN: usize = 1000;
/// Lines looked back through to tell whether a `#` line is inside a fenced
/// code block. A fence opened further up is missed (documented).
const FENCE_LOOKBACK: usize = 200;

/// The indentation width of `line` in cells, or `None` for a blank line.
fn indent(doc: &Document, line: usize, tab_width: usize) -> Option<usize> {
    let text = doc.line_text(line);
    let rest = text.trim_start_matches([' ', '\t']);
    if rest.is_empty() {
        return None;
    }
    let lead = text.len() - rest.len();
    Some(byte_to_visual_col(doc, ByteOffset(doc.line_start(line).0 + lead), tab_width).0)
}

/// The first non-blank line after `line` and its indent, looking at most
/// `limit` lines ahead.
fn next_non_blank(
    doc: &Document,
    line: usize,
    tab_width: usize,
    limit: usize,
) -> Option<(usize, usize)> {
    (line + 1..doc.line_count().min((line + 1).saturating_add(limit)))
        .find_map(|l| indent(doc, l, tab_width).map(|w| (l, w)))
}

/// A Markdown ATX heading's level (1–6).
fn heading_level(text: &str) -> Option<usize> {
    let t = text.strip_prefix("   ").or_else(|| text.strip_prefix("  "));
    let t = t.or_else(|| text.strip_prefix(' ')).unwrap_or(text);
    let hashes = t.len() - t.trim_start_matches('#').len();
    let after = &t[hashes..];
    ((1..=6).contains(&hashes) && (after.is_empty() || after.starts_with([' ', '\t'])))
        .then_some(hashes)
}

fn is_fence(text: &str) -> bool {
    let t = text.trim_start_matches(' ');
    text.len() - t.len() <= 3 && (t.starts_with("```") || t.starts_with("~~~"))
}

/// Whether `line` is inside a fenced code block, judged from the fences in
/// the [`FENCE_LOOKBACK`] lines above it.
fn in_fence(doc: &Document, line: usize) -> bool {
    let fences = (line.saturating_sub(FENCE_LOOKBACK)..line)
        .filter(|&l| is_fence(&doc.line_text(l)))
        .count();
    fences % 2 == 1
}

/// The heading level of `line` if it is a heading outside a code fence.
fn markdown_heading(doc: &Document, line: usize) -> Option<usize> {
    heading_level(&doc.line_text(line)).filter(|_| !in_fence(doc, line))
}

/// The last line `header` folds, or `None` if it is not a fold header. The
/// region is `header + 1 ..= last`.
pub fn region(
    doc: &Document,
    header: usize,
    strategy: FoldStrategy,
    tab_width: usize,
) -> Option<usize> {
    if header >= doc.line_count() {
        return None;
    }
    let mut last = header;
    match strategy {
        FoldStrategy::Indent => {
            let w = indent(doc, header, tab_width)?;
            for l in header + 1..doc.line_count() {
                match indent(doc, l, tab_width) {
                    None => {}
                    Some(i) if i <= w => break,
                    Some(_) => last = l,
                }
            }
        }
        FoldStrategy::Markdown => {
            let level = markdown_heading(doc, header)?;
            let mut fenced = false;
            for l in header + 1..doc.line_count() {
                let text = doc.line_text(l);
                if is_fence(&text) {
                    fenced = !fenced;
                } else if !fenced && heading_level(&text).is_some_and(|h| h <= level) {
                    break;
                }
                if fenced || !text.trim().is_empty() {
                    last = l;
                }
            }
        }
    }
    (last > header).then_some(last)
}

/// Whether `line` starts a fold region. Cheaper than [`region`]: it only
/// looks at the next non-blank line.
pub fn is_header(doc: &Document, line: usize, strategy: FoldStrategy, tab_width: usize) -> bool {
    if line >= doc.line_count() {
        return false;
    }
    match strategy {
        FoldStrategy::Indent => indent(doc, line, tab_width).is_some_and(|w| {
            next_non_blank(doc, line, tab_width, MAX_BLANK_RUN).is_some_and(|(_, n)| n > w)
        }),
        FoldStrategy::Markdown => {
            let Some(level) = heading_level(&doc.line_text(line)) else {
                return false;
            };
            // Folds unless the next non-blank line is a heading that ends it.
            let next = next_non_blank(doc, line, tab_width, MAX_BLANK_RUN);
            next.is_some_and(|(l, _)| {
                let text = doc.line_text(l);
                is_fence(&text) || heading_level(&text).is_none_or(|h| h > level)
            }) && !in_fence(doc, line)
        }
    }
}

/// A fold region: its header line, its level (the nesting depth for
/// indentation, 1 being outermost; the heading level for Markdown) and its
/// last line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub header: usize,
    pub level: usize,
    pub last: usize,
}

/// Every fold region, in header order. Scans the whole document once.
pub fn regions(doc: &Document, strategy: FoldStrategy, tab_width: usize) -> Vec<Region> {
    let mut out = Vec::new();
    match strategy {
        FoldStrategy::Indent => {
            // Regions still open: (indent, index into `out`).
            let mut open: Vec<(usize, usize)> = Vec::new();
            let mut last_non_blank = 0;
            let mut prev: Option<(usize, usize)> = None;
            let close = |open: &mut Vec<(usize, usize)>, out: &mut Vec<Region>, w, last| {
                while let Some(&(o, i)) = open.last() {
                    if o < w {
                        break;
                    }
                    out[i].last = last;
                    open.pop();
                }
            };
            for l in 0..doc.line_count() {
                let Some(w) = indent(doc, l, tab_width) else {
                    continue;
                };
                // The previous non-blank line heads a region if this one is
                // indented deeper.
                if let Some((pl, pw)) = prev
                    && w > pw
                {
                    out.push(Region {
                        header: pl,
                        level: open.len() + 1,
                        last: pl,
                    });
                    open.push((pw, out.len() - 1));
                }
                close(&mut open, &mut out, w, last_non_blank);
                last_non_blank = l;
                prev = Some((l, w));
            }
            close(&mut open, &mut out, 0, last_non_blank);
        }
        FoldStrategy::Markdown => {
            let mut fenced = false;
            for l in 0..doc.line_count() {
                let text = doc.line_text(l);
                if is_fence(&text) {
                    fenced = !fenced;
                } else if !fenced
                    && let Some(level) = heading_level(&text)
                    && let Some(last) = region(doc, l, strategy, tab_width)
                {
                    out.push(Region {
                        header: l,
                        level,
                        last,
                    });
                }
            }
        }
    }
    out
}

/// The innermost region holding `line`: its header (`line` itself if it is
/// one) and last line.
pub fn enclosing(
    doc: &Document,
    line: usize,
    strategy: FoldStrategy,
    tab_width: usize,
) -> Option<(usize, usize)> {
    if let Some(last) = region(doc, line, strategy, tab_width) {
        return Some((line, last));
    }
    match strategy {
        FoldStrategy::Indent => {
            // A blank line belongs with the line after it.
            let w = indent(doc, line, tab_width)
                .or_else(|| next_non_blank(doc, line, tab_width, usize::MAX).map(|(_, w)| w))?;
            let header = (0..line)
                .rev()
                .find(|&l| indent(doc, l, tab_width).is_some_and(|i| i < w))?;
            let last = region(doc, header, strategy, tab_width)?;
            (last >= line).then_some((header, last))
        }
        FoldStrategy::Markdown => (0..line).rev().find_map(|l| {
            let last = region(doc, l, strategy, tab_width)?;
            (last >= line).then_some((l, last))
        }),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Fold {
    /// The hidden text: from the end of the header line (its line break) to
    /// the end of the last hidden line.
    range: Range<ByteOffset>,
    header: usize,
    last: usize,
}

/// The folded regions of one view, possibly nested.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FoldSet {
    /// Sorted by start; any two are nested or disjoint.
    folds: Vec<Fold>,
    /// The hidden lines: the union of the folds' regions as sorted, disjoint
    /// inclusive ranges, with the hidden lines before each.
    hidden: Vec<(usize, usize, usize)>,
    /// Mapped through an edit since the line numbers were computed.
    stale: bool,
}

impl FoldSet {
    pub fn is_empty(&self) -> bool {
        self.folds.is_empty()
    }

    /// The folded header lines, in order.
    pub fn headers(&self) -> impl Iterator<Item = usize> + '_ {
        self.folds.iter().map(|f| f.header)
    }

    pub fn is_folded(&self, header: usize) -> bool {
        self.folds.iter().any(|f| f.header == header)
    }

    /// The hidden line range (inclusive) holding `line`.
    pub fn hidden_range(&self, line: usize) -> Option<(usize, usize)> {
        let i = self.hidden.partition_point(|h| h.1 < line);
        self.hidden
            .get(i)
            .filter(|h| h.0 <= line)
            .map(|h| (h.0, h.1))
    }

    pub fn is_hidden(&self, line: usize) -> bool {
        self.hidden_range(line).is_some()
    }

    /// Hidden lines in total.
    pub fn hidden_count(&self) -> usize {
        self.hidden.last().map_or(0, |h| h.2 + h.1 + 1 - h.0)
    }

    /// Hidden lines before `line`.
    pub fn hidden_before(&self, line: usize) -> usize {
        let i = self.hidden.partition_point(|h| h.0 < line);
        match i.checked_sub(1).map(|i| self.hidden[i]) {
            None => 0,
            Some((start, end, before)) => before + end.min(line - 1) + 1 - start,
        }
    }

    /// The line that is the `index`-th visible line (0-based), or the last
    /// visible one if there are fewer.
    pub fn visible_line(&self, index: usize, line_count: usize) -> usize {
        // A hidden range comes after `start - before` visible lines; the
        // ranges before the wanted line are those it comes after.
        let i = self.hidden.partition_point(|h| h.0 - h.2 <= index);
        let line = match i.checked_sub(1).map(|i| self.hidden[i]) {
            None => index,
            Some((start, end, before)) => index + before + end + 1 - start,
        };
        line.min(line_count.saturating_sub(1))
    }

    /// Folds each `(header, last)`: lines `header + 1 ..= last`.
    pub(crate) fn insert(
        &mut self,
        doc: &Document,
        regions: impl IntoIterator<Item = (usize, usize)>,
    ) {
        for (header, last) in regions {
            if last > header && last < doc.line_count() {
                self.folds.push(Fold {
                    range: doc.line_end(header)..doc.line_end(last),
                    header,
                    last,
                });
            }
        }
        self.folds.sort_by_key(|f| f.range.start);
        self.folds.dedup_by_key(|f| f.header);
        self.rebuild();
    }

    /// Unfolds the fold at `header`; returns whether there was one.
    pub(crate) fn remove(&mut self, header: usize) -> bool {
        let before = self.folds.len();
        self.folds.retain(|f| f.header != header);
        self.rebuild();
        self.folds.len() != before
    }

    /// Keeps the folds whose (header, last) pass `keep`.
    pub(crate) fn retain(&mut self, mut keep: impl FnMut(usize, usize) -> bool) {
        self.folds.retain(|f| keep(f.header, f.last));
        self.rebuild();
    }

    /// Maps the folds through an edit. A fold whose hidden text the edit
    /// touches is dropped; typing at the end of a header line is not inside
    /// it. Line numbers are stale until [`relines`](Self::relines).
    pub(crate) fn map(&mut self, changes: &ChangeSet) {
        self.stale = true;
        self.folds.retain_mut(|f| {
            let touched = changes.edits().iter().any(|e| {
                if e.start == e.end {
                    f.range.start < e.start && e.start <= f.range.end
                } else {
                    e.start <= f.range.end && e.end > f.range.start
                }
            });
            f.range = changes.map_position(f.range.start, Bias::After)
                ..changes.map_position(f.range.end, Bias::Before);
            !touched
        });
    }

    /// Recomputes line numbers after [`map`](Self::map).
    pub(crate) fn relines(&mut self, doc: &Document) {
        if !std::mem::take(&mut self.stale) {
            return;
        }
        for f in &mut self.folds {
            f.header = doc.line_of(f.range.start);
            f.last = doc.line_of(f.range.end);
        }
        self.folds.retain(|f| f.last > f.header);
        self.rebuild();
    }

    fn rebuild(&mut self) {
        self.hidden.clear();
        let mut hidden_so_far = 0;
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for f in &self.folds {
            let (a, b) = (f.header + 1, f.last);
            match merged.last_mut() {
                Some(last) if a <= last.1 + 1 => last.1 = last.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        for (a, b) in merged {
            self.hidden.push((a, b, hidden_so_far));
            hidden_so_far += b + 1 - a;
        }
    }
}
