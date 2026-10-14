//! Change history (Notepad++'s, against the saved file): the gutter marks
//! the lines that differ from the file as it was last loaded or saved, in
//! the compare view's colours: added lines green, changed ones blue, and a
//! red wedge where lines were deleted. The marks come from the compare
//! view's line diff, worked out on a thread once the text has been quiet for
//! a moment, so undoing back to the saved text clears them. Documents
//! without a file, read-only copies and documents over the highlighting
//! size limit get none.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use ochrepad_core::diff::{LineDiff, RowKind, diff_lines};
use ochrepad_core::{BufferId, Document, Editor};

/// Quiet time after an edit before the marks are worked out again.
const PAUSE_SECS: f64 = 0.15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Added,
    Changed,
}

/// The marks for one state of a document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineMarks {
    /// The lines that differ from the saved text, in order.
    pub lines: Vec<(usize, Mark)>,
    /// The lines with deleted lines just before them, in order; the line
    /// count stands for lines deleted at the end.
    pub deleted_before: Vec<usize>,
}

impl LineMarks {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.deleted_before.is_empty()
    }

    pub fn mark(&self, line: usize) -> Option<Mark> {
        self.lines
            .binary_search_by_key(&line, |m| m.0)
            .ok()
            .map(|i| self.lines[i].1)
    }

    /// Whether lines were deleted just before `line`.
    pub fn deleted_before(&self, line: usize) -> bool {
        self.deleted_before.binary_search(&line).is_ok()
    }

    /// The marks from a diff of the saved text (left) with the current one
    /// (right) of `lines` lines.
    pub fn from_diff(diff: &LineDiff, lines: usize) -> Self {
        let mut marks = LineMarks::default();
        let mut deleted = false;
        for row in &diff.rows {
            let Some(right) = row.right else {
                deleted |= row.kind == RowKind::Removed;
                continue;
            };
            if std::mem::take(&mut deleted) {
                marks.deleted_before.push(right);
            }
            match row.kind {
                RowKind::Added => marks.lines.push((right, Mark::Added)),
                RowKind::Changed => marks.lines.push((right, Mark::Changed)),
                RowKind::Equal | RowKind::Removed => {}
            }
        }
        if deleted {
            marks.deleted_before.push(lines);
        }
        marks
    }
}

/// What a document's marks compare with.
enum Base {
    /// The text as loaded or last saved.
    Text(Document),
    /// The file on disk, not read yet (a session brought back unsaved
    /// text).
    File(PathBuf),
    /// Nothing to compare with.
    None,
}

/// A diff running on a thread: (revision, the base it read from disk if it
/// did, the marks).
type Job = Receiver<(u64, Option<Option<Document>>, LineMarks)>;

struct Doc {
    base: Base,
    /// The clean revision `base` is the text of.
    base_revision: Option<u64>,
    marks: Arc<LineMarks>,
    marks_revision: Option<u64>,
    /// The revision last seen, and since when.
    seen: (u64, f64),
    job: Option<Job>,
}

#[derive(Default)]
pub struct ChangeHistory {
    docs: HashMap<BufferId, Doc>,
}

impl ChangeHistory {
    /// The marks to draw for `buffer`, if any.
    pub fn marks(&self, buffer: BufferId) -> Option<&LineMarks> {
        self.docs
            .get(&buffer)
            .map(|d| d.marks.as_ref())
            .filter(|m| !m.is_empty())
    }

    /// A save of `buffer` at `revision` wrote `doc`: the marks compare with
    /// that from now on.
    pub fn saved(&mut self, buffer: BufferId, doc: Document, revision: u64) {
        if let Some(d) = self.docs.get_mut(&buffer) {
            d.base = Base::Text(doc);
            d.base_revision = Some(revision);
            d.marks_revision = None;
            d.job = None;
        }
    }

    /// Per frame: follows each document's revision, and works out marks
    /// for documents that changed and went quiet. Documents over `limit`
    /// bytes get none. Returns whether work is pending.
    pub fn tick(&mut self, editor: &Editor, now: f64, limit: usize) -> bool {
        self.docs.retain(|b, _| editor.buffer(*b).is_some());
        let mut pending = false;
        for b in editor.buffers() {
            let revision = b.revision();
            let doc = self.docs.entry(b.id()).or_insert_with(|| Doc {
                base: match &b.meta.path {
                    Some(path) if b.is_dirty() => Base::File(path.clone()),
                    _ => Base::None,
                },
                base_revision: None,
                marks: Arc::default(),
                marks_revision: None,
                seen: (revision, now),
                job: None,
            });
            if !b.is_dirty() {
                // The text is the file's: it is what to compare with.
                if doc.base_revision != Some(revision) {
                    doc.base = Base::Text(b.doc().clone());
                    doc.base_revision = Some(revision);
                }
                doc.marks = Arc::default();
                doc.marks_revision = Some(revision);
                doc.job = None;
                continue;
            }
            let none = b.meta.path.is_none()
                || b.read_only
                || b.doc().len().0 > limit
                || matches!(doc.base, Base::None);
            if none {
                doc.marks = Arc::default();
                doc.job = None;
                continue;
            }
            if doc.seen.0 != revision {
                doc.seen = (revision, now);
            }
            if let Some(rx) = &doc.job
                && let Ok((rev, loaded, marks)) = rx.try_recv()
            {
                doc.job = None;
                match loaded {
                    Some(Some(text)) => doc.base = Base::Text(text),
                    Some(None) => doc.base = Base::None,
                    None => {}
                }
                doc.marks = Arc::new(marks);
                doc.marks_revision = Some(rev);
            }
            if doc.marks_revision == Some(revision) {
                continue;
            }
            pending = true;
            if doc.job.is_some() || now - doc.seen.1 < PAUSE_SECS {
                continue;
            }
            let base = match &doc.base {
                Base::Text(d) => Ok(d.clone()),
                Base::File(p) => Err(p.clone()),
                Base::None => continue,
            };
            let current = b.doc().clone();
            let (tx, rx) = channel();
            let spawned = std::thread::Builder::new()
                .name("ochrepad-changes".into())
                .spawn(move || {
                    let (base, loaded) = match base {
                        Ok(d) => (Some(d), None),
                        Err(path) => {
                            let d = ochrepad_fileio::load(&path)
                                .ok()
                                .map(|d| Document::new(&d.text));
                            (d.clone(), Some(d))
                        }
                    };
                    let marks = base.map_or_else(LineMarks::default, |base| {
                        LineMarks::from_diff(&diff_lines(&base, &current), current.line_count())
                    });
                    let _ = tx.send((revision, loaded, marks));
                });
            if spawned.is_ok() {
                doc.job = Some(rx);
            }
        }
        pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marks(old: &str, new: &str) -> LineMarks {
        let (a, b) = (Document::new(old), Document::new(new));
        LineMarks::from_diff(&diff_lines(&a, &b), b.line_count())
    }

    #[test]
    fn added_changed_and_deleted_lines() {
        let m = marks("a\nb\nc\nd", "a\nB\nc\nnew\nd");
        assert_eq!(m.lines, vec![(1, Mark::Changed), (3, Mark::Added)]);
        assert!(m.deleted_before.is_empty());
        let m = marks("a\nb\nc\nd", "a\nd");
        assert!(m.lines.is_empty());
        assert_eq!(m.deleted_before, vec![1]);
        assert!(m.deleted_before(1) && !m.deleted_before(0));
        // Lines deleted at the end are marked after the last line.
        let m = marks("a\nb\nc", "a");
        assert_eq!(m.deleted_before, vec![1]);
        assert_eq!(m.mark(0), None);
        assert!(marks("same\ntext", "same\ntext").is_empty());
    }
}
