//! A background thread for every disk write, so typing never waits on I/O
//! (PLAN M10). Jobs carry a cheap clone of the rope; encoding happens on the
//! worker. Dropping the [`Worker`] waits for queued jobs to finish.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use ochrepad_core::{Document, FileMeta};
use serde::{Deserialize, Serialize};

use crate::{save, write_atomic};

/// What a backup snapshot records besides the text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotInfo {
    pub meta: FileMeta,
    /// "new 3" for buffers that were never saved.
    pub untitled: Option<String>,
    pub language: Option<String>,
}

pub enum Job {
    /// Encode `doc` with `meta` and write it atomically to `path`.
    Save {
        path: PathBuf,
        doc: Document,
        meta: FileMeta,
    },
    /// Write `<dir>/<id>.txt` (UTF-8 text) and `<dir>/<id>.json` (info).
    Snapshot {
        dir: PathBuf,
        id: String,
        doc: Document,
        info: SnapshotInfo,
    },
    /// Delete files; missing ones are fine.
    Delete(Vec<PathBuf>),
    /// Write bytes atomically, creating the directory.
    Write { path: PathBuf, bytes: Vec<u8> },
}

/// A finished job: the tag `submit` returned, and the outcome.
#[derive(Debug)]
pub struct Done {
    pub tag: u64,
    pub result: Result<(), String>,
}

pub struct Worker {
    tx: Option<Sender<(u64, Job)>>,
    rx: Receiver<Done>,
    handle: Option<JoinHandle<()>>,
    next_tag: u64,
    in_flight: usize,
    /// Results of jobs that ran on the calling thread (no worker).
    fallback_done: Vec<Done>,
}

pub fn snapshot_paths(dir: &Path, id: &str) -> (PathBuf, PathBuf) {
    (
        dir.join(format!("{id}.txt")),
        dir.join(format!("{id}.json")),
    )
}

/// Reads a snapshot written by [`Job::Snapshot`].
pub fn read_snapshot(dir: &Path, id: &str) -> Result<(String, SnapshotInfo), String> {
    let (text_path, info_path) = snapshot_paths(dir, id);
    let info = crate::load_json::<SnapshotInfo>(&info_path)?
        .ok_or_else(|| format!("{} is missing", info_path.display()))?;
    let text =
        std::fs::read_to_string(&text_path).map_err(|e| format!("{}: {e}", text_path.display()))?;
    Ok((text, info))
}

fn run(job: Job) -> Result<(), String> {
    match job {
        Job::Save { path, doc, meta } => {
            save(&path, &doc.to_string(), &meta).map_err(|e| e.to_string())
        }
        Job::Snapshot { dir, id, doc, info } => {
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            let (text_path, info_path) = snapshot_paths(&dir, &id);
            write_atomic(&text_path, doc.to_string().as_bytes())
                .map_err(|e| format!("{}: {e}", text_path.display()))?;
            crate::save_json(&info_path, &info).map_err(|e| format!("{}: {e}", info_path.display()))
        }
        Job::Delete(paths) => {
            for p in paths {
                match std::fs::remove_file(&p) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(format!("{}: {e}", p.display())),
                }
            }
            Ok(())
        }
        Job::Write { path, bytes } => {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            write_atomic(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))
        }
    }
}

impl Default for Worker {
    fn default() -> Self {
        Self::spawn()
    }
}

impl Worker {
    pub fn spawn() -> Self {
        let (tx, jobs) = mpsc::channel::<(u64, Job)>();
        let (done_tx, rx) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("ochrepad-disk".to_owned())
            .spawn(move || {
                for (tag, job) in jobs {
                    let result = run(job);
                    if done_tx.send(Done { tag, result }).is_err() {
                        break;
                    }
                }
            })
            .ok();
        Self {
            tx: Some(tx),
            rx,
            handle,
            next_tag: 1,
            in_flight: 0,
            fallback_done: Vec::new(),
        }
    }

    /// Queues a job; returns its tag. If the thread could not start, the
    /// job runs here instead (it still never loses data).
    pub fn submit(&mut self, job: Job) -> u64 {
        let tag = self.next_tag;
        self.next_tag += 1;
        self.in_flight += 1;
        let job = match &self.tx {
            Some(tx) if self.handle.is_some() => match tx.send((tag, job)) {
                Ok(()) => return tag,
                Err(mpsc::SendError((_, job))) => job,
            },
            _ => job,
        };
        // Fallback: no worker thread.
        let result = run(job);
        self.in_flight -= 1;
        self.fallback_done.push(Done { tag, result });
        tag
    }

    /// Finished jobs since the last call.
    pub fn poll(&mut self) -> Vec<Done> {
        let mut out: Vec<Done> = std::mem::take(&mut self.fallback_done);
        while let Ok(d) = self.rx.try_recv() {
            self.in_flight -= 1;
            out.push(d);
        }
        out
    }

    /// Jobs submitted but not yet reported by `poll`.
    pub fn in_flight(&self) -> usize {
        self.in_flight
    }

    /// Blocks until every queued job has finished, and returns their
    /// results (used at exit and by tests).
    pub fn wait_idle(&mut self) -> Vec<Done> {
        let mut out = std::mem::take(&mut self.fallback_done);
        while self.in_flight > 0 {
            match self.rx.recv() {
                Ok(d) => {
                    self.in_flight -= 1;
                    out.push(d);
                }
                Err(_) => break,
            }
        }
        out
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Closing the channel ends the thread after the queued jobs.
        self.tx = None;
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
