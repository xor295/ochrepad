//! A set of line numbers that grows at its end, kept as one bit per line,
//! for the viewer's filters (PLAN_v0.3.0 P8): a filter over a 40-million-
//! line log costs 5 MB whether it keeps a few lines or nearly all of them.
//! Counting the lines before a line (rank) and finding the n-th line in or
//! out of the set (select) use a count of the lines before every 512.

/// Words (of 64 lines) per block of the count.
const BLOCK: usize = 8;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineSet {
    words: Vec<u64>,
    /// The lines in the set before each block of [`BLOCK`] words.
    blocks: Vec<u64>,
    len: u64,
    last: Option<u64>,
}

impl LineSet {
    /// How many lines are in the set.
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Adds `line`. Lines come in order: one not after the last is ignored.
    pub fn push(&mut self, line: u64) {
        if self.last.is_some_and(|l| line <= l) {
            return;
        }
        let w = (line / 64) as usize;
        while self.words.len() <= w {
            if self.words.len().is_multiple_of(BLOCK) {
                self.blocks.push(self.len);
            }
            self.words.push(0);
        }
        self.words[w] |= 1 << (line % 64);
        self.len += 1;
        self.last = Some(line);
    }

    pub fn contains(&self, line: u64) -> bool {
        self.words
            .get((line / 64) as usize)
            .is_some_and(|w| w & (1 << (line % 64)) != 0)
    }

    /// How many lines of the set come before `line`.
    pub fn rank(&self, line: u64) -> u64 {
        let w = (line / 64) as usize;
        if w >= self.words.len() {
            return self.len;
        }
        let b = w / BLOCK;
        let before: u64 = self.words[b * BLOCK..w]
            .iter()
            .map(|x| u64::from(x.count_ones()))
            .sum();
        let below = self.words[w] & ((1u64 << (line % 64)) - 1);
        self.blocks[b] + before + u64::from(below.count_ones())
    }

    /// The `i`-th line of the set, from 0.
    pub fn select(&self, i: u64) -> Option<u64> {
        if i >= self.len {
            return None;
        }
        let b = self.blocks.partition_point(|&n| n <= i) - 1;
        let mut left = i - self.blocks[b];
        for w in b * BLOCK..((b + 1) * BLOCK).min(self.words.len()) {
            let ones = u64::from(self.words[w].count_ones());
            if left < ones {
                return Some(w as u64 * 64 + nth_one(self.words[w], left));
            }
            left -= ones;
        }
        None
    }

    /// The `i`-th line not in the set, from 0.
    pub fn select_out(&self, i: u64) -> u64 {
        let out_before = |b: usize| (b * BLOCK * 64) as u64 - self.blocks[b];
        // The last block with at most `i` lines out of the set before it.
        let (mut lo, mut hi) = (0, self.blocks.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if out_before(mid) <= i {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let Some(b) = lo.checked_sub(1) else {
            return i;
        };
        let mut left = i - out_before(b);
        for w in b * BLOCK..((b + 1) * BLOCK).min(self.words.len()) {
            let zeros = 64 - u64::from(self.words[w].count_ones());
            if left < zeros {
                return w as u64 * 64 + nth_one(!self.words[w], left);
            }
            left -= zeros;
        }
        // Past the last word nothing is in the set.
        self.words.len() as u64 * 64 + left
    }
}

/// The position of the `n`-th set bit of `word`, from 0 (it has one).
fn nth_one(mut word: u64, n: u64) -> u64 {
    for _ in 0..n {
        word &= word - 1;
    }
    u64::from(word.trailing_zeros())
}

impl Extend<u64> for LineSet {
    fn extend<I: IntoIterator<Item = u64>>(&mut self, lines: I) {
        for l in lines {
            self.push(l);
        }
    }
}

impl FromIterator<u64> for LineSet {
    fn from_iter<I: IntoIterator<Item = u64>>(lines: I) -> Self {
        let mut set = LineSet::default();
        set.extend(lines);
        set
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Checks every query against a plain list of the same lines.
    fn check(lines: &[u64]) {
        let set: LineSet = lines.iter().copied().collect();
        assert_eq!(set.len(), lines.len() as u64);
        let end = lines.last().map_or(0, |l| l + 1) + 700;
        for (i, &l) in lines.iter().enumerate() {
            assert_eq!(set.select(i as u64), Some(l));
        }
        assert_eq!(set.select(lines.len() as u64), None);
        let outside: Vec<u64> = (0..end).filter(|l| !lines.contains(l)).collect();
        for (i, &l) in outside.iter().enumerate() {
            assert_eq!(set.select_out(i as u64), l, "out {i}");
        }
        for line in 0..end {
            let below = lines.iter().filter(|&&l| l < line).count() as u64;
            assert_eq!(set.rank(line), below, "rank {line}");
            assert_eq!(set.contains(line), lines.contains(&line));
        }
    }

    #[test]
    fn rank_and_select_agree_with_a_list() {
        check(&[]);
        check(&[0]);
        check(&[3, 64, 65, 511, 512, 513, 1000, 1023, 1024, 4000]);
        check(&(0..1500).collect::<Vec<_>>());
        check(&(0..3000).filter(|l| l % 7 != 3).collect::<Vec<_>>());
        check(&(0..3000).filter(|l| l % 97 == 5).collect::<Vec<_>>());
    }

    #[test]
    fn lines_come_in_order() {
        let mut set: LineSet = [5, 9].into_iter().collect();
        set.push(9);
        set.push(2);
        assert_eq!(set.len(), 2);
        set.push(70_000);
        assert_eq!(set.select(2), Some(70_000));
        assert_eq!(set.rank(70_000), 2);
    }

    #[test]
    fn a_large_set_is_small() {
        // A bit and a little more per line, however many are in the set.
        let set: LineSet = (0..4_000_000u64).filter(|l| l % 10 != 0).collect();
        assert_eq!(set.len(), 3_600_000);
        let bytes = (set.words.len() + set.blocks.len()) * 8;
        assert!(bytes < 600 << 10, "{bytes}");
        assert_eq!(set.select_out(3), 30);
        assert_eq!(set.select(9), Some(11));
    }
}
