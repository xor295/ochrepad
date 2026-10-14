//! The Column Editor (Notepad++'s Edit > Column Editor, Alt+C): the same
//! text, or a sequence of numbers, inserted at one column on many lines.
//!
//! Where it goes, as in Notepad++:
//! - with a column selection, on each of its lines at its left column (the
//!   text inside the rectangle is replaced);
//! - with several selections, at each of them (replacing it);
//! - with one caret, on its line and every line below it, at the caret's
//!   column (the empty line after a final line break is left out);
//! - with one selection, on each of its lines at the column it starts in.
//!
//! A line too short to reach the column is padded with spaces.

use std::collections::HashMap;
use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::changeset::{Bias, ChangeSet, Edit};
use crate::history::EditKind;
use crate::position::{ByteOffset, VisualCol, byte_to_visual_col, visual_col_to_byte};
use crate::resolve::Resolved;
use crate::selection::{Selection, SelectionSet};
use crate::text::Document;

/// What the Column Editor inserts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColumnInsert {
    /// The same text on every line.
    Text(String),
    /// One number per line.
    Numbers(NumberSequence),
}

/// Numbers for the Column Editor: `initial`, then `initial + increase`, and
/// so on, each on `repeat` lines in a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NumberSequence {
    pub initial: i64,
    pub increase: i64,
    /// How many lines get each number (at least 1).
    pub repeat: usize,
    pub leading: Leading,
    pub format: NumberFormat,
}

impl Default for NumberSequence {
    fn default() -> Self {
        Self {
            initial: 1,
            increase: 1,
            repeat: 1,
            leading: Leading::None,
            format: NumberFormat::Dec,
        }
    }
}

/// Padding that makes all the numbers equally wide.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Leading {
    #[default]
    None,
    /// Zeros between the sign and the digits.
    Zeros,
    /// Spaces before the number (right-aligned).
    Spaces,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NumberFormat {
    #[default]
    Dec,
    /// Upper-case hexadecimal digits, as Notepad++ writes them.
    Hex,
    Oct,
    Bin,
}

impl NumberSequence {
    /// The first `count` numbers, formatted. Arithmetic saturates rather
    /// than overflowing.
    pub fn render(&self, count: usize) -> Vec<String> {
        let repeat = self.repeat.max(1);
        let parts: Vec<(bool, String)> = (0..count)
            .map(|i| {
                let steps = i64::try_from(i / repeat).unwrap_or(i64::MAX);
                let value = self
                    .initial
                    .saturating_add(steps.saturating_mul(self.increase));
                (value < 0, digits(value.unsigned_abs(), self.format))
            })
            .collect();
        let digits_width = parts.iter().map(|(_, d)| d.len()).max().unwrap_or(0);
        let full_width = parts
            .iter()
            .map(|(neg, d)| d.len() + usize::from(*neg))
            .max()
            .unwrap_or(0);
        parts
            .into_iter()
            .map(|(neg, d)| {
                let sign = if neg { "-" } else { "" };
                match self.leading {
                    Leading::None => format!("{sign}{d}"),
                    Leading::Zeros => format!("{sign}{d:0>digits_width$}"),
                    Leading::Spaces => format!("{:>full_width$}", format!("{sign}{d}")),
                }
            })
            .collect()
    }
}

fn digits(n: u64, format: NumberFormat) -> String {
    match format {
        NumberFormat::Dec => n.to_string(),
        NumberFormat::Hex => format!("{n:X}"),
        NumberFormat::Oct => format!("{n:o}"),
        NumberFormat::Bin => format!("{n:b}"),
    }
}

impl ColumnInsert {
    /// The text for each of `count` lines.
    pub fn items(&self, count: usize) -> Vec<String> {
        match self {
            ColumnInsert::Text(text) => vec![text.clone(); count],
            ColumnInsert::Numbers(seq) => seq.render(count),
        }
    }
}

/// Where one item goes: the text it replaces (empty for an insertion), and
/// the spaces that first take a short line out to the column.
struct Target {
    range: Range<ByteOffset>,
    pad: usize,
}

/// The insertion point at visual column `col` of `line`: the grapheme
/// boundary at or before it, or the line's end plus padding.
fn at_column(doc: &Document, line: usize, col: VisualCol, tab_width: usize) -> Target {
    let end = doc.line_end(line);
    let width = byte_to_visual_col(doc, end, tab_width).0;
    if width < col.0 {
        Target {
            range: end..end,
            pad: col.0 - width,
        }
    } else {
        let at = visual_col_to_byte(doc, line, col, tab_width);
        Target {
            range: at..at,
            pad: 0,
        }
    }
}

/// Resolves the Column Editor's insertion (see the module documentation).
pub(crate) fn insert(
    doc: &Document,
    sels: &SelectionSet,
    what: &ColumnInsert,
    tab_width: usize,
) -> Resolved {
    if matches!(what, ColumnInsert::Text(t) if t.is_empty()) {
        return Resolved::Selections(sels.clone());
    }
    let many = sels.rect().is_some() || sels.len() > 1;
    let targets: Vec<Target> = if let Some(r) = sels.rect() {
        // A column selection has one selection per line; a line too short
        // to reach the rectangle has a caret at its end.
        let on_line: HashMap<usize, &Selection> = sels
            .ranges()
            .iter()
            .map(|s| (doc.line_of(s.start()), s))
            .collect();
        (r.top()..=r.bottom().min(doc.line_count() - 1))
            .map(|line| match on_line.get(&line) {
                Some(s) if byte_to_visual_col(doc, s.start(), tab_width) >= r.left() => Target {
                    range: s.range(),
                    pad: 0,
                },
                _ => at_column(doc, line, r.left(), tab_width),
            })
            .collect()
    } else if sels.len() > 1 {
        sels.ranges()
            .iter()
            .map(|s| Target {
                range: s.range(),
                pad: 0,
            })
            .collect()
    } else {
        let s = sels.primary();
        let first = doc.line_of(s.start());
        let last = if s.is_empty() {
            // Down to the end, without the empty line a final line break
            // leaves.
            let last = doc.line_count() - 1;
            if last > first && doc.line_start(last) == doc.line_end(last) {
                last - 1
            } else {
                last
            }
        } else {
            let last = doc.line_of(s.end());
            if last > first && s.end() == doc.line_start(last) {
                last - 1
            } else {
                last
            }
        };
        let col = byte_to_visual_col(doc, s.start(), tab_width);
        (first..=last)
            .map(|line| at_column(doc, line, col, tab_width))
            .collect()
    };
    let items = what.items(targets.len());
    let edits: Vec<Edit> = targets
        .iter()
        .zip(&items)
        .map(|(t, item)| {
            Edit::new(
                t.range.start,
                t.range.end,
                format!("{}{item}", " ".repeat(t.pad)),
            )
        })
        .collect();
    let changes = ChangeSet::new(doc.len(), edits);
    if changes.is_empty() {
        return Resolved::Selections(sels.clone());
    }
    // A caret after each item (only after the first for one caret, which
    // may have filled the rest of the document).
    let after_item = |t: &Target| Selection::caret(changes.map_position(t.range.end, Bias::After));
    let after = if many {
        SelectionSet::new(
            targets.iter().map(after_item).collect(),
            sels.primary_index(),
        )
    } else {
        SelectionSet::single(after_item(&targets[0]))
    };
    Resolved::Edit {
        changes,
        after,
        kind: EditKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(f: impl FnOnce(&mut NumberSequence)) -> NumberSequence {
        let mut s = NumberSequence::default();
        f(&mut s);
        s
    }

    #[test]
    fn numbers_count_repeat_and_pad() {
        assert_eq!(seq(|_| {}).render(3), ["1", "2", "3"]);
        assert_eq!(
            seq(|s| {
                s.initial = 0;
                s.increase = 5;
                s.repeat = 2;
            })
            .render(5),
            ["0", "0", "5", "5", "10"]
        );
        assert_eq!(
            seq(|s| {
                s.initial = 8;
                s.leading = Leading::Zeros;
            })
            .render(3),
            ["08", "09", "10"]
        );
        assert_eq!(
            seq(|s| {
                s.initial = 9;
                s.leading = Leading::Spaces;
            })
            .render(2),
            [" 9", "10"]
        );
    }

    #[test]
    fn number_formats_and_negative_numbers() {
        let hex = seq(|s| {
            s.initial = 9;
            s.format = NumberFormat::Hex;
        });
        assert_eq!(hex.render(3), ["9", "A", "B"]);
        let bin = seq(|s| {
            s.initial = 1;
            s.format = NumberFormat::Bin;
            s.leading = Leading::Zeros;
        });
        assert_eq!(bin.render(3), ["01", "10", "11"]);
        let oct = seq(|s| {
            s.initial = 7;
            s.format = NumberFormat::Oct;
        });
        assert_eq!(oct.render(2), ["7", "10"]);
        let down = seq(|s| {
            s.initial = 1;
            s.increase = -1;
            s.leading = Leading::Zeros;
        });
        assert_eq!(down.render(3), ["1", "0", "-1"]);
        let spaced = seq(|s| {
            s.initial = 1;
            s.increase = -10;
            s.leading = Leading::Spaces;
        });
        assert_eq!(spaced.render(2), [" 1", "-9"]);
        // Repeat 0 counts as 1, and huge steps saturate.
        let edge = seq(|s| {
            s.initial = i64::MAX - 1;
            s.repeat = 0;
        });
        assert_eq!(
            edge.render(3),
            [
                (i64::MAX - 1).to_string(),
                i64::MAX.to_string(),
                i64::MAX.to_string()
            ]
        );
    }
}
