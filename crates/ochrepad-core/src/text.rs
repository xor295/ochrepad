//! `Document`: the rope, addressed only by [`ByteOffset`].
//!
//! Every Ropey call in the crate lives in this module, so a later move to Ropey 2
//! (byte-indexed) stays local. Ropey is built without `unicode_lines`/`cr_lines`,
//! so only `\n` ends a line; line endings are normalized by `fileio`.

use std::fmt;
use std::ops::Range;

use ropey::Rope;

use crate::position::{ByteOffset, CharIdx};

#[derive(Clone, Default)]
pub struct Document {
    rope: Rope,
}

impl Document {
    pub fn new(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
        }
    }

    /// The text in order, in the rope's pieces (for hashing without a copy).
    pub fn chunks(&self) -> impl Iterator<Item = &str> + '_ {
        self.rope.chunks()
    }

    /// Length in bytes; also the offset of the end of the document.
    pub fn len(&self) -> ByteOffset {
        ByteOffset(self.rope.len_bytes())
    }

    pub fn is_empty(&self) -> bool {
        self.rope.len_bytes() == 0
    }

    /// Number of lines. An empty document, or one ending in `\n`, still has a
    /// (possibly empty) last line.
    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    /// Line containing `pos`. A `\n` belongs to the line it ends.
    pub fn line_of(&self, pos: ByteOffset) -> usize {
        self.rope.byte_to_line(pos.0)
    }

    pub fn line_start(&self, line: usize) -> ByteOffset {
        ByteOffset(self.rope.line_to_byte(line))
    }

    /// End of the line's content, excluding its `\n`.
    pub fn line_end(&self, line: usize) -> ByteOffset {
        if line + 1 < self.line_count() {
            ByteOffset(self.rope.line_to_byte(line + 1) - 1)
        } else {
            self.len()
        }
    }

    /// The line's content, excluding its `\n`.
    pub fn line_text(&self, line: usize) -> String {
        self.slice(self.line_start(line)..self.line_end(line))
    }

    pub fn slice(&self, range: Range<ByteOffset>) -> String {
        self.rope.byte_slice(range.start.0..range.end.0).to_string()
    }

    pub fn is_char_boundary(&self, pos: ByteOffset) -> bool {
        if pos.0 > self.rope.len_bytes() {
            return false;
        }
        let (chunk, chunk_start, _, _) = self.rope.chunk_at_byte(pos.0);
        chunk.is_char_boundary(pos.0 - chunk_start)
    }

    /// The chunk of text containing `pos` and the offset at which it starts.
    /// At the end of the document this is the last chunk. Grapheme segmentation
    /// in `position` walks the rope with this.
    pub fn chunk_at(&self, pos: ByteOffset) -> (&str, ByteOffset) {
        let (chunk, start, _, _) = self.rope.chunk_at_byte(pos.0);
        (chunk, ByteOffset(start))
    }

    /// The char starting at `pos`, if any.
    pub fn char_at(&self, pos: ByteOffset) -> Option<char> {
        if pos >= self.len() {
            return None;
        }
        let (chunk, start, _, _) = self.rope.chunk_at_byte(pos.0);
        chunk[pos.0 - start..].chars().next()
    }

    pub fn to_char_idx(&self, pos: ByteOffset) -> CharIdx {
        CharIdx(self.rope.byte_to_char(pos.0))
    }

    /// The one replacement that turns `self` into `new`: the range of `self`
    /// that differs and the range of `new` that took its place, both on
    /// char boundaries (`None` when the texts are equal). Compares the
    /// ropes' pieces from both ends, so an edit in a large document costs
    /// little.
    pub fn changed_range(&self, new: &Document) -> Option<(Range<ByteOffset>, Range<ByteOffset>)> {
        let (old_len, new_len) = (self.rope.len_bytes(), new.rope.len_bytes());
        let prefix = common_run(self.rope.chunks(), new.rope.chunks(), false);
        if prefix == old_len && prefix == new_len {
            return None;
        }
        fn back(r: &Rope) -> ropey::iter::Chunks<'_> {
            r.chunks_at_byte(r.len_bytes()).0.reversed()
        }
        let most = old_len.min(new_len) - prefix;
        let suffix = common_run(back(&self.rope), back(&new.rope), true).min(most);
        // The same bytes up to `prefix` (and after the suffix) in both, so a
        // boundary in one is a boundary in the other.
        let mut start = prefix;
        while !self.is_char_boundary(ByteOffset(start)) {
            start -= 1;
        }
        let mut suffix = suffix;
        while !self.is_char_boundary(ByteOffset(old_len - suffix)) {
            suffix -= 1;
        }
        Some((
            ByteOffset(start)..ByteOffset(old_len - suffix),
            ByteOffset(start)..ByteOffset(new_len - suffix),
        ))
    }

    /// Replaces `range` with `text`. Both ends must be char boundaries.
    pub fn replace(&mut self, range: Range<ByteOffset>, text: &str) {
        debug_assert!(self.is_char_boundary(range.start) && self.is_char_boundary(range.end));
        let start = self.to_char_idx(range.start);
        if range.start < range.end {
            let end = self.to_char_idx(range.end);
            self.rope.remove(start.0..end.0);
        }
        if !text.is_empty() {
            self.rope.insert(start.0, text);
        }
    }
}

/// How many bytes two texts, given as pieces, have in common from their
/// start (or, with `backwards`, from their end, the pieces coming last
/// first).
fn common_run<'a>(
    mut a: impl Iterator<Item = &'a str>,
    mut b: impl Iterator<Item = &'a str>,
    backwards: bool,
) -> usize {
    let (mut x, mut y): (&[u8], &[u8]) = (&[], &[]);
    let mut n = 0;
    loop {
        while x.is_empty() {
            match a.next() {
                Some(c) => x = c.as_bytes(),
                None => return n,
            }
        }
        while y.is_empty() {
            match b.next() {
                Some(c) => y = c.as_bytes(),
                None => return n,
            }
        }
        let k = x.len().min(y.len());
        let (px, py) = if backwards {
            (&x[x.len() - k..], &y[y.len() - k..])
        } else {
            (&x[..k], &y[..k])
        };
        // Whole pieces are usually equal (a quick compare); only the one
        // that differs is looked at byte by byte.
        let same = if px == py {
            k
        } else if backwards {
            px.iter()
                .rev()
                .zip(py.iter().rev())
                .take_while(|(p, q)| p == q)
                .count()
        } else {
            px.iter().zip(py).take_while(|(p, q)| p == q).count()
        };
        n += same;
        if same < k {
            return n;
        }
        if backwards {
            x = &x[..x.len() - k];
            y = &y[..y.len() - k];
        } else {
            x = &x[k..];
            y = &y[k..];
        }
    }
}

impl fmt::Display for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for chunk in self.rope.chunks() {
            f.write_str(chunk)?;
        }
        Ok(())
    }
}

impl fmt::Debug for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Document({:?})", self.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_split_only_on_lf() {
        let d = Document::new("a\r\nb\rc\u{2028}d\n");
        assert_eq!(d.line_count(), 3);
        assert_eq!(d.line_text(0), "a\r");
        assert_eq!(d.line_text(1), "b\rc\u{2028}d");
        assert_eq!(d.line_text(2), "");
        assert_eq!(d.line_end(0), ByteOffset(2));
    }

    #[test]
    fn replace_by_bytes() {
        let mut d = Document::new("héllo wörld");
        d.replace(ByteOffset(1)..ByteOffset(3), "e");
        assert_eq!(d.to_string(), "hello wörld");
        d.replace(ByteOffset(5)..ByteOffset(5), ",");
        assert_eq!(d.to_string(), "hello, wörld");
        assert!(!d.is_char_boundary(ByteOffset(9)));
    }
}
