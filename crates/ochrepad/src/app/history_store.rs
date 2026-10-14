//! Undo history kept between sessions (PLAN_v0.3.0 P5). Each document's
//! history is a file in the data folder's `history`: named after its file
//! (a hash of the canonical path) while the text is what is on disk, or
//! after its backup snapshot while it has unsaved changes. The file records
//! the SHA-256 of the text the history ends at, and a history is restored
//! only onto that exact text, after core checks every step against it.
//!
//! Histories are written when a document is saved, backed up or closed, and
//! at exit, through the worker; at most [`HISTORY_BYTES`] of the newest
//! steps per document. The oldest files beyond [`HISTORY_FILES`] are pruned
//! at startup.

use std::path::{Path, PathBuf};

use ochrepad_core::{BufferId, Document};
use ochrepad_fileio::worker::{Job, read_history};

use super::OchrepadApp;
use super::autosave::PendingJob;

/// The most history kept for one document, in bytes of text (roughly).
pub const HISTORY_BYTES: usize = 2 * 1024 * 1024;
/// History files kept; older ones are deleted at startup.
const HISTORY_FILES: usize = 300;

/// Where a document's history is stored.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Key {
    /// Its file, whose text it matches.
    File(PathBuf),
    /// Its backup snapshot (unsaved changes).
    Backup(String),
}

impl OchrepadApp {
    fn history_dir(&self) -> Option<&Path> {
        self.persist
            .history_dir
            .as_deref()
            .filter(|_| self.settings.undo_history)
    }

    fn history_file(&self, key: &Key) -> Option<PathBuf> {
        let dir = self.history_dir()?;
        let name = match key {
            Key::File(path) => {
                let canonical = super::canonical(path);
                let hash = ochrepad_core::sha256_hex(&Document::new(&canonical.to_string_lossy()));
                format!("{}.json", &hash[..32])
            }
            Key::Backup(id) => format!("backup-{id}.json"),
        };
        Some(dir.join(name))
    }

    /// Where `buffer`'s history belongs now: its file while the text is what
    /// is on disk, else its backup while it has one.
    fn history_key(&self, buffer: BufferId) -> Option<Key> {
        let b = self.editor.buffer(buffer)?;
        match (&b.meta.path, b.is_dirty()) {
            (Some(path), false) => Some(Key::File(path.clone())),
            (_, true) if self.snapshots_active() => self
                .persist
                .snapshot_ids
                .get(&buffer)
                .map(|id| Key::Backup(id.clone())),
            _ => None,
        }
    }

    /// Writes `buffer`'s history where it belongs now, unless that was
    /// already done for this revision.
    pub(super) fn write_history(&mut self, buffer: BufferId) {
        let Some(key) = self.history_key(buffer) else {
            return;
        };
        let Some(file) = self.history_file(&key) else {
            return;
        };
        let Some(b) = self.editor.buffer(buffer) else {
            return;
        };
        let stamp = (b.revision(), file.clone());
        if self.persist.history_written.get(&buffer) == Some(&stamp) {
            return;
        }
        let history = b.saved_history(HISTORY_BYTES);
        if history.is_empty() {
            return;
        }
        let job = Job::History {
            file,
            path: b.meta.path.clone(),
            doc: b.doc().clone(),
            history,
        };
        let tag = self.persist.worker.submit(job);
        self.persist.pending.insert(tag, PendingJob::Other);
        self.persist.history_written.insert(buffer, stamp);
    }

    /// Writes every document's history (at exit).
    pub(super) fn write_all_histories(&mut self) {
        let ids: Vec<BufferId> = self.editor.buffers().map(|b| b.id()).collect();
        for id in ids {
            self.write_history(id);
        }
    }

    /// Gives `buffer`, just opened from `path`, the history it had when it
    /// was last closed with this same text.
    pub(super) fn restore_file_history(&mut self, buffer: BufferId, path: &Path) {
        self.restore_history(buffer, &Key::File(path.to_path_buf()), true);
    }

    /// Gives `buffer`, just restored from backup `id`, its history.
    pub(super) fn restore_backup_history(&mut self, buffer: BufferId, id: &str) {
        self.restore_history(buffer, &Key::Backup(id.to_owned()), false);
    }

    fn restore_history(&mut self, buffer: BufferId, key: &Key, on_disk: bool) {
        let Some(file) = self.history_file(key) else {
            return;
        };
        // Most files have none; reading nothing is cheap.
        let record = match read_history(&file) {
            Ok(Some(r)) => r,
            Ok(None) => return,
            Err(_) => {
                // Unreadable: drop it rather than ask about it each time.
                self.delete_history_file(file);
                return;
            }
        };
        let Some(b) = self.editor.buffer_mut(buffer) else {
            return;
        };
        let same_file = match key {
            Key::File(path) => record
                .path
                .as_deref()
                .is_some_and(|p| super::canonical(p) == super::canonical(path)),
            Key::Backup(_) => true,
        };
        if !same_file || record.text_sha256 != ochrepad_core::sha256_hex(b.doc()) {
            // The file changed since (or the name hash collided): the history
            // belongs to other text.
            return;
        }
        if b.restore_history(record.history, on_disk).is_err() {
            self.delete_history_file(file);
        }
    }

    fn delete_history_file(&mut self, file: PathBuf) {
        let tag = self.persist.worker.submit(Job::Delete(vec![file]));
        self.persist.pending.insert(tag, PendingJob::Other);
    }

    /// The history kept for backup `id` is no longer needed (the document
    /// was saved, or closed without saving).
    pub(super) fn delete_backup_history(&mut self, id: &str) {
        if let Some(file) = self.history_file(&Key::Backup(id.to_owned())) {
            self.delete_history_file(file);
        }
    }

    /// Edit > Clear Undo History: forgets the active document's undo and
    /// redo steps, here and on disk.
    pub fn clear_undo_history(&mut self) {
        let buffer = self.buffer_id();
        let mut files = Vec::new();
        if let Some(path) = self.buffer().meta.path.clone() {
            files.extend(self.history_file(&Key::File(path)));
        }
        if let Some(id) = self.persist.snapshot_ids.get(&buffer).cloned() {
            files.extend(self.history_file(&Key::Backup(id)));
        }
        if let Some(b) = self.editor.buffer_mut(buffer) {
            b.clear_history();
        }
        self.persist.history_written.remove(&buffer);
        for f in files {
            self.delete_history_file(f);
        }
        self.note_text("Undo history cleared");
    }

    /// Deletes the oldest history files beyond the limit (at startup).
    pub(super) fn prune_histories(&mut self) {
        if let Some(dir) = self.persist.history_dir.clone() {
            let tag = self.persist.worker.submit(Job::Prune {
                dir,
                keep: HISTORY_FILES,
            });
            self.persist.pending.insert(tag, PendingJob::Other);
        }
    }
}
