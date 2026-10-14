//! Replace in Files, as in Notepad++'s Find in Files: replaces every match
//! of the find panel's pattern in the files of a folder. First the files
//! are searched on a thread and the matches counted, and the user is asked.
//! Then documents open in ochrepad get a Replace All each (an undo step,
//! left unsaved), and the other files are rewritten on a thread and saved by
//! the worker, atomically, with their own encoding, byte order mark and
//! line endings. Binary files and files over 50 MB are left alone, as Find
//! in Files leaves them.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};

use egui::{Id, Modal, Ui};
use ochrepad_core::{
    Action, BufferId, ByteOffset, ChangeSet, Document, Edit, FileMeta, SearchQuery, Searcher,
};
use ochrepad_fileio::tree::WalkOptions;

use super::OchrepadApp;
use super::autosave::PendingJob;
use crate::search_results::MAX_FILE_BYTES;

/// What the count found: files on disk and open documents with matches.
pub(super) struct Plan {
    query: SearchQuery,
    replacement: String,
    folder: PathBuf,
    files: Vec<(PathBuf, usize)>,
    open: Vec<(BufferId, usize)>,
}

pub(super) enum CountMsg {
    Progress(usize),
    Done(Vec<(PathBuf, usize)>, Vec<(BufferId, usize)>),
    Failed(String),
}

pub(super) enum WriteMsg {
    /// A file's new text, to be saved, and how many matches were replaced.
    Ready(PathBuf, Document, FileMeta, usize),
    /// A file that could not be read or replaced.
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
    Writing {
        rx: Receiver<WriteMsg>,
        /// Matches replaced in open documents, and their number.
        in_open: (usize, usize),
        /// Files rewritten so far and their matches; files still being
        /// saved; failures.
        saved: usize,
        saving: usize,
        matches: usize,
        failed: Vec<String>,
        reading_done: bool,
    },
}

/// The edits replacing every match of `searcher` in `doc` with `replacement`
/// (expanded per match).
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
                let path = b.meta.path.as_deref()?;
                let doc = (!b.read_only).then(|| b.doc().clone());
                Some((super::canonical(path), b.id(), doc))
            })
            .collect();
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let dir = folder.clone();
        let spawned = std::thread::Builder::new()
            .name("ochrepad-replace-count".into())
            .spawn(move || {
                let walk = match ochrepad_fileio::tree::walk_files(&dir, &options) {
                    Ok(w) => w,
                    Err(e) => {
                        let _ = tx.send(CountMsg::Failed(e));
                        return;
                    }
                };
                let (mut files, mut in_open, mut seen) = (Vec::new(), Vec::new(), 0usize);
                for path in walk {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    seen += 1;
                    if seen % 100 == 0 && tx.send(CountMsg::Progress(seen)).is_err() {
                        return;
                    }
                    let canonical = super::canonical(&path);
                    if let Some((_, id, doc)) = open.iter().find(|(p, _, _)| *p == canonical) {
                        if let Some(doc) = doc {
                            let n = searcher.matches(doc, ByteOffset(0)..doc.len()).count();
                            if n > 0 {
                                in_open.push((*id, n));
                            }
                        }
                        continue;
                    }
                    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_FILE_BYTES) {
                        continue;
                    }
                    let Ok(decoded) = ochrepad_fileio::load(&path) else {
                        continue;
                    };
                    let doc = Document::new(&decoded.text);
                    let n = searcher.matches(&doc, ByteOffset(0)..doc.len()).count();
                    if n > 0 {
                        files.push((path, n));
                    }
                }
                let _ = tx.send(CountMsg::Done(files, in_open));
            });
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
                        Ok(CountMsg::Done(on_disk, open)) => {
                            next = Some(Ok((on_disk, open)));
                            break;
                        }
                        Ok(CountMsg::Failed(e)) => {
                            next = Some(Err(e));
                            break;
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            next = Some(Err("the search stopped".to_owned()));
                            break;
                        }
                    }
                }
                match next {
                    Some(Ok((on_disk, open))) if on_disk.is_empty() && open.is_empty() => {
                        self.find.set_message("Not found", true);
                    }
                    Some(Ok((on_disk, open))) => {
                        self.replace_in_files = Stage::Confirm(Plan {
                            query,
                            replacement,
                            folder,
                            files: on_disk,
                            open,
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
            Stage::Writing {
                rx,
                in_open,
                mut saved,
                mut saving,
                mut matches,
                mut failed,
                mut reading_done,
            } => {
                while let Ok(msg) = rx.try_recv() {
                    match msg {
                        WriteMsg::Ready(path, doc, meta, n) => {
                            let tag =
                                self.persist
                                    .worker
                                    .submit(ochrepad_fileio::worker::Job::Save {
                                        path: path.clone(),
                                        doc,
                                        meta,
                                    });
                            self.persist
                                .pending
                                .insert(tag, PendingJob::ReplaceInFiles { path, matches: n });
                            saving += 1;
                        }
                        WriteMsg::Failed(path, e) => {
                            failed.push(format!("{}: {e}", path.display()));
                        }
                        WriteMsg::Done => reading_done = true,
                    }
                }
                // Saves the worker finished (counted in `handle_done`).
                for (ok, n, error) in self.replace_saves.drain(..) {
                    saving = saving.saturating_sub(1);
                    if ok {
                        saved += 1;
                        matches += n;
                    } else if let Some(e) = error {
                        failed.push(e);
                    }
                }
                if reading_done && saving == 0 {
                    let (open_matches, open_docs) = in_open;
                    let mut text = format!(
                        "Replaced {} matches in {} files",
                        matches + open_matches,
                        saved + open_docs
                    );
                    if open_docs > 0 {
                        text.push_str(&format!(" ({open_docs} open, not saved yet)"));
                    }
                    if failed.is_empty() {
                        self.find.set_message(text, false);
                    } else {
                        self.message = Some(format!(
                            "{text}.\nThese files were left as they were:\n{}",
                            failed.join("\n")
                        ));
                    }
                } else {
                    self.replace_in_files = Stage::Writing {
                        rx,
                        in_open,
                        saved,
                        saving,
                        matches,
                        failed,
                        reading_done,
                    };
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(20));
                }
            }
        }
    }

    fn confirm_replace_in_files(&mut self, ui: &mut Ui, plan: Plan) {
        let on_disk: usize = plan.files.iter().map(|f| f.1).sum();
        let open: usize = plan.open.iter().map(|f| f.1).sum();
        let total = on_disk + open;
        let files = plan.files.len() + plan.open.len();
        let resp = Modal::new(Id::new("ochrepad-replace-files")).show(ui.ctx(), |ui| {
            ui.label(format!(
                "Replace {total} matches in {files} files in {}?",
                plan.folder.display()
            ));
            ui.add_space(4.0);
            if !plan.files.is_empty() {
                ui.label(format!(
                    "{} files are saved straight away; that can't be undone.",
                    plan.files.len()
                ));
            }
            if !plan.open.is_empty() {
                ui.label(format!(
                    "{} open documents get an undo step each and are not saved.",
                    plan.open.len()
                ));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Replace All Matches").clicked() {
                    Some(true)
                } else if ui.button("Cancel").clicked() {
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
        let (tx, rx) = channel();
        let (query, replacement, files) = (plan.query, plan.replacement, plan.files);
        let spawned = std::thread::Builder::new()
            .name("ochrepad-replace-files".into())
            .spawn(move || {
                let Ok(searcher) = Searcher::new(&query) else {
                    let _ = tx.send(WriteMsg::Done);
                    return;
                };
                for (path, _) in files {
                    let msg = match ochrepad_fileio::load(&path) {
                        Ok(decoded) => {
                            let doc = Document::new(&decoded.text);
                            let (new, n) = replace_all(&doc, &searcher, &replacement);
                            if n == 0 {
                                continue;
                            }
                            WriteMsg::Ready(path, new, decoded.meta, n)
                        }
                        Err(e) => WriteMsg::Failed(path, e.to_string()),
                    };
                    if tx.send(msg).is_err() {
                        return;
                    }
                }
                let _ = tx.send(WriteMsg::Done);
            });
        if spawned.is_err() {
            self.message = Some("Replace in Files: could not start".to_owned());
            return;
        }
        self.replace_in_files = Stage::Writing {
            rx,
            in_open,
            saved: 0,
            saving: 0,
            matches: 0,
            failed: Vec::new(),
            reading_done: false,
        };
    }

    /// The worker finished saving `path` (with `matches` replaced) for
    /// Replace in Files.
    pub(super) fn replace_in_files_saved(
        &mut self,
        path: &Path,
        matches: usize,
        result: Result<(), String>,
    ) {
        let error = result.err().map(|e| format!("{}: {e}", path.display()));
        self.replace_saves.push((error.is_none(), matches, error));
    }

    /// Whether Replace in Files is counting, asking or writing.
    pub fn replacing_in_files(&self) -> bool {
        !matches!(self.replace_in_files, Stage::Idle)
    }
}
