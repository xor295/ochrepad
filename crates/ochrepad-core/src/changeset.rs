//! `ChangeSet`: concrete, non-overlapping edits against one document state.

use serde::{Deserialize, Serialize};

use crate::position::ByteOffset;
use crate::text::Document;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edit {
    pub start: ByteOffset,
    pub end: ByteOffset,
    pub text: String,
}

impl Edit {
    pub fn new(start: ByteOffset, end: ByteOffset, text: impl Into<String>) -> Self {
        Self {
            start,
            end,
            text: text.into(),
        }
    }

    fn is_noop(&self) -> bool {
        self.start == self.end && self.text.is_empty()
    }
}

/// Which side of an insertion a position at the insertion point ends up on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bias {
    Before,
    After,
}

/// Sorted, non-overlapping, non-touching edits against a document of `base_len`
/// bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeSet {
    base_len: ByteOffset,
    edits: Vec<Edit>,
}

impl ChangeSet {
    /// Sorts the edits and merges touching ones (concatenating their text) and
    /// overlapping deletions. Overlapping edits that both insert text are a
    /// caller bug.
    pub fn new(base_len: ByteOffset, mut edits: Vec<Edit>) -> Self {
        edits.retain(|e| !e.is_noop());
        edits.sort_by_key(|e| (e.start, e.end));
        let mut out: Vec<Edit> = Vec::with_capacity(edits.len());
        for e in edits {
            debug_assert!(e.start <= e.end && e.end <= base_len);
            match out.last_mut() {
                Some(last) if e.start <= last.end => {
                    debug_assert!(
                        e.start == last.end || (last.text.is_empty() && e.text.is_empty()),
                        "overlapping edits that insert text: {last:?} {e:?}"
                    );
                    last.end = last.end.max(e.end);
                    last.text.push_str(&e.text);
                }
                _ => out.push(e),
            }
        }
        Self {
            base_len,
            edits: out,
        }
    }

    pub fn edits(&self) -> &[Edit] {
        &self.edits
    }

    /// Whether this applies to `doc`: its length matches, and the edits are
    /// in order, inside the text and on character boundaries. A change set
    /// read back from disk is checked with this before it is used.
    pub fn fits(&self, doc: &Document) -> bool {
        let char_boundary = |p: ByteOffset| p <= doc.len() && doc.is_char_boundary(p);
        self.base_len == doc.len()
            && self
                .edits
                .iter()
                .all(|e| e.start <= e.end && char_boundary(e.start) && char_boundary(e.end))
            && self.edits.windows(2).all(|w| w[0].end <= w[1].start)
    }

    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    pub fn base_len(&self) -> ByteOffset {
        self.base_len
    }

    pub fn new_len(&self) -> ByteOffset {
        let delta: isize = self
            .edits
            .iter()
            .map(|e| e.text.len() as isize - (e.end.0 - e.start.0) as isize)
            .sum();
        ByteOffset((self.base_len.0 as isize + delta) as usize)
    }

    /// Applies the edits to `doc`, which must be in the base state.
    pub fn apply(&self, doc: &mut Document) {
        debug_assert_eq!(doc.len(), self.base_len);
        for e in self.edits.iter().rev() {
            doc.replace(e.start..e.end, &e.text);
        }
    }

    /// The change set that undoes this one. `doc` must be in the base state.
    pub fn invert(&self, doc: &Document) -> ChangeSet {
        let mut delta: isize = 0;
        let mut edits = Vec::with_capacity(self.edits.len());
        for e in &self.edits {
            let start = ByteOffset((e.start.0 as isize + delta) as usize);
            edits.push(Edit::new(
                start,
                ByteOffset(start.0 + e.text.len()),
                doc.slice(e.start..e.end),
            ));
            delta += e.text.len() as isize - (e.end.0 - e.start.0) as isize;
        }
        ChangeSet {
            base_len: self.new_len(),
            edits,
        }
    }

    /// Maps a position in the base state to the new state.
    ///
    /// Positions inside a replaced range move to its start (`Before`) or to the
    /// end of the replacement (`After`). A position at the start of a non-empty
    /// replaced range stays before it and one at its end stays after it,
    /// whatever the bias; the bias only decides ties at pure insertions and
    /// positions strictly inside a replaced range.
    pub fn map_position(&self, pos: ByteOffset, bias: Bias) -> ByteOffset {
        let mut delta: isize = 0;
        for e in &self.edits {
            let new_start = (e.start.0 as isize + delta) as usize;
            let new_end = new_start + e.text.len();
            if pos < e.start {
                break;
            }
            if pos <= e.end {
                let after = if e.start == e.end {
                    bias == Bias::After
                } else if pos == e.start {
                    false
                } else if pos == e.end {
                    true
                } else {
                    bias == Bias::After
                };
                return ByteOffset(if after { new_end } else { new_start });
            }
            delta += e.text.len() as isize - (e.end.0 - e.start.0) as isize;
        }
        ByteOffset((pos.0 as isize + delta) as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(n: usize) -> ByteOffset {
        ByteOffset(n)
    }

    #[test]
    fn apply_and_invert() {
        let mut doc = Document::new("hello world");
        let cs = ChangeSet::new(
            doc.len(),
            vec![Edit::new(b(6), b(11), "there"), Edit::new(b(0), b(5), "hi")],
        );
        let inv = cs.invert(&doc);
        cs.apply(&mut doc);
        assert_eq!(doc.to_string(), "hi there");
        assert_eq!(cs.new_len(), doc.len());
        inv.apply(&mut doc);
        assert_eq!(doc.to_string(), "hello world");
    }

    #[test]
    fn merges_touching_and_overlapping_deletes() {
        let cs = ChangeSet::new(
            b(10),
            vec![
                Edit::new(b(2), b(3), ""),
                Edit::new(b(3), b(4), ""),
                Edit::new(b(3), b(5), ""),
                Edit::new(b(7), b(7), ""),
            ],
        );
        assert_eq!(cs.edits(), &[Edit::new(b(2), b(5), "")]);
    }

    #[test]
    fn map_position_bias() {
        // Insert "XY" at 2, replace 5..7 with "Z".
        let cs = ChangeSet::new(
            b(10),
            vec![Edit::new(b(2), b(2), "XY"), Edit::new(b(5), b(7), "Z")],
        );
        let m = |p, bias| cs.map_position(b(p), bias).0;
        assert_eq!(m(1, Bias::After), 1);
        assert_eq!(m(2, Bias::Before), 2);
        assert_eq!(m(2, Bias::After), 4);
        assert_eq!(m(4, Bias::Before), 6);
        assert_eq!(m(5, Bias::After), 7); // start of replaced range stays before
        assert_eq!(m(6, Bias::Before), 7);
        assert_eq!(m(6, Bias::After), 8);
        assert_eq!(m(7, Bias::Before), 8); // end of replaced range stays after
        assert_eq!(m(10, Bias::Before), 11);
    }
}
