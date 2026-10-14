//! Replace in Files, as in Notepad++'s Find in Files: replaces every match
//! of the find panel's pattern in the files of a folder. First the files
//! are searched on a thread and the matches counted, and the user is asked.
//! Then documents open in ochrepad get a Replace All each (an undo step,
//! left unsaved), and the other files are rewritten on a thread and saved by
//! the worker, atomically, with their own encoding, byte order mark and
//! line endings.
//!
//! Only what the pattern matches may change in a file. So a file is left
//! alone, and listed, if it would not load and save back byte for byte
//! (mixed line endings, which a save makes all alike; legacy bytes with
//! two forms), if it is read-only on disk, or if it changed after it was
//! read (the worker checks its length and modification time just before
//! writing). Binary files and files over 50 MB are skipped, as Find in Files
//! skips them. The writing shows its progress and can be stopped; ochrepad
//! does not exit in the middle of it.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, channel, sync_channel};
use std::time::SystemTime;

use egui::{Id, Modal, Ui};
use ochrepad_core::{
    Action, BufferId, ByteOffset, ChangeSet, Document, Edit, SearchQuery, Searcher,
};
use ochrepad_fileio::tree::WalkOptions;

use super::OchrepadApp;
use super::autosave::PendingJob;
use crate::search_results::MAX_FILE_BYTES;

/// Saves handed to the worker at once at most (the rest wait on the
/// reading thread, so memory stays bounded).
const MAX_IN_FLIGHT: usize = 4;
/// Files a report lists by name at most.
const MAX_LISTED: usize = 20;

/// Why a file with matches is left alone.
const MIXED: &str = "would not be saved back byte for byte (mixed line endings?)";
const READ_ONLY: &str = "read-only on disk";
const READ_ONLY_DOC: &str = "open as Read-Only";

/// A file to rewrite, as it was when counted.
pub(super) struct PlanFile {
    path: PathBuf,
    matches: usize,
    len: u64,
    modified: Option<SystemTime>,
}

/// What the count found.
pub(super) struct Plan {
    query: SearchQuery,
    replacement: String,
    folder: PathBuf,
    files: Vec<PlanFile>,
    open: Vec<(BufferId, usize)>,
    /// Files with matches that will be left alone, and why.
    skipped: Vec<(PathBuf, &'static str)>,
}

/// What the counting thread found: files to rewrite, open documents with
/// matches, and files with matches left alone.
type Counted = (
    Vec<PlanFile>,
    Vec<(BufferId, usize)>,
    Vec<(PathBuf, &'static str)>,
);

pub(super) enum CountMsg {
    Progress(usize),
    Done(Counted),
    Failed(String),
}

pub(super) enum WriteMsg {
    /// A file's new bytes, to be written if it is still as it was read.
    Ready {
        path: PathBuf,
        bytes: Vec<u8>,
        len: u64,
        modified: Option<SystemTime>,
        matches: usize,
    },
    /// A file left as it was, and why.
    Failed(PathBuf, String),
    Done,
}

#[derive(Default)]
pub(super) enum Stage {
    #[default]
    Idle,
    Counting {
        rx: Receiver<CountMsg>,
        cancel: Arc<AtomicBool>,
        files: usize,
        query: SearchQuery,
        replacement: String,
        folder: PathBuf,
    },
    Confirm(Plan),
    Writing(Writing),
}

pub(super) struct Writing {
    rx: Receiver<WriteMsg>,
    stop: Arc<AtomicBool>,
    /// Files to rewrite in all.
    total: usize,
    /// Matches replaced in open documents, and their number.
    in_open: (usize, usize),
    /// Files written, and the matches in them; saves in flight.
    saved: usize,
    matches: usize,
    saving: usize,
    /// Files left alone: when counting, and while writing.
    left: Vec<String>,
    reading_done: bool,
}

/// `doc` with every match of `searcher` replaced (each expanded).
fn replace_all(doc: &Document, searcher: &Searcher, replacement: &str) -> (Document, usize) {
    let matches = searcher.find_all(doc, ByteOffset(0)..doc.len());
    let edits: Vec<Edit> = matches
        .iter()
        .map(|m| Edit::new(m.start, m.end, searcher.expand(doc, m.clone(), replacement)))
        .collect();
    let changes = ChangeSet::new(doc.len(), edits);
    let mut out = doc.clone();
    changes.apply(&mut out);
    (out, matches.len())
}

fn stamp(meta: &std::fs::Metadata) -> (u64, Option<SystemTime>) {
    (meta.len(), meta.modified().ok())
}

/// "path: why" lines for a report, the first [`MAX_LISTED`] of them.
fn listed(lines: &[String]) -> String {
    let mut out: Vec<&str> = lines.iter().take(MAX_LISTED).map(String::as_str).collect();
    let more = lines.len().saturating_sub(MAX_LISTED);
    let tail = format!("and {more} more");
    if more > 0 {
        out.push(&tail);
    }
    out.join("\n")
}

/// Counts the matches in the folder's files, on the counting thread.
fn count(
    searcher: &Searcher,
    dir: &Path,
    options: &WalkOptions,
    open: Vec<(PathBuf, BufferId, Option<Document>)>,
    stop: &AtomicBool,
    tx: &std::sync::mpsc::Sender<CountMsg>,
) -> Result<Option<Counted>, String> {
    let walk = ochrepad_fileio::tree::walk_files(dir, options)?;
    // Open documents by canonical path; a walked file is canonicalized
    // only if it has an open document's name.
    let names: HashSet<OsString> = open
        .iter()
        .filter_map(|o| o.0.file_name().map(OsString::from))
        .collect();
    let open: Vec<(PathBuf, BufferId, Option<Document>)> = open
        .into_iter()
        .map(|(p, id, doc)| (super::canonical(&p), id, doc))
        .collect();
    let (mut files, mut in_open, mut skipped) = (Vec::new(), Vec::new(), Vec::new());
    for (seen, path) in walk.enumerate() {
        if stop.load(Ordering::Relaxed) {
            return Ok(None);
        }
        if seen % 100 == 99 && tx.send(CountMsg::Progress(seen + 1)).is_err() {
            return Ok(None);
        }
        let opened = path
            .file_name()
            .filter(|n| names.contains(*n))
            .map(|_| super::canonical(&path))
            .and_then(|c| open.iter().find(|(p, _, _)| *p == c));
        if let Some((_, id, doc)) = opened {
            match doc {
                Some(doc) => {
                    let n = searcher.matches(doc, ByteOffset(0)..doc.len()).count();
                    if n > 0 {
                        in_open.push((*id, n));
                    }
                }
                None => {
                    let doc_has = |p: &Path| {
                        ochrepad_fileio::load(p).is_ok_and(|d| {
                            let doc = Document::new(&d.text);
                            searcher.find_next(&doc, ByteOffset(0)).is_some()
                        })
                    };
                    if doc_has(&path) {
                        skipped.push((path, READ_ONLY_DOC));
                    }
                }
            }
            continue;
        }
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if meta.len() > MAX_FILE_BYTES {
            continue;
        }
        // Binary, undecodable and unreadable files are skipped.
        let Ok(bytes) = ochrepad_fileio::read_file(&path) else {
            continue;
        };
        let Ok(decoded) = ochrepad_fileio::decode(&bytes) else {
            continue;
        };
        let doc = Document::new(&decoded.text);
        let n = searcher.matches(&doc, ByteOffset(0)..doc.len()).count();
        if n == 0 {
            continue;
        }
        if meta.permissions().readonly() {
            skipped.push((path, READ_ONLY));
        } else if !ochrepad_fileio::round_trips(&bytes, &decoded) {
            skipped.push((path, MIXED));
        } else {
            let (len, modified) = stamp(&meta);
            files.push(PlanFile {
                path,
                matches: n,
                len,
                modified,
            });
        }
    }
    Ok(Some((files, in_open, skipped)))
}

/// Makes each file's new bytes, on the writing thread; `tx` holds a few at
/// a time, so the thread waits while the worker writes.
fn rewrite(
    files: Vec<PlanFile>,
    searcher: &Searcher,
    replacement: &str,
    stop: &AtomicBool,
    tx: &SyncSender<WriteMsg>,
) {
    for f in files {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let fail = |why: String| WriteMsg::Failed(f.path.clone(), why);
        let msg = match std::fs::metadata(&f.path) {
            Ok(meta) if stamp(&meta) != (f.len, f.modified) => {
                fail("it changed since the matches were counted".to_owned())
            }
            Ok(_) => match ochrepad_fileio::read_file(&f.path)
                .map_err(|e| e.to_string())
                .and_then(|bytes| {
                    let decoded = ochrepad_fileio::decode(&bytes).map_err(|e| e.to_string())?;
                    if !ochrepad_fileio::round_trips(&bytes, &decoded) {
                        return Err(MIXED.to_owned());
                    }
                    let doc = Document::new(&decoded.text);
                    let (new, n) = replace_all(&doc, searcher, replacement);
                    let bytes = ochrepad_fileio::encode(&new.to_string(), &decoded.meta)
                        .map_err(|e| e.to_string())?;
                    Ok((bytes, n))
                }) {
                Ok((_, 0)) => continue,
                Ok((bytes, matches)) => WriteMsg::Ready {
                    path: f.path,
                    bytes,
                    len: f.len,
                    modified: f.modified,
                    matches,
                },
                Err(e) => fail(e),
            },
            Err(e) => fail(e.to_string()),
        };
        if tx.send(msg).is_err() {
            return;
        }
    }
    let _ = tx.send(WriteMsg::Done);
}

impl OchrepadApp {
    /// Replace in Files: counts the matches in the find panel's folder, then
    /// asks.
    pub fn plan_replace_in_files(&mut self) {
        if !matches!(self.replace_in_files, Stage::Idle) {
            return;
        }
        let folder = PathBuf::from(self.find.folder.trim());
        if !folder.is_dir() {
            self.find
                .set_message(format!("{} is not a folder", folder.display()), true);
            return;
        }
        let mut query = self.find.query();
        query.options.in_selection = false;
        let Ok(searcher) = Searcher::new(&query) else {
            return;
        };
        let options = WalkOptions {
            subfolders: self.find.subfolders,
            include_ignored: self.find.include_ignored,
            filters: ochrepad_fileio::tree::parse_filters(&self.find.filters),
        };
        // Open documents are counted as they are in the editor; a Read-Only
        // one is left alone, in the editor and on disk.
        let open: Vec<(PathBuf, BufferId, Option<Document>)> = self
            .editor
            .buffers()
            .filter_map(|b| {
                let path = b.meta.path.clone()?;
                let doc = (!b.read_only).then(|| b.doc().clone());
                Some((path, b.id(), doc))
            })
            .collect();
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let dir = folder.clone();
        let spawned = std::thread::Builder::new()
            .name("ochrepad-replace-count".into())
            .spawn(
                move || match count(&searcher, &dir, &options, open, &stop, &tx) {
                    Ok(Some(counted)) => {
                        let _ = tx.send(CountMsg::Done(counted));
                    }
                    Ok(None) => {}
                    Err(e) => {
                        let _ = tx.send(CountMsg::Failed(e));
                    }
                },
            );
        if spawned.is_err() {
            self.find
                .set_message("Could not start the search".to_owned(), true);
            return;
        }
        self.replace_in_files = Stage::Counting {
            rx,
            cancel,
            files: 0,
            query,
            replacement: self.find.replacement.clone(),
            folder,
        };
    }

    /// Per frame: the count's progress, the question, and the writing.
    pub(super) fn replace_in_files_frame(&mut self, ui: &mut Ui) {
        match std::mem::take(&mut self.replace_in_files) {
            Stage::Idle => {}
            Stage::Counting {
                rx,
                cancel,
                mut files,
                query,
                replacement,
                folder,
            } => {
                let mut next = None;
                loop {
                    match rx.try_recv() {
                        Ok(CountMsg::Progress(n)) => files = n,
                        Ok(CountMsg::Done(counted)) => {
                            next = Some(Ok(counted));
                            break;
                        }
                        Ok(CountMsg::Failed(e)) => {
                            next = Some(Err(e));
                            break;
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            next = Some(Err("the search stopped".to_owned()));
                            break;
                        }
                    }
                }
                match next {
                    Some(Ok((on_disk, open, skipped)))
                        if on_disk.is_empty() && open.is_empty() && skipped.is_empty() =>
                    {
                        self.find.set_message("Not found", true);
                    }
                    Some(Ok((files, open, skipped))) => {
                        self.replace_in_files = Stage::Confirm(Plan {
                            query,
                            replacement,
                            folder,
                            files,
                            open,
                            skipped,
                        });
                    }
                    Some(Err(e)) => self.message = Some(format!("Replace in Files: {e}")),
                    None => {
                        let mut cancelled = false;
                        Modal::new(Id::new("ochrepad-replace-files-count")).show(ui.ctx(), |ui| {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(format!(
                                    "Counting the matches in {}… {files} files so far",
                                    folder.display()
                                ));
                            });
                            ui.add_space(8.0);
                            cancelled = ui.button("Cancel").clicked();
                        });
                        if cancelled {
                            cancel.store(true, Ordering::Relaxed);
                        } else {
                            self.replace_in_files = Stage::Counting {
                                rx,
                                cancel,
                                files,
                                query,
                                replacement,
                                folder,
                            };
                            ui.ctx()
                                .request_repaint_after(std::time::Duration::from_millis(50));
                        }
                    }
                }
            }
            Stage::Confirm(plan) => self.confirm_replace_in_files(ui, plan),
            Stage::Writing(w) => self.write_replace_in_files(ui, w),
        }
    }

    fn confirm_replace_in_files(&mut self, ui: &mut Ui, plan: Plan) {
        let on_disk: usize = plan.files.iter().map(|f| f.matches).sum();
        let open: usize = plan.open.iter().map(|f| f.1).sum();
        let total = on_disk + open;
        let files = plan.files.len() + plan.open.len();
        let resp = Modal::new(Id::new("ochrepad-replace-files")).show(ui.ctx(), |ui| {
            if files > 0 {
                ui.label(format!(
                    "Replace {total} matches in {files} files in {}?",
                    plan.folder.display()
                ));
            } else {
                ui.label(format!(
                    "No file in {} can be changed.",
                    plan.folder.display()
                ));
            }
            ui.add_space(4.0);
            if !plan.files.is_empty() {
                ui.label(format!(
                    "{} files are saved as they are done; that can't be undone.",
                    plan.files.len()
                ));
            }
            if !plan.open.is_empty() {
                ui.label(format!(
                    "{} open documents get an undo step each and are not saved.",
                    plan.open.len()
                ));
            }
            if !plan.skipped.is_empty() {
                ui.add_space(4.0);
                ui.label(format!(
                    "{} files with matches are left alone:",
                    plan.skipped.len()
                ));
                let lines: Vec<String> = plan
                    .skipped
                    .iter()
                    .map(|(p, why)| format!("{}: {why}", p.display()))
                    .collect();
                ui.label(egui::RichText::new(listed(&lines)).weak());
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if files > 0 && ui.button("Replace All Matches").clicked() {
                    Some(true)
                } else if ui
                    .button(if files > 0 { "Cancel" } else { "Close" })
                    .clicked()
                {
                    Some(false)
                } else {
                    None
                }
            })
            .inner
        });
        // Only a button or Escape answers (not a click beside the dialog):
        // this writes files.
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        let choice = if escape { Some(false) } else { resp.inner };
        match choice {
            None => self.replace_in_files = Stage::Confirm(plan),
            Some(false) => {}
            Some(true) => self.replace_in_files_start(plan),
        }
    }

    fn replace_in_files_start(&mut self, plan: Plan) {
        // Open documents: one undo step each, as Replace All in All Open
        // Documents does.
        let mut in_open = (0, 0);
        for (buffer, n) in &plan.open {
            let view = self.editor.views_of(*buffer).next().map(|v| v.id());
            if let Some(view) = view
                && self
                    .editor
                    .apply_unrecorded(
                        view,
                        Action::ReplaceAll {
                            query: plan.query.clone(),
                            replacement: plan.replacement.clone(),
                        },
                    )
                    .is_ok()
            {
                in_open.0 += n;
                in_open.1 += 1;
            }
        }
        let left: Vec<String> = plan
            .skipped
            .iter()
            .map(|(p, why)| format!("{}: {why}", p.display()))
            .collect();
        let total = plan.files.len();
        let (tx, rx) = sync_channel(MAX_IN_FLIGHT);
        let stop = Arc::new(AtomicBool::new(false));
        let halt = stop.clone();
        let (query, replacement, files) = (plan.query, plan.replacement, plan.files);
        let spawned = std::thread::Builder::new()
            .name("ochrepad-replace-files".into())
            .spawn(move || match Searcher::new(&query) {
                Ok(searcher) => rewrite(files, &searcher, &replacement, &halt, &tx),
                Err(_) => {
                    let _ = tx.send(WriteMsg::Done);
                }
            });
        if spawned.is_err() {
            self.message = Some("Replace in Files: could not start".to_owned());
            return;
        }
        self.replace_in_files = Stage::Writing(Writing {
            rx,
            stop,
            total,
            in_open,
            saved: 0,
            matches: 0,
            saving: 0,
            left,
            reading_done: false,
        });
    }

    fn write_replace_in_files(&mut self, ui: &mut Ui, mut w: Writing) {
        // New bytes go to the worker a few at a time.
        while w.saving < MAX_IN_FLIGHT && !w.reading_done {
            match w.rx.try_recv() {
                Ok(WriteMsg::Ready {
                    path,
                    bytes,
                    len,
                    modified,
                    matches,
                }) => {
                    let tag = self
                        .persist
                        .worker
                        .submit(ochrepad_fileio::worker::Job::Replace {
                            path: path.clone(),
                            bytes,
                            len,
                            modified,
                        });
                    self.persist
                        .pending
                        .insert(tag, PendingJob::ReplaceInFiles { path, matches });
                    w.saving += 1;
                }
                Ok(WriteMsg::Failed(path, e)) => w.left.push(format!("{}: {e}", path.display())),
                Ok(WriteMsg::Done) => w.reading_done = true,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    w.left
                        .push("the rest: the replacing stopped unexpectedly".to_owned());
                    w.reading_done = true;
                }
            }
        }
        // Saves the worker finished (counted in `handle_done`).
        for (ok, n, error) in self.replace_saves.drain(..) {
            w.saving = w.saving.saturating_sub(1);
            if ok {
                w.saved += 1;
                w.matches += n;
            } else if let Some(e) = error {
                w.left.push(e);
            }
        }
        if w.reading_done && w.saving == 0 {
            self.finish_replace_in_files(w);
            return;
        }
        let mut stop = false;
        let done = w.saved + w.left.len();
        Modal::new(Id::new("ochrepad-replace-files-writing")).show(ui.ctx(), |ui| {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("Replacing… {done} of {} files", w.total));
            });
            ui.add_space(8.0);
            stop = ui
                .add_enabled(!w.stop.load(Ordering::Relaxed), egui::Button::new("Stop"))
                .on_hover_text("Files already written stay so; the others are left alone")
                .clicked();
        });
        if stop {
            w.stop.store(true, Ordering::Relaxed);
        }
        self.replace_in_files = Stage::Writing(w);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(20));
    }

    fn finish_replace_in_files(&mut self, w: Writing) {
        let (open_matches, open_docs) = w.in_open;
        let stopped = w.stop.load(Ordering::Relaxed);
        let mut text = format!(
            "Replaced {} matches in {} files",
            w.matches + open_matches,
            w.saved + open_docs
        );
        if open_docs > 0 {
            text.push_str(&format!(" ({open_docs} open, not saved yet)"));
        }
        if stopped {
            text.push_str("; stopped before the rest");
        }
        if w.left.is_empty() {
            self.note_text(text.clone());
            self.find.set_message(text, false);
        } else {
            self.message = Some(format!(
                "{text}.\nThese files were left as they were:\n{}",
                listed(&w.left)
            ));
        }
    }

    /// The worker finished saving `path` (with `matches` replaced) for
    /// Replace in Files.
    pub(super) fn replace_in_files_saved(
        &mut self,
        path: &Path,
        matches: usize,
        result: Result<(), String>,
    ) {
        let error = result.err().map(|e| {
            if e.starts_with(&path.display().to_string()) {
                e
            } else {
                format!("{}: {e}", path.display())
            }
        });
        self.replace_saves.push((error.is_none(), matches, error));
    }

    /// Whether Replace in Files is counting, asking or writing.
    pub fn replacing_in_files(&self) -> bool {
        !matches!(self.replace_in_files, Stage::Idle)
    }

    /// Whether Replace in Files is writing files (exiting waits for it).
    pub fn writing_in_files(&self) -> bool {
        matches!(self.replace_in_files, Stage::Writing(_))
    }
}
