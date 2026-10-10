//! Background saving, backup snapshots and session restore (PLAN M10).
//!
//! Every disk write goes through the `fileio` worker thread. Notepad++'s
//! "autosave" is a backup system and so is this: every few seconds each
//! dirty buffer's text is copied to the data directory, and on exit the
//! open tabs (with those snapshots) are written to `session.json`, so
//! unsaved work comes back dirty on the next launch. Optionally, files with
//! a path are also saved in place (write-through).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use egui::Ui;
use egui_dock::DockState;
use ochrepad_core::{Action, BufferId, Selection, SelectionSet, ViewId};
use ochrepad_fileio as fileio;
use ochrepad_fileio::worker::{Job, SnapshotInfo, Worker, read_snapshot, snapshot_paths};
use serde::{Deserialize, Serialize};

use super::{OchrepadApp, Tab};

#[derive(Clone, Debug, PartialEq)]
pub struct AutosaveSettings {
    /// Periodic backup snapshots of unsaved buffers, and restoring them with
    /// the session. With this on, exiting does not ask about unsaved files.
    pub snapshots: bool,
    /// Seconds between autosave ticks.
    pub interval_secs: f64,
    /// Also save buffers that have a file to that file, on the timer and
    /// when the window loses focus.
    pub write_through: bool,
}

impl Default for AutosaveSettings {
    fn default() -> Self {
        Self {
            snapshots: true,
            interval_secs: 7.0,
            write_through: false,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Session {
    pub buffers: Vec<SessionBuffer>,
    pub tabs: Vec<SessionTab>,
    /// The dock layout with tabs as indices into `tabs`.
    pub layout: Option<DockState<usize>>,
    /// Index into `tabs`.
    pub active: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionBuffer {
    pub path: Option<std::path::PathBuf>,
    /// A backup snapshot holding unsaved text.
    pub snapshot: Option<String>,
    pub language: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionTab {
    /// Index into `buffers`.
    pub buffer: usize,
    pub anchor: usize,
    pub head: usize,
    pub first_line: usize,
    pub x_offset: f32,
}

enum PendingJob {
    Save {
        buffer: BufferId,
        revision: u64,
        path: PathBuf,
    },
    Other,
}

#[derive(Default)]
pub(super) struct Persist {
    worker: Worker,
    pending: HashMap<u64, PendingJob>,
    pub settings: AutosaveSettings,
    backup_dir: Option<PathBuf>,
    session_file: Option<PathBuf>,
    snapshot_ids: HashMap<BufferId, String>,
    /// Revision of the last snapshot written for each buffer.
    snapshotted: HashMap<BufferId, u64>,
    pub saving: HashSet<BufferId>,
    last_tick: Option<f64>,
    last_session: Vec<u8>,
    window_focused: bool,
}

impl OchrepadApp {
    pub fn autosave_settings(&mut self) -> &mut AutosaveSettings {
        &mut self.persist.settings
    }

    /// Uses `config_dir` for `session.json` and `data_dir/backup` for
    /// snapshots, and restores the last session.
    pub fn set_storage(&mut self, config_dir: &Path, data_dir: &Path) {
        self.persist.session_file = Some(config_dir.join("session.json"));
        self.persist.backup_dir = Some(data_dir.join("backup"));
        self.restore_session();
    }

    pub(super) fn snapshots_active(&self) -> bool {
        self.persist.settings.snapshots && self.persist.backup_dir.is_some()
    }

    /// Whether a save of `buffer` is in progress.
    pub fn is_saving(&self, buffer: BufferId) -> bool {
        self.persist.saving.contains(&buffer)
    }

    /// Queues a save of `buffer` to `path`; the buffer is marked clean (and
    /// takes the path) when the write succeeds.
    pub(super) fn submit_save(&mut self, buffer: BufferId, path: PathBuf) -> bool {
        let Some(b) = self.editor.buffer(buffer) else {
            return false;
        };
        let job = Job::Save {
            path: path.clone(),
            doc: b.doc().clone(),
            meta: b.meta.clone(),
        };
        let revision = b.revision();
        let tag = self.persist.worker.submit(job);
        self.persist.pending.insert(
            tag,
            PendingJob::Save {
                buffer,
                revision,
                path,
            },
        );
        self.persist.saving.insert(buffer);
        true
    }

    /// Handles finished disk jobs.
    pub(super) fn poll_disk(&mut self, ui: &Ui) {
        let done = self.persist.worker.poll();
        self.handle_done(done);
        if self.persist.worker.in_flight() > 0 {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(5));
        }
    }

    /// Waits for every queued write (exit, tests).
    pub fn flush_disk(&mut self) {
        let done = self.persist.worker.wait_idle();
        self.handle_done(done);
    }

    fn handle_done(&mut self, done: Vec<fileio::worker::Done>) {
        for d in done {
            match self.persist.pending.remove(&d.tag) {
                Some(PendingJob::Save {
                    buffer,
                    revision,
                    path,
                }) => {
                    self.persist.saving.remove(&buffer);
                    match d.result {
                        Ok(()) => self.save_finished(buffer, revision, path),
                        Err(e) => {
                            self.message = Some(format!("Could not save {}:\n{e}", path.display()));
                            // A close or exit waiting on this save stops.
                            self.close_queue.clear();
                            self.exit_queue.clear();
                            self.exiting = false;
                        }
                    }
                }
                Some(PendingJob::Other) | None => {
                    if let Err(e) = d.result {
                        self.message = Some(format!("Could not write a backup:\n{e}"));
                    }
                }
            }
        }
    }

    fn save_finished(&mut self, buffer: BufferId, revision: u64, path: PathBuf) {
        let Some(b) = self.editor.buffer_mut(buffer) else {
            return;
        };
        b.mark_saved_at(revision);
        let renamed = b.meta.path.as_deref() != Some(path.as_path());
        if renamed {
            b.meta.path = Some(path);
        }
        let clean = !b.is_dirty();
        if renamed {
            self.untitled.remove(&buffer);
            // A new name may mean a new language, as in Notepad++.
            self.attach_highlighting(buffer);
        }
        if clean {
            self.delete_snapshot(buffer);
        }
    }

    /// Removes `buffer`'s backup snapshot, if it has one.
    pub(super) fn delete_snapshot(&mut self, buffer: BufferId) {
        if self.persist.snapshotted.remove(&buffer).is_none() {
            return;
        }
        let (Some(dir), Some(id)) = (
            &self.persist.backup_dir,
            self.persist.snapshot_ids.get(&buffer),
        ) else {
            return;
        };
        let (txt, json) = snapshot_paths(dir, id);
        let tag = self.persist.worker.submit(Job::Delete(vec![txt, json]));
        self.persist.pending.insert(tag, PendingJob::Other);
    }

    fn snapshot_id(&mut self, buffer: BufferId) -> String {
        self.persist
            .snapshot_ids
            .entry(buffer)
            .or_insert_with(|| {
                let millis = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis());
                format!("{millis:x}-{}", buffer.0)
            })
            .clone()
    }

    /// Writes snapshots of dirty buffers that changed since their last one,
    /// and deletes those of buffers that are clean again.
    pub(super) fn snapshot_dirty(&mut self) {
        let Some(dir) = self.persist.backup_dir.clone() else {
            return;
        };
        let buffers: Vec<(BufferId, bool, u64)> = self
            .editor
            .buffers()
            .map(|b| (b.id(), b.is_dirty(), b.revision()))
            .collect();
        for (id, dirty, revision) in buffers {
            if !dirty {
                self.delete_snapshot(id);
                continue;
            }
            if self.persist.snapshotted.get(&id) == Some(&revision) {
                continue;
            }
            let snapshot = self.snapshot_id(id);
            let Some(b) = self.editor.buffer(id) else {
                continue;
            };
            let job = Job::Snapshot {
                dir: dir.clone(),
                id: snapshot,
                doc: b.doc().clone(),
                info: SnapshotInfo {
                    meta: b.meta.clone(),
                    untitled: self.untitled.get(&id).cloned(),
                    language: b.language.clone(),
                },
            };
            let tag = self.persist.worker.submit(job);
            self.persist.pending.insert(tag, PendingJob::Other);
            self.persist.snapshotted.insert(id, revision);
        }
    }

    /// Saves dirty buffers that have a file (write-through).
    fn save_dirty_files(&mut self) {
        let targets: Vec<(BufferId, PathBuf)> = self
            .editor
            .buffers()
            .filter(|b| b.is_dirty() && !self.persist.saving.contains(&b.id()))
            .filter_map(|b| b.meta.path.clone().map(|p| (b.id(), p)))
            .collect();
        for (buffer, path) in targets {
            self.submit_save(buffer, path);
        }
    }

    /// Runs every frame: the autosave timer and write-through on focus loss.
    pub(super) fn autosave_tick(&mut self, ui: &Ui) {
        let focused = ui.input(|i| i.viewport().focused.unwrap_or(true));
        let lost_focus = self.persist.window_focused && !focused;
        self.persist.window_focused = focused;
        if lost_focus && self.persist.settings.write_through {
            self.save_dirty_files();
        }

        let now = ui.input(|i| i.time);
        let interval = self.persist.settings.interval_secs.max(0.5);
        let last = *self.persist.last_tick.get_or_insert(now);
        if now - last < interval {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(interval - (now - last)));
            return;
        }
        self.persist.last_tick = Some(now);
        if self.snapshots_active() {
            self.snapshot_dirty();
        }
        if self.persist.settings.write_through {
            self.save_dirty_files();
        }
        self.write_session();
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(interval));
    }

    /// The open tabs as a session. Unsaved buffers are included only with
    /// their snapshot (and untitled ones only then).
    pub fn session(&self) -> Session {
        let snapshots = self.snapshots_active();
        let mut session = Session::default();
        let mut buffer_index: HashMap<BufferId, usize> = HashMap::new();
        let mut tab_index: HashMap<ViewId, usize> = HashMap::new();
        for view in self.tabs() {
            let Some(v) = self.editor.view(view) else {
                continue;
            };
            let id = v.buffer();
            let Some(b) = self.editor.buffer(id) else {
                continue;
            };
            let index = match buffer_index.get(&id) {
                Some(&i) => i,
                None => {
                    let snapshot = (snapshots && b.is_dirty())
                        .then(|| self.persist.snapshot_ids.get(&id).cloned())
                        .flatten();
                    if b.meta.path.is_none() && snapshot.is_none() {
                        continue;
                    }
                    session.buffers.push(SessionBuffer {
                        path: b.meta.path.clone(),
                        snapshot,
                        language: b.language.clone(),
                    });
                    buffer_index.insert(id, session.buffers.len() - 1);
                    session.buffers.len() - 1
                }
            };
            let sel = v.selections.primary();
            tab_index.insert(view, session.tabs.len());
            session.tabs.push(SessionTab {
                buffer: index,
                anchor: sel.anchor.0,
                head: sel.head.0,
                first_line: v.scroll.first_line,
                x_offset: v.scroll.x_offset,
            });
        }
        session.layout = Some(self.dock.filter_map_tabs(|t| match t {
            Tab::Editor(v) => tab_index.get(v).copied(),
            Tab::Preview(_) => None,
        }));
        session.active = tab_index.get(&self.active).copied();
        session
    }

    /// Writes the session file if it changed (in the background).
    pub(super) fn write_session(&mut self) {
        let Some(path) = self.persist.session_file.clone() else {
            return;
        };
        let Ok(bytes) = serde_json::to_vec_pretty(&self.session()) else {
            return;
        };
        if bytes == self.persist.last_session {
            return;
        }
        self.persist.last_session = bytes.clone();
        let tag = self.persist.worker.submit(Job::Write { path, bytes });
        self.persist.pending.insert(tag, PendingJob::Other);
    }

    /// Exit with snapshots on: back up everything, write the session, wait
    /// for the disk, and close without asking.
    pub(super) fn exit_with_snapshots(&mut self) {
        self.snapshot_dirty();
        self.write_session();
        self.flush_disk();
        self.close_confirmed = true;
    }

    /// Restores the session written at the last exit.
    pub fn restore_session(&mut self) {
        let Some(file) = self.persist.session_file.clone() else {
            return;
        };
        let session = match fileio::load_json::<Session>(&file) {
            Ok(Some(s)) => s,
            Ok(None) => return,
            Err(e) => {
                self.message = Some(format!("Could not restore the last session:\n{e}"));
                return;
            }
        };
        let mut problems = Vec::new();
        let mut buffers: Vec<Option<BufferId>> = Vec::new();
        for sb in &session.buffers {
            let from_snapshot = match (&sb.snapshot, &self.persist.backup_dir) {
                (Some(id), Some(dir)) => match read_snapshot(dir, id) {
                    Ok(found) => Some((id.clone(), found)),
                    Err(e) => {
                        problems.push(format!("backup {id}: {e}"));
                        None
                    }
                },
                _ => None,
            };
            let buffer = if let Some((id, (text, info))) = from_snapshot {
                let b = self.editor.add_buffer(&text, info.meta);
                if let Some(buf) = self.editor.buffer_mut(b) {
                    buf.mark_modified();
                    let revision = buf.revision();
                    self.persist.snapshotted.insert(b, revision);
                }
                self.persist.snapshot_ids.insert(b, id);
                if let Some(name) = info.untitled {
                    self.untitled.insert(b, name);
                }
                Some(b)
            } else if let Some(path) = &sb.path {
                match fileio::load(path) {
                    Ok(d) => Some(self.editor.add_buffer(&d.text, d.meta)),
                    Err(e) => {
                        problems.push(format!("{}: {e}", path.display()));
                        None
                    }
                }
            } else {
                None
            };
            if let Some(b) = buffer {
                self.attach_highlighting(b);
                if let Some(language) = &sb.language {
                    let dark = self.dark;
                    if let Some(buf) = self.editor.buffer_mut(b) {
                        self.highlighting.set_language(buf, language, dark);
                    }
                }
            }
            buffers.push(buffer);
        }
        let views: Vec<Option<ViewId>> = session
            .tabs
            .iter()
            .map(|tab| {
                let buffer = buffers.get(tab.buffer).copied().flatten()?;
                let view = self.editor.add_view(buffer).ok()?;
                let sel = Selection::new(
                    ochrepad_core::ByteOffset(tab.anchor),
                    ochrepad_core::ByteOffset(tab.head),
                );
                let _ = self
                    .editor
                    .apply(view, Action::SetSelections(SelectionSet::single(sel)));
                if let Some(v) = self.editor.view_mut(view) {
                    v.scroll.first_line = tab.first_line;
                    v.scroll.x_offset = tab.x_offset;
                }
                Some(view)
            })
            .collect();
        let restored: Vec<ViewId> = views.iter().flatten().copied().collect();
        if restored.is_empty() {
            if !problems.is_empty() {
                self.message = Some(format!(
                    "Some files from the last session could not be opened:\n{}",
                    problems.join("\n")
                ));
            }
            return;
        }
        let dock = session
            .layout
            .map(|l| l.filter_map_tabs(|i| views.get(*i).copied().flatten().map(Tab::Editor)))
            .filter(|d| d.iter_all_tabs().count() == restored.len())
            .unwrap_or_else(|| DockState::new(restored.iter().copied().map(Tab::Editor).collect()));

        // Replace the empty startup tab(s).
        let startup: Vec<ViewId> = self.tabs();
        self.dock = dock;
        for view in startup {
            let buffer = self.editor.view(view).map(|v| v.buffer());
            self.editor.remove_view(view);
            self.view_states.remove(&view);
            if let Some(b) = buffer
                && self.editor.views_of(b).next().is_none()
            {
                self.editor.remove_buffer(b);
                self.highlighting.detach(b);
                self.untitled.remove(&b);
            }
        }
        let next = self
            .untitled
            .values()
            .filter_map(|n| n.strip_prefix("new ")?.parse::<usize>().ok())
            .max()
            .unwrap_or(0);
        self.next_untitled = next + 1;
        let active = session
            .active
            .and_then(|i| views.get(i).copied().flatten())
            .unwrap_or(restored[0]);
        self.activate(active);
        if !problems.is_empty() {
            self.message = Some(format!(
                "Some files from the last session could not be opened:\n{}",
                problems.join("\n")
            ));
        }
        // What was restored is what is on disk now.
        self.persist.last_session = serde_json::to_vec_pretty(&self.session()).unwrap_or_default();
    }
}
