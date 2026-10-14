//! Noticing when an open file changes on disk (PLAN M13): a `notify` watcher
//! on the directories of open files; each buffer remembers the modified time
//! and size of its file when it was loaded or saved. A change by another
//! program asks to reload (a clean buffer) or warns (a dirty one); a deleted
//! file asks whether to keep it open.
//!
//! The same watcher covers the file tree's root and expanded folders, so the
//! tree relists a folder when something in it changes.

use std::collections::{HashMap, HashSet, VecDeque};
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
                            self.external.due.insert(*b, now + SETTLE_SECS);
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
        if !self.external.queue.iter().any(|(b, _)| *b == buffer) {
            self.external.queue.push_back((buffer, change));
        }
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
