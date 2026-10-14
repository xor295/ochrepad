//! Noticing when an open file changes on disk (PLAN M13): a `notify` watcher
//! on the directories of open files; each buffer remembers the modified time
//! and size of its file when it was loaded or saved. A change by another
//! program asks to reload (a clean buffer) or warns (a dirty one); a deleted
//! file asks whether to keep it open.
//!
//! The same watcher covers the file tree's root and expanded folders, so the
//! tree relists a folder when something in it changes.
//!
//! Follow mode (View > Follow, PLAN_v0.3.0 P6) is `tail -f`: when a
//! followed file grows, the new text is appended to its clean document
//! without asking (and without an undo step), after checking that the end
//! it had is still there; a file that shrank or changed before its end is
//! reloaded. Views whose caret was at the end keep showing it.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::SystemTime;

use egui::{Id, Modal, Ui};
use notify::{RecursiveMode, Watcher};
use ochrepad_core::BufferId;
use ochrepad_fileio as fileio;

use super::{OchrepadApp, canonical};

/// Wait this long after the last event before looking (writers often
/// touch a file several times).
const SETTLE_SECS: f64 = 0.3;
/// How soon a followed file is looked at after it changes. Unlike other
/// files, more changes do not postpone it (a log may never pause).
const FOLLOW_SECS: f64 = 0.1;
/// Bytes of a followed file's end that must still be there for a change to
/// count as an append.
const FOLLOW_TAIL: u64 = 4096;
/// More text than this appended at once is read by reloading the file.
const FOLLOW_MAX_APPEND: u64 = 64 * 1024 * 1024;

type Stamp = (SystemTime, u64);

/// `canonical`, but also for a file that no longer exists (its directory
/// is canonicalized), so that a removal event matches the open file.
fn watch_key(path: &Path) -> PathBuf {
    match (path.parent(), path.file_name()) {
        (Some(dir), Some(name)) => canonical(dir).join(name),
        _ => canonical(path),
    }
}

fn stamp(path: &Path) -> Option<Stamp> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskChange {
    Modified,
    Deleted,
}

#[derive(Default)]
pub(super) struct External {
    watcher: Option<notify::RecommendedWatcher>,
    rx: Option<Receiver<notify::Result<notify::Event>>>,
    watched: HashSet<PathBuf>,
    /// The tree's watched folders as (canonical, as the tree knows them),
    /// recomputed when the tree's folders change.
    tree_dirs: Vec<(PathBuf, PathBuf)>,
    tree_dirs_for: Vec<PathBuf>,
    stamps: HashMap<BufferId, Option<Stamp>>,
    due: HashMap<BufferId, f64>,
    pub queue: VecDeque<(BufferId, DiskChange)>,
    /// Followed documents.
    pub follow: HashSet<BufferId>,
    /// For each followed document: how much of its file it has (in bytes),
    /// and the last bytes of that.
    tails: HashMap<BufferId, (u64, Vec<u8>)>,
}

/// The size of the file at `path` and its last [`FOLLOW_TAIL`] bytes.
fn read_tail(path: &Path) -> Option<(u64, Vec<u8>)> {
    let mut f = std::fs::File::open(path).ok()?;
    let meta = f.metadata().ok()?;
    if !meta.is_file() {
        return None;
    }
    let len = meta.len();
    let start = len.saturating_sub(FOLLOW_TAIL);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut tail = Vec::new();
    f.take(len - start).read_to_end(&mut tail).ok()?;
    Some((len, tail))
}

/// Bytes `from..to` of the file at `path`.
fn read_range(path: &Path, from: u64, to: u64) -> Option<Vec<u8>> {
    let mut f = std::fs::File::open(path).ok()?;
    f.seek(SeekFrom::Start(from)).ok()?;
    let mut out = Vec::new();
    f.take(to - from).read_to_end(&mut out).ok()?;
    (out.len() as u64 == to - from).then_some(out)
}

/// How much of `new` (appended UTF-8 bytes) can be taken now: whole
/// characters, keeping back a final CR whose LF may still come. `None` if
/// it is not UTF-8 text.
fn takeable(new: &[u8]) -> Option<usize> {
    let mut take = match std::str::from_utf8(new) {
        Ok(_) => new.len(),
        // Cut inside the last character: the rest is still being written.
        Err(e) if e.error_len().is_none() => e.valid_up_to(),
        Err(_) => return None,
    };
    if take > 0 && new[take - 1] == b'\r' {
        take -= 1;
    }
    (!new[..take].contains(&0)).then_some(take)
}

impl OchrepadApp {
    /// Records what is on disk for `buffer` now (after load, save, reload).
    pub(super) fn record_disk_stamp(&mut self, buffer: BufferId) {
        let s = self
            .editor
            .buffer(buffer)
            .and_then(|b| b.meta.path.as_deref())
            .and_then(stamp);
        self.external.stamps.insert(buffer, s);
    }

    /// The external changes waiting for an answer.
    pub fn pending_disk_changes(&self) -> Vec<(BufferId, DiskChange)> {
        self.external.queue.iter().copied().collect()
    }

    /// Per frame: keeps the watcher on the open files' directories and turns
    /// its events into checks, then checks that are due.
    pub(super) fn watch_files(&mut self, ui: &Ui) {
        if self.external.watcher.is_none() {
            let (tx, rx) = channel();
            let ctx = ui.ctx().clone();
            if let Ok(w) = notify::recommended_watcher(move |res| {
                let _ = tx.send(res);
                ctx.request_repaint();
            }) {
                self.external.watcher = Some(w);
                self.external.rx = Some(rx);
            }
        }
        let files: Vec<(BufferId, PathBuf)> = self
            .editor
            .buffers()
            .filter_map(|b| b.meta.path.as_ref().map(|p| (b.id(), watch_key(p))))
            .collect();
        let mut dirs: HashSet<PathBuf> = files
            .iter()
            .filter_map(|(_, p)| p.parent().map(Path::to_path_buf))
            .collect();
        let tree_dirs = self
            .tree
            .as_ref()
            .map(|t| t.watched_dirs())
            .unwrap_or_default();
        if tree_dirs != self.external.tree_dirs_for {
            self.external.tree_dirs = tree_dirs
                .iter()
                .map(|d| (canonical(d), d.clone()))
                .collect();
            self.external.tree_dirs_for = tree_dirs;
        }
        dirs.extend(self.external.tree_dirs.iter().map(|(c, _)| c.clone()));
        if let Some(w) = &mut self.external.watcher {
            for d in dirs.difference(&self.external.watched) {
                let _ = w.watch(d, RecursiveMode::NonRecursive);
            }
            for d in self.external.watched.difference(&dirs) {
                let _ = w.unwatch(d);
            }
        }
        self.external.watched = dirs;

        let now = ui.input(|i| i.time);
        if let Some(rx) = &self.external.rx {
            for event in rx.try_iter().flatten() {
                for p in &event.paths {
                    let p = watch_key(p);
                    for (b, path) in &files {
                        if *path == p {
                            if self.external.follow.contains(b) {
                                self.external.due.entry(*b).or_insert(now + FOLLOW_SECS);
                            } else {
                                self.external.due.insert(*b, now + SETTLE_SECS);
                            }
                        }
                    }
                    if let (Some(tree), Some(parent)) = (&mut self.tree, p.parent()) {
                        for (c, d) in &self.external.tree_dirs {
                            if c == parent {
                                tree.changed(d, now);
                            }
                        }
                    }
                }
            }
        }
        let due: Vec<BufferId> = self
            .external
            .due
            .iter()
            .filter(|(_, t)| **t <= now)
            .map(|(b, _)| *b)
            .collect();
        for b in due {
            if self.is_saving(b) {
                // Our own write in progress: look again afterwards.
                self.external.due.insert(b, now + SETTLE_SECS);
                continue;
            }
            self.external.due.remove(&b);
            self.check_disk(b);
        }
        if let Some(next) = self.external.due.values().copied().reduce(f64::min) {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64((next - now).max(0.0)));
        }
    }

    /// Compares the file with what was recorded; queues a question if it
    /// changed or disappeared.
    fn check_disk(&mut self, buffer: BufferId) {
        let Some(path) = self.editor.buffer(buffer).and_then(|b| b.meta.path.clone()) else {
            return;
        };
        let now = stamp(&path);
        let change = match now {
            None if !path.exists() => DiskChange::Deleted,
            None => return,
            Some(s) if self.external.stamps.get(&buffer) == Some(&Some(s)) => return,
            Some(_) => DiskChange::Modified,
        };
        if change == DiskChange::Modified
            && self.external.follow.contains(&buffer)
            && self.follow_change(buffer)
        {
            return;
        }
        if !self.external.queue.iter().any(|(b, _)| *b == buffer) {
            self.external.queue.push_back((buffer, change));
        }
    }

    /// Whether `buffer` follows its file (View > Follow).
    pub fn is_following(&self, buffer: BufferId) -> bool {
        self.external.follow.contains(&buffer)
    }

    /// Starts or stops following `buffer`'s file. Starting brings the
    /// document up to date and moves the active view to its end.
    pub fn set_following(&mut self, buffer: BufferId, on: bool) {
        if !on {
            self.external.follow.remove(&buffer);
            self.external.tails.remove(&buffer);
            return;
        }
        let Some(path) = self.editor.buffer(buffer).and_then(|b| b.meta.path.clone()) else {
            return;
        };
        let clean = self.editor.buffer(buffer).is_some_and(|b| !b.is_dirty());
        let changed = self.external.stamps.get(&buffer) != Some(&stamp(&path));
        if clean && changed {
            self.reload_from_disk(buffer);
        }
        self.external.follow.insert(buffer);
        if let Some(t) = read_tail(&path) {
            self.external.tails.insert(buffer, t);
        }
        if self
            .editor
            .view(self.active)
            .is_some_and(|v| v.buffer() == buffer)
        {
            self.apply(ochrepad_core::Action::MoveCursor {
                motion: ochrepad_core::Motion::DocEnd,
                extend: false,
            });
        }
    }

    /// A followed file changed: appends what it gained, or reloads it.
    /// Returns false to ask as usual (the document has unsaved changes).
    fn follow_change(&mut self, buffer: BufferId) -> bool {
        let Some(b) = self.editor.buffer(buffer) else {
            return false;
        };
        let Some(path) = b.meta.path.clone() else {
            return false;
        };
        if b.is_dirty() {
            return false;
        }
        let end = b.doc().len();
        let utf8 = b.meta.encoding == "UTF-8";
        // The views watching the end keep watching it.
        let at_end: Vec<ochrepad_core::ViewId> = self
            .editor
            .views_of(buffer)
            .filter(|v| v.selections.primary().head == end)
            .map(|v| v.id())
            .collect();
        let appended = self.follow_append(buffer, &path, utf8);
        if !appended {
            self.reload_from_disk(buffer);
            if let Some(t) = read_tail(&path) {
                self.external.tails.insert(buffer, t);
            }
            let end = self.editor.buffer(buffer).map(|b| b.doc().len());
            for &v in &at_end {
                if let Some(end) = end {
                    let _ = self.editor.apply(
                        v,
                        ochrepad_core::Action::SetSelections(ochrepad_core::SelectionSet::caret(
                            end,
                        )),
                    );
                }
            }
        }
        for v in at_end {
            self.view_states.entry(v).or_default().reveal_caret();
        }
        true
    }

    /// Appends the bytes the file gained since the remembered end, if that
    /// end is still there. False when it must be reloaded instead.
    fn follow_append(&mut self, buffer: BufferId, path: &Path, utf8: bool) -> bool {
        let Some((had, tail)) = self.external.tails.get(&buffer).cloned() else {
            return false;
        };
        let Some(len) = std::fs::metadata(path).ok().map(|m| m.len()) else {
            return false;
        };
        if !utf8 || len < had || len - had > FOLLOW_MAX_APPEND {
            return false;
        }
        let from = had - tail.len() as u64;
        let Some(bytes) = read_range(path, from, len) else {
            return false;
        };
        if !bytes.starts_with(&tail) {
            return false;
        }
        let new = &bytes[tail.len()..];
        let Some(take) = takeable(new) else {
            return false;
        };
        let Ok(text) = std::str::from_utf8(&new[..take]) else {
            return false;
        };
        let text = fileio::normalize_line_endings(text);
        if self.editor.append_unrecorded(buffer, &text).is_err() {
            return false;
        }
        let kept = &bytes[..tail.len() + take];
        let keep_from = kept.len().saturating_sub(FOLLOW_TAIL as usize);
        self.external
            .tails
            .insert(buffer, (had + take as u64, kept[keep_from..].to_vec()));
        self.record_disk_stamp(buffer);
        true
    }

    /// Reloads `buffer` from its file (undoable).
    pub fn reload_from_disk(&mut self, buffer: BufferId) {
        let Some(path) = self.editor.buffer(buffer).and_then(|b| b.meta.path.clone()) else {
            return;
        };
        match fileio::load(&path) {
            Ok(d) => {
                if let Err(e) = self.editor.reload_buffer(buffer, &d.text, d.meta) {
                    self.message = Some(format!("Could not reload {}:\n{e}", path.display()));
                }
                self.record_disk_stamp(buffer);
                self.delete_snapshot(buffer);
            }
            Err(e) => self.message = Some(format!("Could not reload {}:\n{e}", path.display())),
        }
    }

    /// The question about the oldest external change, if no other dialog is
    /// showing.
    pub(super) fn external_dialog(&mut self, ui: &mut Ui) {
        if self.message.is_some() || self.confirming.is_some() {
            return;
        }
        let Some(&(buffer, change)) = self.external.queue.front() else {
            return;
        };
        let Some(b) = self.editor.buffer(buffer) else {
            self.external.queue.pop_front();
            return;
        };
        let name = self.buffer_name(buffer);
        let dirty = b.is_dirty();
        let (text, yes, no) = match (change, dirty) {
            (DiskChange::Modified, false) => (
                format!("{name} was changed by another program.\nReload it?"),
                "Reload",
                "Keep this version",
            ),
            (DiskChange::Modified, true) => (
                format!(
                    "{name} was changed by another program, and you have unsaved changes.\n\
                     Reload it and lose your changes?"
                ),
                "Reload",
                "Keep my changes",
            ),
            (DiskChange::Deleted, _) => (
                format!("{name} no longer exists on disk.\nKeep it open?"),
                "Keep open",
                "Close",
            ),
        };
        let resp = Modal::new(Id::new("ochrepad-external")).show(ui.ctx(), |ui| {
            ui.label(text);
            ui.add_space(8.0);
            ui.horizontal(|ui| (ui.button(yes).clicked(), ui.button(no).clicked()))
                .inner
        });
        let (first, second) = resp.inner;
        if !(first || second) {
            return;
        }
        self.external.queue.pop_front();
        match (change, first) {
            (DiskChange::Modified, true) => self.reload_from_disk(buffer),
            (DiskChange::Modified, false) => self.record_disk_stamp(buffer),
            (DiskChange::Deleted, true) => {
                // Keep: the text is no longer on disk, so it is unsaved.
                if let Some(b) = self.editor.buffer_mut(buffer) {
                    b.mark_modified();
                }
                self.record_disk_stamp(buffer);
            }
            (DiskChange::Deleted, false) => {
                let views: Vec<_> = self.editor.views_of(buffer).map(|v| v.id()).collect();
                for v in views {
                    self.close_view(v);
                }
            }
        }
    }
}
