//! Bookmarks: marked lines, as in Notepad++ (Search > Bookmark). They
//! belong to the document, so every view of it shows the same ones, and
//! changing them is not an undo step. They follow edits the way Scintilla's
//! line markers do: text inserted at the start of a marked line pushes the
//! mark down with the line's text, a mark whose line break before it is
//! deleted joins the line before, and the marks in a block of replaced
//! lines stay on the same lines as far as the new text reaches (sorting
//! lines keeps marks where they were).

use crate::action::{Action, ActionError};
use crate::changeset::{ChangeSet, Edit};
use crate::history::EditKind;
use crate::resolve::Resolved;
use crate::selection::SelectionSet;
use crate::text::Document;

/// The marked lines of one document, sorted and without repeats.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bookmarks {
    lines: Vec<usize>,
}

/// How one edit of a change set changed the lines, in the coordinates of
/// the document before the change: lines `start..=old_end` became
/// `start..=start + new_lines`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct EditLines {
    pub start: usize,
    pub old_end: usize,
    /// Line breaks in the new text.
    pub new_lines: usize,
    /// The edit begins at the start of line `start`.
    pub at_line_start: bool,
    /// The edit only inserts.
    pub insertion: bool,
}

impl Bookmarks {
    /// The marked lines, in order.
    pub fn lines(&self) -> &[usize] {
        &self.lines
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn contains(&self, line: usize) -> bool {
        self.lines.binary_search(&line).is_ok()
    }

    /// Replaces the marks with `lines`; lines past the last of `line_count`
    /// are dropped.
    pub fn set(&mut self, lines: impl IntoIterator<Item = usize>, line_count: usize) {
        let mut lines: Vec<usize> = lines.into_iter().filter(|&l| l < line_count).collect();
        lines.sort_unstable();
        lines.dedup();
        self.lines = lines;
    }

    /// Marks `line`; returns whether it was unmarked.
    pub fn insert(&mut self, line: usize) -> bool {
        match self.lines.binary_search(&line) {
            Ok(_) => false,
            Err(i) => {
                self.lines.insert(i, line);
                true
            }
        }
    }

    /// Unmarks `line`; returns whether it was marked.
    pub fn remove(&mut self, line: usize) -> bool {
        match self.lines.binary_search(&line) {
            Ok(i) => {
                self.lines.remove(i);
                true
            }
            Err(_) => false,
        }
    }

    /// Marks `line` if it is unmarked, else unmarks it; returns whether it
    /// is marked now.
    pub fn toggle(&mut self, line: usize) -> bool {
        !self.remove(line) && self.insert(line)
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }

    /// The first marked line after `line`, else the first one (wrapping
    /// around, as F2 does).
    pub fn next_after(&self, line: usize) -> Option<usize> {
        let i = self.lines.partition_point(|&l| l <= line);
        self.lines.get(i).or(self.lines.first()).copied()
    }

    /// The last marked line before `line`, else the last one.
    pub fn prev_before(&self, line: usize) -> Option<usize> {
        let i = self.lines.partition_point(|&l| l < line);
        i.checked_sub(1)
            .and_then(|i| self.lines.get(i))
            .or(self.lines.last())
            .copied()
    }

    /// Marks every unmarked line of a document of `line_count` lines and
    /// unmarks every marked one.
    pub fn invert(&mut self, line_count: usize) {
        let mut marked = self.lines.iter().copied().peekable();
        let mut inverted = Vec::with_capacity(line_count.saturating_sub(self.lines.len()));
        for line in 0..line_count {
            if marked.peek() == Some(&line) {
                marked.next();
            } else {
                inverted.push(line);
            }
        }
        self.lines = inverted;
    }

    /// Moves the marks through a change set's `edits` (sorted, in the old
    /// document's lines); `line_count` is the new document's.
    pub(crate) fn map(&mut self, edits: &[EditLines], line_count: usize) {
        if self.lines.is_empty() || edits.is_empty() {
            return;
        }
        let mut out = Vec::with_capacity(self.lines.len());
        // Lines gained by the edits wholly before the current mark.
        let mut delta: isize = 0;
        let mut i = 0;
        for &line in &self.lines {
            while let Some(e) = edits.get(i).filter(|e| e.old_end < line) {
                delta += e.new_lines as isize - (e.old_end - e.start) as isize;
                i += 1;
            }
            let new = match edits.get(i) {
                // Inside a replaced block: the same line of the new text, or
                // its last line when the new text is shorter.
                Some(e) if e.start < line => {
                    e.start as isize + delta + (line - e.start).min(e.new_lines) as isize
                }
                // Text inserted at the start of the line goes before it.
                Some(e) if e.start == line && e.at_line_start && e.insertion => {
                    (line + e.new_lines) as isize + delta
                }
                _ => line as isize + delta,
            };
            out.push(new.clamp(0, line_count.saturating_sub(1) as isize) as usize);
        }
        out.sort_unstable();
        out.dedup();
        self.lines = out;
    }
}

/// The marked lines' text, each with its line break (the last line of the
/// document has none): what Copy Bookmarked Lines puts on the clipboard.
pub fn marked_text(doc: &Document, marks: &Bookmarks) -> String {
    let last = doc.line_count() - 1;
    let mut text = String::new();
    for &line in marks.lines() {
        text.push_str(&doc.line_text(line));
        if line < last {
            text.push('\n');
        }
    }
    text
}

/// The lines that hold a selection's head (a caret's line), in order.
pub(crate) fn caret_lines(doc: &Document, sels: &SelectionSet) -> Vec<usize> {
    let mut lines: Vec<usize> = sels.ranges().iter().map(|s| doc.line_of(s.head)).collect();
    lines.dedup();
    lines
}

/// Resolves the bookmark actions that move the caret or edit the text.
pub(crate) fn resolve(
    action: &Action,
    doc: &Document,
    sels: &SelectionSet,
    marks: &Bookmarks,
) -> Result<Resolved, ActionError> {
    if marks.is_empty() {
        return Err(ActionError::NoBookmarks);
    }
    let caret = doc.line_of(sels.primary().head);
    let go = |line: Option<usize>| {
        let line = line.ok_or(ActionError::NoBookmarks)?;
        Ok(Resolved::Selections(SelectionSet::caret(
            doc.line_start(line),
        )))
    };
    match action {
        Action::NextBookmark => go(marks.next_after(caret)),
        Action::PrevBookmark => go(marks.prev_before(caret)),
        Action::RemoveBookmarkedLines => {
            Ok(crate::transform::remove_lines(doc, sels, marks.lines()))
        }
        Action::RemoveUnbookmarkedLines => {
            let mut unmarked = marks.clone();
            unmarked.invert(doc.line_count());
            Ok(crate::transform::remove_lines(doc, sels, unmarked.lines()))
        }
        Action::ReplaceBookmarkedLines(text) => {
            let edits: Vec<Edit> = marks
                .lines()
                .iter()
                .map(|&l| Edit::new(doc.line_start(l), doc.line_end(l), text.as_str()))
                .collect();
            let changes = ChangeSet::new(doc.len(), edits);
            let after = SelectionSet::caret(
                changes.map_position(sels.primary().head, crate::changeset::Bias::Before),
            );
            Ok(Resolved::Edit {
                changes,
                after,
                kind: EditKind::Other,
            })
        }
        _ => Ok(Resolved::Selections(sels.clone())),
    }
}

/// The marks after Paste to (Replace) Bookmarked Lines put `text` on each
/// of `before`: the first line of each replacement.
pub(crate) fn after_replace(before: &[usize], text: &str) -> Vec<usize> {
    let breaks = text.bytes().filter(|&b| b == b'\n').count();
    before
        .iter()
        .enumerate()
        .map(|(i, &l)| l + i * breaks)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marks(lines: &[usize]) -> Bookmarks {
        let mut b = Bookmarks::default();
        b.set(lines.iter().copied(), usize::MAX);
        b
    }

    #[test]
    fn toggle_insert_remove_and_set() {
        let mut b = Bookmarks::default();
        assert!(b.toggle(3));
        assert!(b.toggle(1));
        assert_eq!(b.lines(), &[1, 3]);
        assert!(!b.toggle(3));
        assert_eq!(b.lines(), &[1]);
        assert!(!b.insert(1));
        assert!(b.remove(1));
        assert!(b.is_empty());
        b.set([5, 2, 2, 9], 6);
        assert_eq!(b.lines(), &[2, 5]);
    }

    #[test]
    fn next_and_previous_wrap_around() {
        let b = marks(&[2, 5, 9]);
        assert_eq!(b.next_after(0), Some(2));
        assert_eq!(b.next_after(2), Some(5));
        assert_eq!(b.next_after(9), Some(2));
        assert_eq!(b.prev_before(5), Some(2));
        assert_eq!(b.prev_before(2), Some(9));
        assert_eq!(b.prev_before(100), Some(9));
        assert_eq!(Bookmarks::default().next_after(0), None);
    }

    #[test]
    fn invert_marks_the_other_lines() {
        let mut b = marks(&[0, 2]);
        b.invert(5);
        assert_eq!(b.lines(), &[1, 3, 4]);
        b.invert(5);
        assert_eq!(b.lines(), &[0, 2]);
    }

    #[test]
    fn marked_text_keeps_line_breaks_except_at_the_end() {
        let doc = Document::new("a\nb\nc");
        assert_eq!(marked_text(&doc, &marks(&[0, 2])), "a\nc");
        assert_eq!(marked_text(&doc, &marks(&[1])), "b\n");
    }

    #[test]
    fn after_replace_counts_the_inserted_lines() {
        assert_eq!(after_replace(&[1, 4], "x\ny"), vec![1, 5]);
        assert_eq!(after_replace(&[1, 4], "x"), vec![1, 4]);
    }
}
