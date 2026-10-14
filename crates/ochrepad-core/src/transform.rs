//! Text transforms: case conversion, line sorting, and line operations
//! (trim, duplicate, move, join, remove empty or duplicate lines). Each
//! resolves to one transaction.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashSet};
use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::changeset::{Bias, ChangeSet, Edit};
use crate::history::EditKind;
use crate::position::{ByteOffset, visual_col_to_byte, visual_col_to_byte_ceil};
use crate::resolve::Resolved;
use crate::selection::{Selection, SelectionSet};
use crate::text::Document;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaseMode {
    Upper,
    Lower,
    /// First letter of each word upper case, the rest lower case.
    Title,
    /// Swap the case of every letter.
    Invert,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortOptions {
    pub descending: bool,
    pub case_insensitive: bool,
    /// Compare by the number at the start of each line (integer or
    /// decimal); lines without one sort first.
    pub numeric: bool,
    /// Drop lines equal to the line before them after sorting.
    pub remove_duplicates: bool,
}

/// Converts `text` to `mode`. The result may be longer or shorter (`ß` →
/// `SS`).
pub fn convert_case(text: &str, mode: CaseMode) -> String {
    match mode {
        CaseMode::Upper => text.to_uppercase(),
        CaseMode::Lower => text.to_lowercase(),
        CaseMode::Invert => text
            .chars()
            .flat_map(|c| -> Box<dyn Iterator<Item = char>> {
                if c.is_uppercase() {
                    Box::new(c.to_lowercase())
                } else if c.is_lowercase() {
                    Box::new(c.to_uppercase())
                } else {
                    Box::new(std::iter::once(c))
                }
            })
            .collect(),
        CaseMode::Title => {
            let mut out = String::with_capacity(text.len());
            let mut in_word = false;
            for c in text.chars() {
                if c.is_alphanumeric() {
                    if in_word {
                        out.extend(c.to_lowercase());
                    } else {
                        out.extend(c.to_uppercase());
                    }
                    in_word = true;
                } else {
                    out.push(c);
                    // "don't" stays one word.
                    in_word = in_word && (c == '\'' || c == '\u{2019}');
                }
            }
            out
        }
    }
}

fn line_range(doc: &Document, line: usize) -> Range<ByteOffset> {
    doc.line_start(line)..doc.line_end(line)
}

/// Lines a selection covers. A non-empty selection ending at the very start
/// of a line does not include that line (as in Notepad++).
fn lines_of(doc: &Document, s: &Selection) -> (usize, usize) {
    let first = doc.line_of(s.start());
    let mut last = doc.line_of(s.end());
    if !s.is_empty() && last > first && s.end() == doc.line_start(last) {
        last -= 1;
    }
    (first, last)
}

/// Sorts and merges overlapping or touching ranges.
fn merge(mut ranges: Vec<Range<ByteOffset>>) -> Vec<Range<ByteOffset>> {
    ranges.sort_by_key(|r| (r.start, r.end));
    let mut out: Vec<Range<ByteOffset>> = Vec::with_capacity(ranges.len());
    for r in ranges {
        match out.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => out.push(r),
        }
    }
    out
}

fn edit_result(changes: ChangeSet, after: SelectionSet, sels: &SelectionSet) -> Resolved {
    if changes.is_empty() {
        Resolved::Selections(sels.clone())
    } else {
        Resolved::Edit {
            changes,
            after,
            kind: EditKind::Other,
        }
    }
}

/// Maps every selection through `changes` (anchor and head both `Before`,
/// so insertions at a caret land after it).
fn map_before(sels: &SelectionSet, changes: &ChangeSet) -> SelectionSet {
    sels.map(|s| Selection {
        anchor: changes.map_position(s.anchor, Bias::Before),
        head: changes.map_position(s.head, Bias::Before),
        goal: None,
    })
}

/// Converts each selection, or the caret's line for an empty selection.
pub(crate) fn case(doc: &Document, sels: &SelectionSet, mode: CaseMode) -> Resolved {
    let targets = sels
        .ranges()
        .iter()
        .map(|s| {
            if s.is_empty() {
                line_range(doc, doc.line_of(s.head))
            } else {
                s.range()
            }
        })
        .collect();
    let pieces: Vec<(Range<ByteOffset>, String, String)> = merge(targets)
        .into_iter()
        .map(|r| {
            let old = doc.slice(r.clone());
            let new = convert_case(&old, mode);
            (r, old, new)
        })
        .collect();
    let changes = ChangeSet::new(
        doc.len(),
        pieces
            .iter()
            .filter(|(_, old, new)| old != new)
            .map(|(r, _, new)| Edit::new(r.start, r.end, new.as_str()))
            .collect(),
    );
    // Range ends map to the ends of the converted text, so selections cover
    // it even when its length changed; a caret inside keeps its char index.
    let remap = |pos: ByteOffset| {
        let Some((r, old, new)) = pieces.iter().find(|(r, ..)| r.start <= pos && pos <= r.end)
        else {
            return changes.map_position(pos, Bias::Before);
        };
        let new_start = changes.map_position(r.start, Bias::Before);
        if pos == r.end {
            return ByteOffset(new_start.0 + new.len());
        }
        let chars = old[..pos.0 - r.start.0].chars().count();
        let off = new.char_indices().nth(chars).map_or(new.len(), |(i, _)| i);
        ByteOffset(new_start.0 + off)
    };
    let after = sels
        .map(|s| Selection::new(remap(s.anchor), remap(s.head)))
        .with_rect(sels.rect().cloned());
    edit_result(changes, after, sels)
}

/// The number at the start of `s` (after leading whitespace).
fn leading_number(s: &str) -> Option<f64> {
    let t = s.trim_start();
    let b = t.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
        i += 1;
    }
    let digits_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let mut digits = i - digits_start;
    if i < b.len() && b[i] == b'.' {
        let frac_start = i + 1;
        let mut j = frac_start;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > frac_start {
            digits += j - frac_start;
            i = j;
        }
    }
    if digits == 0 {
        return None;
    }
    t[..i].parse().ok()
}

/// Sorts the selected lines, the rectangle's lines keyed by the text inside
/// it, or the whole document when nothing is selected.
pub(crate) fn sort(
    doc: &Document,
    sels: &SelectionSet,
    o: SortOptions,
    tab_width: usize,
) -> Resolved {
    let rect = sels.rect().cloned();
    let any_selected = sels.ranges().iter().any(|s| !s.is_empty());
    let (first, mut last) = if let Some(r) = &rect {
        (r.top(), r.bottom())
    } else if any_selected {
        let spans: Vec<_> = sels.ranges().iter().map(|s| lines_of(doc, s)).collect();
        let first = spans.iter().map(|s| s.0).min().unwrap_or(0);
        let last = spans.iter().map(|s| s.1).max().unwrap_or(0);
        (first, last)
    } else {
        (0, doc.line_count() - 1)
    };
    // A trailing newline's empty last line is not a line to sort.
    if last > first && last + 1 == doc.line_count() && doc.line_text(last).is_empty() {
        last -= 1;
    }
    if last <= first {
        return Resolved::Selections(sels.clone());
    }
    let lines: Vec<String> = (first..=last).map(|l| doc.line_text(l)).collect();
    let keys: Vec<String> = (first..=last)
        .zip(&lines)
        .map(|(line, text)| {
            let key = match &rect {
                Some(r) => {
                    let start = doc.line_start(line).0;
                    let right = visual_col_to_byte(doc, line, r.right(), tab_width).0 - start;
                    let left = (visual_col_to_byte_ceil(doc, line, r.left(), tab_width).0 - start)
                        .min(right);
                    text[left..right].to_owned()
                }
                None => text.clone(),
            };
            if o.case_insensitive {
                key.to_lowercase()
            } else {
                key
            }
        })
        .collect();
    let compare = |a: usize, b: usize| -> Ordering {
        let ord = if o.numeric {
            match (leading_number(&keys[a]), leading_number(&keys[b])) {
                (Some(x), Some(y)) => x.total_cmp(&y),
                (None, Some(_)) => Ordering::Less,
                (Some(_), None) => Ordering::Greater,
                (None, None) => keys[a].cmp(&keys[b]),
            }
        } else {
            keys[a].cmp(&keys[b])
        };
        if o.descending { ord.reverse() } else { ord }
    };
    let mut order: Vec<usize> = (0..lines.len()).collect();
    order.sort_by(|&a, &b| compare(a, b)); // stable
    if o.remove_duplicates {
        let same = |a: &str, b: &str| {
            if o.case_insensitive {
                a.to_lowercase() == b.to_lowercase()
            } else {
                a == b
            }
        };
        order.dedup_by(|b, a| same(&lines[*a], &lines[*b]));
    }
    let new: Vec<&str> = order.iter().map(|&i| lines[i].as_str()).collect();
    let new = new.join("\n");
    let start = doc.line_start(first);
    let end = doc.line_end(last);
    let changes = ChangeSet::new(doc.len(), vec![Edit::new(start, end, new.as_str())]);
    let after = if any_selected || rect.is_some() {
        SelectionSet::single(Selection::new(start, ByteOffset(start.0 + new.len())))
    } else {
        SelectionSet::caret(start)
    };
    edit_result(changes, after, sels)
}

/// The lines touched by non-empty selections, or every line if none.
fn selected_lines(doc: &Document, sels: &SelectionSet) -> BTreeSet<usize> {
    let selected: BTreeSet<usize> = sels
        .ranges()
        .iter()
        .filter(|s| !s.is_empty())
        .flat_map(|s| {
            let (a, b) = lines_of(doc, s);
            a..=b
        })
        .collect();
    if selected.is_empty() {
        (0..doc.line_count()).collect()
    } else {
        selected
    }
}

/// Removes spaces and tabs at the ends of the selected lines (all lines
/// when nothing is selected).
pub(crate) fn trim_trailing(doc: &Document, sels: &SelectionSet) -> Resolved {
    let edits = selected_lines(doc, sels)
        .into_iter()
        .filter_map(|line| {
            let text = doc.line_text(line);
            let keep = text.trim_end_matches([' ', '\t']).len();
            (keep < text.len()).then(|| {
                let start = doc.line_start(line).0;
                Edit::new(ByteOffset(start + keep), doc.line_end(line), "")
            })
        })
        .collect();
    let changes = ChangeSet::new(doc.len(), edits);
    let after = map_before(sels, &changes);
    edit_result(changes, after, sels)
}

/// Duplicates the caret's line below itself, or a selection right after it.
/// The carets and selections stay on the originals.
pub(crate) fn duplicate(doc: &Document, sels: &SelectionSet) -> Resolved {
    let mut lines = BTreeSet::new();
    let mut edits = Vec::new();
    for s in sels.ranges() {
        if s.is_empty() {
            lines.insert(doc.line_of(s.head));
        } else {
            edits.push(Edit::new(s.end(), s.end(), doc.slice(s.range())));
        }
    }
    for line in lines {
        let end = doc.line_end(line);
        edits.push(Edit::new(end, end, format!("\n{}", doc.line_text(line))));
    }
    let changes = ChangeSet::new(doc.len(), edits);
    let after = map_before(sels, &changes);
    edit_result(changes, after, sels)
}

/// Moves the block of lines covered by the selections up or down one line;
/// the selections move with it.
pub(crate) fn move_lines(doc: &Document, sels: &SelectionSet, up: bool) -> Resolved {
    let spans: Vec<_> = sels.ranges().iter().map(|s| lines_of(doc, s)).collect();
    let first = spans.iter().map(|s| s.0).min().unwrap_or(0);
    let last = spans.iter().map(|s| s.1).max().unwrap_or(0);
    let block = doc.slice(doc.line_start(first)..doc.line_end(last));
    let (edit, shift): (Edit, isize) = if up {
        if first == 0 {
            return Resolved::Selections(sels.clone());
        }
        let prev = doc.line_text(first - 1);
        (
            Edit::new(
                doc.line_start(first - 1),
                doc.line_end(last),
                format!("{block}\n{prev}"),
            ),
            -(prev.len() as isize + 1),
        )
    } else {
        if last + 1 >= doc.line_count() {
            return Resolved::Selections(sels.clone());
        }
        let next = doc.line_text(last + 1);
        (
            Edit::new(
                doc.line_start(first),
                doc.line_end(last + 1),
                format!("{next}\n{block}"),
            ),
            next.len() as isize + 1,
        )
    };
    let changes = ChangeSet::new(doc.len(), vec![edit]);
    // Every selection endpoint lies in the moved block (or at the start of
    // the line after it), so all of them shift by the same amount. Moving
    // down past the last line, "the start of the line after the block" no
    // longer exists (the block now ends the document): clamp to the end.
    let new_len = changes.new_len().0 as isize;
    let shift_pos = |p: ByteOffset| ByteOffset((p.0 as isize + shift).clamp(0, new_len) as usize);
    let after = sels.map(|s| Selection::new(shift_pos(s.anchor), shift_pos(s.head)));
    edit_result(changes, after, sels)
}

/// Joins the selected lines (or the caret's line with the next) into one,
/// replacing each line break and the whitespace around it with one space.
pub(crate) fn join(doc: &Document, sels: &SelectionSet) -> Resolved {
    let last_line = doc.line_count() - 1;
    let mut breaks = BTreeSet::new();
    for s in sels.ranges() {
        let (a, b) = lines_of(doc, s);
        let b = if a == b { (a + 1).min(last_line) } else { b };
        breaks.extend(a..b);
    }
    let edits = breaks
        .into_iter()
        .map(|line| {
            let text = doc.line_text(line);
            let next = doc.line_text(line + 1);
            let kept = text.trim_end_matches([' ', '\t']);
            let rest = next.trim_start_matches([' ', '\t']);
            let start = ByteOffset(doc.line_start(line).0 + kept.len());
            let end = ByteOffset(doc.line_start(line + 1).0 + (next.len() - rest.len()));
            let sep = if kept.is_empty() || rest.is_empty() {
                ""
            } else {
                " "
            };
            Edit::new(start, end, sep)
        })
        .collect();
    let changes = ChangeSet::new(doc.len(), edits);
    let after = map_before(sels, &changes);
    edit_result(changes, after, sels)
}

/// The lines a removing line operation looks at: the rectangle's lines, the
/// lines touched by non-empty selections, or every line. A trailing
/// newline's empty last line is not a line here (as in `sort`).
fn target_lines(doc: &Document, sels: &SelectionSet) -> Vec<usize> {
    let mut lines: Vec<usize> = match sels.rect() {
        Some(r) => (r.top()..=r.bottom()).collect(),
        None => selected_lines(doc, sels).into_iter().collect(),
    };
    let last = doc.line_count() - 1;
    if last > 0 && lines.last() == Some(&last) && doc.line_text(last).is_empty() {
        lines.pop();
    }
    lines
}

/// Deletes whole lines (sorted, distinct) with their line breaks. A run of
/// lines that ends the document takes the line break before it instead, so
/// no empty line is left behind.
fn remove_lines(doc: &Document, sels: &SelectionSet, lines: &[usize]) -> Resolved {
    let last = doc.line_count() - 1;
    let mut edits = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let first = lines[i];
        while i + 1 < lines.len() && lines[i + 1] == lines[i] + 1 {
            i += 1;
        }
        let end = lines[i];
        i += 1;
        let range = if end < last {
            doc.line_start(first)..doc.line_start(end + 1)
        } else if first > 0 {
            doc.line_end(first - 1)..doc.len()
        } else {
            ByteOffset(0)..doc.len()
        };
        if range.start < range.end {
            edits.push(Edit::new(range.start, range.end, ""));
        }
    }
    let changes = ChangeSet::new(doc.len(), edits);
    let after = map_before(sels, &changes);
    edit_result(changes, after, sels)
}

/// Removes the empty lines among the target lines; with
/// `include_whitespace`, also lines of only spaces and tabs.
pub(crate) fn remove_empty(
    doc: &Document,
    sels: &SelectionSet,
    include_whitespace: bool,
) -> Resolved {
    let lines: Vec<usize> = target_lines(doc, sels)
        .into_iter()
        .filter(|&line| {
            let text = doc.line_text(line);
            if include_whitespace {
                text.trim_matches([' ', '\t']).is_empty()
            } else {
                text.is_empty()
            }
        })
        .collect();
    remove_lines(doc, sels, &lines)
}

/// Removes target lines equal to an earlier target line, keeping the first
/// of each and the order; with `consecutive_only`, only lines equal to the
/// line right before them.
pub(crate) fn remove_duplicates(
    doc: &Document,
    sels: &SelectionSet,
    consecutive_only: bool,
) -> Resolved {
    let mut seen = HashSet::new();
    let mut prev: Option<(usize, String)> = None;
    let mut lines = Vec::new();
    for line in target_lines(doc, sels) {
        let text = doc.line_text(line);
        let duplicate = if consecutive_only {
            prev.as_ref()
                .is_some_and(|(l, t)| l + 1 == line && *t == text)
        } else {
            !seen.insert(text.clone())
        };
        if duplicate {
            lines.push(line);
        }
        prev = Some((line, text));
    }
    remove_lines(doc, sels, &lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_modes() {
        let s = "hello WORLD, don't straße";
        assert_eq!(
            convert_case(s, CaseMode::Upper),
            "HELLO WORLD, DON'T STRASSE"
        );
        assert_eq!(
            convert_case(s, CaseMode::Lower),
            "hello world, don't straße"
        );
        assert_eq!(
            convert_case(s, CaseMode::Title),
            "Hello World, Don't Straße"
        );
        assert_eq!(
            convert_case(s, CaseMode::Invert),
            "HELLO world, DON'T STRASSE"
        );
    }

    #[test]
    fn numbers_at_line_starts() {
        assert_eq!(leading_number("  42 apples"), Some(42.0));
        assert_eq!(leading_number("-3.5x"), Some(-3.5));
        assert_eq!(leading_number("7."), Some(7.0));
        assert_eq!(leading_number("x1"), None);
        assert_eq!(leading_number("-"), None);
    }
}
