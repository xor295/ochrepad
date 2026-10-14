//! Searching several documents at once (PLAN_v0.2.0 N6) and the Search
//! Results panel at the bottom of the window.
//!
//! A search runs on a background thread over clones of the documents (rope
//! clones are cheap) and sends each document's matches back as it finishes,
//! so a big file never holds up a frame. Results keep the revision they were
//! found at; a result in a document edited since is looked up again on its
//! line when clicked.
//!
//! Find in Files (N8) walks a folder on the same kind of thread, reading
//! each file with `fileio::load` (encodings handled; binary and
//! undecodable files skipped) or taking an open document's text instead.

use std::collections::{HashSet, VecDeque};
use std::ffi::OsString;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::text::LayoutJob;
use egui::{
    Color32, FontId, Pos2, Response, ScrollArea, Sense, Stroke, TextFormat, Ui, Vec2, WidgetInfo,
    WidgetType,
};
use ochrepad_core::{BufferId, ByteOffset, Document, SearchQuery, Searcher, SelectionSet};

/// Matches kept per search; the rest are counted as cut off.
pub const MAX_MATCHES: usize = 10_000;
/// Searches kept in the panel.
const MAX_SEARCHES: usize = 10;
/// Longer lines are shown as a window around their first match.
const MAX_LINE_BYTES: usize = 300;
/// Find in Files skips files larger than this (and counts them).
pub const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;
/// Find in Files reports its progress every this many files.
const PROGRESS_EVERY: usize = 64;

/// Where a result is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Buffer(BufferId),
    /// A file that was not open when it was searched.
    Path(PathBuf),
    /// A large file open in a viewer; a hit's `start` and `end` are offsets
    /// in the file.
    Viewer(crate::viewer::ViewerId),
}

/// An open document Find in Files should search instead of its file.
pub struct OpenDoc {
    pub path: PathBuf,
    pub source: Source,
    pub doc: Document,
    pub revision: u64,
}

/// One document to search.
pub struct Job {
    pub source: Source,
    pub name: String,
    pub doc: Document,
    pub revision: u64,
    /// Search only these selections ("in selection").
    pub selections: Option<SelectionSet>,
}

/// One line with matches.
#[derive(Clone, Debug)]
pub struct Hit {
    /// 0-based line.
    pub line: usize,
    /// The line's text (or a window of a long line) and the matched byte
    /// ranges within it.
    pub text: String,
    pub ranges: Vec<Range<usize>>,
    /// The first match on the line, and its byte column in the line.
    pub start: ByteOffset,
    pub end: ByteOffset,
    pub column: usize,
}

pub struct FileHits {
    pub source: Source,
    pub name: String,
    pub revision: u64,
    pub hits: Vec<Hit>,
    pub matches: usize,
    pub expanded: bool,
}

pub struct Search {
    pub query: SearchQuery,
    /// "in 3 open documents", "in doc.md".
    pub scope: String,
    pub files: Vec<FileHits>,
    pub matches: usize,
    /// More matches were found than are kept.
    pub truncated: bool,
    pub done: bool,
    /// Stopped before it finished.
    pub stopped: bool,
    pub expanded: bool,
    /// Find in Files: files looked at, and files skipped for their size.
    pub files_searched: Option<usize>,
    pub skipped_large: usize,
    /// The search could not run (a bad filter, an unreadable folder).
    pub error: Option<String>,
    receiver: Option<Receiver<Msg>>,
    cancel: Arc<AtomicBool>,
}

enum Msg {
    File(FileHits),
    Progress {
        files: usize,
    },
    Done {
        truncated: bool,
        skipped_large: usize,
    },
    Failed(String),
}

/// What a click in the panel asks the app to do.
pub struct GoTo {
    pub source: Source,
    pub revision: u64,
    pub hit: Hit,
    pub query: SearchQuery,
}

#[derive(Default)]
pub struct SearchResults {
    pub open: bool,
    /// Newest first.
    pub searches: VecDeque<Search>,
}

impl SearchResults {
    /// Starts searching `jobs` for `query` in the background, as the newest
    /// search; older ones are collapsed.
    pub fn start(&mut self, query: SearchQuery, scope: String, jobs: Vec<Job>) {
        self.spawn(query, scope, false, None, move |q, c, tx| {
            search(q, jobs, c, tx)
        });
    }

    /// Starts a search that `work` does on the background thread (the
    /// viewer's, over a file too large to open), and lists what it returns:
    /// the results and whether more matches were found than are kept.
    /// `cancel` is the flag Stop and Clear set; the work should watch it
    /// (a file source can stop its reads on it).
    pub fn start_external(
        &mut self,
        query: SearchQuery,
        scope: String,
        cancel: Arc<AtomicBool>,
        work: impl FnOnce(Arc<AtomicBool>) -> Result<(Vec<FileHits>, bool), String> + Send + 'static,
    ) {
        self.spawn(
            query,
            scope,
            false,
            Some(cancel),
            move |_, cancel, tx| match work(cancel) {
                Ok((files, truncated)) => {
                    for f in files {
                        if tx.send(Msg::File(f)).is_err() {
                            return;
                        }
                    }
                    let _ = tx.send(Msg::Done {
                        truncated,
                        skipped_large: 0,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Msg::Failed(e));
                }
            },
        );
    }

    /// Starts Find in Files over `folder`.
    pub fn start_in_files(
        &mut self,
        query: SearchQuery,
        folder: PathBuf,
        options: ochrepad_fileio::tree::WalkOptions,
        open: Vec<OpenDoc>,
    ) {
        let scope = format!("in {}", folder.display());
        self.spawn(query, scope, true, None, move |q, c, tx| {
            search_files(q, &folder, &options, open, c, tx)
        });
    }

    fn spawn(
        &mut self,
        query: SearchQuery,
        scope: String,
        in_files: bool,
        cancel: Option<Arc<AtomicBool>>,
        work: impl FnOnce(SearchQuery, Arc<AtomicBool>, Sender<Msg>) + Send + 'static,
    ) {
        for s in &mut self.searches {
            s.expanded = false;
        }
        let cancel = cancel.unwrap_or_default();
        let (tx, rx) = channel();
        let (q, c) = (query.clone(), cancel.clone());
        std::thread::Builder::new()
            .name("ochrepad-search".into())
            .spawn(move || work(q, c, tx))
            .ok();
        self.searches.push_front(Search {
            query,
            scope,
            files: Vec::new(),
            matches: 0,
            truncated: false,
            done: false,
            stopped: false,
            expanded: true,
            files_searched: in_files.then_some(0),
            skipped_large: 0,
            error: None,
            receiver: Some(rx),
            cancel,
        });
        while self.searches.len() > MAX_SEARCHES {
            if let Some(old) = self.searches.pop_back() {
                old.cancel.store(true, Ordering::Relaxed);
            }
        }
        self.open = true;
    }

    /// Takes in results that arrived; returns whether a search is running.
    pub fn poll(&mut self) -> bool {
        let mut running = false;
        for s in &mut self.searches {
            let Some(rx) = &s.receiver else { continue };
            loop {
                let msg = match rx.try_recv() {
                    Ok(msg) => msg,
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    // The search thread ended without saying it was done
                    // (it failed).
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        if !s.done {
                            s.error = Some("The search stopped unexpectedly".into());
                            s.done = true;
                        }
                        break;
                    }
                };
                match msg {
                    Msg::File(f) => {
                        s.matches += f.matches;
                        s.files.push(f);
                    }
                    Msg::Progress { files } => s.files_searched = Some(files),
                    Msg::Done {
                        truncated,
                        skipped_large,
                    } => {
                        s.truncated = truncated;
                        s.skipped_large = skipped_large;
                        s.done = true;
                    }
                    Msg::Failed(e) => {
                        s.error = Some(e);
                        s.done = true;
                    }
                }
            }
            if s.done {
                s.receiver = None;
            } else {
                running = true;
            }
        }
        running
    }

    /// Stops the searches still running; their results so far stay.
    pub fn stop(&mut self) {
        for s in self.searches.iter_mut().filter(|s| !s.done) {
            s.cancel.store(true, Ordering::Relaxed);
            s.done = true;
            s.stopped = true;
            s.receiver = None;
        }
    }

    pub fn clear(&mut self) {
        for s in self.searches.drain(..) {
            s.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// The panel body. Returns a result the user picked.
    pub fn ui(&mut self, ui: &mut Ui, match_bg: Color32) -> Option<GoTo> {
        let mut go = None;
        ui.horizontal(|ui| {
            ui.strong("Search Results");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    self.open = false;
                }
                if ui.button("Clear").clicked() {
                    self.clear();
                }
                if self.searches.iter().any(|s| !s.done) && ui.button("Stop").clicked() {
                    self.stop();
                }
            });
        });
        ui.separator();
        if self.searches.is_empty() {
            ui.weak("Use Find All in the find panel (Ctrl+F) to list matches here.");
            return None;
        }
        // Only the rows on screen are laid out, so long lists stay fast.
        let rows = self.rows();
        let font = FontId::monospace(13.0);
        let row_h = ui.fonts_mut(|f| f.row_height(&font)) + 6.0;
        let ui_font = egui::TextStyle::Body.resolve(ui.style());
        let text_color = ui.visuals().text_color();
        ScrollArea::vertical()
            .auto_shrink(false)
            .show_rows(ui, row_h, rows.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for row in &rows[range] {
                    match *row {
                        Row::Search(i) => {
                            let s = &mut self.searches[i];
                            let mut notes = Vec::new();
                            if let Some(n) = s.files_searched {
                                notes.push(format!("{} searched", files(n)));
                            }
                            if !s.done {
                                notes.push("searching…".to_owned());
                            }
                            if s.stopped {
                                notes.push("stopped".to_owned());
                            }
                            if s.truncated {
                                notes.push(format!("the first {MAX_MATCHES} are listed"));
                            }
                            if s.skipped_large > 0 {
                                notes.push(format!(
                                    "{} over {} MB skipped",
                                    files(s.skipped_large),
                                    MAX_FILE_BYTES / (1024 * 1024)
                                ));
                            }
                            if let Some(e) = &s.error {
                                notes.push(e.clone());
                            }
                            let status = if notes.is_empty() {
                                String::new()
                            } else {
                                format!(" ({})", notes.join(", "))
                            };
                            let text = format!(
                                "\"{}\" {}: {} matches in {} documents{status}",
                                s.query.pattern,
                                s.scope,
                                s.matches,
                                s.files.len(),
                            );
                            let mut job = LayoutJob::default();
                            let mut format = TextFormat::simple(ui_font.clone(), text_color);
                            format.color = ui.visuals().strong_text_color();
                            job.append(&text, 0.0, format);
                            if result_row(ui, row_h, 0.0, Some(s.expanded), job).clicked() {
                                s.expanded = !s.expanded;
                            }
                        }
                        Row::File(i, j) => {
                            let f = &mut self.searches[i].files[j];
                            let text = format!("{} ({} matches)", f.name, f.matches);
                            let job =
                                LayoutJob::simple_singleline(text, ui_font.clone(), text_color);
                            if result_row(ui, row_h, 16.0, Some(f.expanded), job).clicked() {
                                f.expanded = !f.expanded;
                            }
                        }
                        Row::Hit(i, j, k) => {
                            let s = &self.searches[i];
                            let f = &s.files[j];
                            let hit = &f.hits[k];
                            let job = hit_job(ui, hit, &font, match_bg);
                            if result_row(ui, row_h, 32.0, None, job).clicked() {
                                go = Some(GoTo {
                                    source: f.source.clone(),
                                    revision: f.revision,
                                    hit: hit.clone(),
                                    query: s.query.clone(),
                                });
                            }
                        }
                    }
                }
            });
        go
    }

    fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for (i, s) in self.searches.iter().enumerate() {
            rows.push(Row::Search(i));
            if !s.expanded {
                continue;
            }
            for (j, f) in s.files.iter().enumerate() {
                rows.push(Row::File(i, j));
                if f.expanded {
                    rows.extend((0..f.hits.len()).map(|k| Row::Hit(i, j, k)));
                }
            }
        }
        rows
    }
}

#[derive(Clone, Copy)]
enum Row {
    Search(usize),
    File(usize, usize),
    Hit(usize, usize, usize),
}

/// "1 file", "3 files".
fn files(n: usize) -> String {
    if n == 1 {
        "1 file".to_owned()
    } else {
        format!("{n} files")
    }
}

/// One clickable row of the panel: `indent`, an open/closed triangle for
/// headers (`arrow`), then `job`. Its accessibility label is the text.
fn result_row(ui: &mut Ui, h: f32, indent: f32, arrow: Option<bool>, job: LayoutJob) -> Response {
    let label = job.text.clone();
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &label));
    let painter = ui.painter();
    let visuals = ui.visuals();
    if response.hovered() || response.has_focus() {
        painter.rect_filled(rect, 3.0, visuals.widgets.hovered.weak_bg_fill);
    }
    let x = rect.min.x + 4.0 + indent;
    if let Some(open) = arrow {
        let c = Pos2::new(x + 5.0, rect.center().y);
        let s = 3.5;
        let points = if open {
            vec![
                c + Vec2::new(-s, -s / 2.0),
                c + Vec2::new(0.0, s / 2.0 + 1.0),
                c + Vec2::new(s, -s / 2.0),
            ]
        } else {
            vec![
                c + Vec2::new(-s / 2.0, -s),
                c + Vec2::new(s / 2.0 + 1.0, 0.0),
                c + Vec2::new(-s / 2.0, s),
            ]
        };
        painter.line(points, Stroke::new(1.5, visuals.text_color()));
    }
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let pos = Pos2::new(x + 16.0, rect.center().y - galley.size().y / 2.0);
    painter.galley(pos, galley, visuals.text_color());
    response
}

/// "Line 12:  text" with the matches tinted.
fn hit_job(ui: &Ui, hit: &Hit, font: &FontId, match_bg: Color32) -> LayoutJob {
    let color = ui.visuals().text_color();
    let weak = ui.visuals().weak_text_color();
    let plain = TextFormat::simple(font.clone(), color);
    let mut job = LayoutJob::default();
    job.append(
        &format!("Line {}: ", hit.line + 1),
        0.0,
        TextFormat::simple(font.clone(), weak),
    );
    let text = hit.text.trim_end();
    let lead = text.len() - text.trim_start().len();
    let mut at = lead;
    for r in &hit.ranges {
        let (a, b) = (r.start.max(at).min(text.len()), r.end.min(text.len()));
        if a >= b {
            continue;
        }
        job.append(&text[at..a], 0.0, plain.clone());
        let mut m = plain.clone();
        m.background = match_bg;
        job.append(&text[a..b], 0.0, m);
        at = b;
    }
    job.append(&text[at.min(text.len())..], 0.0, plain);
    job
}

/// The background search: every job in turn, one message per document with
/// matches, then `Done`.
fn search(query: SearchQuery, jobs: Vec<Job>, cancel: Arc<AtomicBool>, tx: Sender<Msg>) {
    let Ok(searcher) = Searcher::new(&query) else {
        let _ = tx.send(Msg::Failed("invalid pattern".into()));
        return;
    };
    let mut budget = Budget::default();
    for job in jobs {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let Some(file) = budget.search(&searcher, &job.doc, job.selections.as_ref(), |found| {
            FileHits {
                hits: hits(&job.doc, found),
                matches: found.len(),
                source: job.source.clone(),
                name: job.name.clone(),
                revision: job.revision,
                expanded: true,
            }
        }) else {
            if budget.left == 0 {
                break;
            }
            continue;
        };
        if tx.send(Msg::File(file)).is_err() {
            return;
        }
    }
    let _ = tx.send(Msg::Done {
        truncated: budget.truncated,
        skipped_large: 0,
    });
}

/// The matches still to be kept in one search.
struct Budget {
    left: usize,
    truncated: bool,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            left: MAX_MATCHES,
            truncated: false,
        }
    }
}

impl Budget {
    /// Searches `doc` (only `selections` when given); with matches, makes
    /// the results with `make`.
    fn search(
        &mut self,
        searcher: &Searcher,
        doc: &Document,
        selections: Option<&SelectionSet>,
        make: impl FnOnce(&[Range<ByteOffset>]) -> FileHits,
    ) -> Option<FileHits> {
        if self.left == 0 {
            self.truncated = true;
            return None;
        }
        let found: Vec<Range<ByteOffset>> = match selections {
            Some(sels) => searcher.find_in(doc, sels),
            None => searcher
                .matches(doc, ByteOffset(0)..doc.len())
                .take(self.left + 1)
                .collect(),
        };
        if found.len() > self.left {
            self.truncated = true;
        }
        let found = &found[..found.len().min(self.left)];
        self.left -= found.len();
        (!found.is_empty()).then(|| make(found))
    }
}

/// Find in Files: every file `options` selects under `folder`, in order.
fn search_files(
    query: SearchQuery,
    folder: &Path,
    options: &ochrepad_fileio::tree::WalkOptions,
    open: Vec<OpenDoc>,
    cancel: Arc<AtomicBool>,
    tx: Sender<Msg>,
) {
    let Ok(searcher) = Searcher::new(&query) else {
        let _ = tx.send(Msg::Failed("invalid pattern".into()));
        return;
    };
    let walk = match ochrepad_fileio::tree::walk_files(folder, options) {
        Ok(w) => w,
        Err(e) => {
            let _ = tx.send(Msg::Failed(e));
            return;
        }
    };
    // Open documents by canonical path; only files with an open document's
    // name are canonicalized while walking.
    let open_names: HashSet<OsString> = open
        .iter()
        .filter_map(|o| o.path.file_name().map(OsString::from))
        .collect();
    let open: Vec<(PathBuf, OpenDoc)> = open
        .into_iter()
        .map(|o| {
            (
                std::fs::canonicalize(&o.path).unwrap_or_else(|_| o.path.clone()),
                o,
            )
        })
        .collect();
    let mut budget = Budget::default();
    let (mut files, mut skipped_large) = (0, 0);
    for path in walk {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        if budget.left == 0 {
            budget.truncated = true;
            break;
        }
        files += 1;
        if files % PROGRESS_EVERY == 0 && tx.send(Msg::Progress { files }).is_err() {
            return;
        }
        let name = path
            .strip_prefix(folder)
            .unwrap_or(&path)
            .display()
            .to_string();
        let opened = path
            .file_name()
            .filter(|n| open_names.contains(*n))
            .and_then(|_| std::fs::canonicalize(&path).ok())
            .and_then(|c| open.iter().find(|(p, _)| *p == c))
            .map(|(_, o)| o);
        let loaded;
        let (doc, source, revision) = match opened {
            Some(o) => (&o.doc, o.source.clone(), o.revision),
            None => {
                if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_FILE_BYTES) {
                    skipped_large += 1;
                    continue;
                }
                // Binary, undecodable and unreadable files are skipped.
                let Ok(decoded) = ochrepad_fileio::load(&path) else {
                    continue;
                };
                loaded = Document::new(&decoded.text);
                (&loaded, Source::Path(path.clone()), 0)
            }
        };
        let file = budget.search(&searcher, doc, None, |found| FileHits {
            hits: hits(doc, found),
            matches: found.len(),
            source,
            name,
            revision,
            expanded: true,
        });
        if let Some(file) = file
            && tx.send(Msg::File(file)).is_err()
        {
            return;
        }
    }
    let _ = tx.send(Msg::Progress { files });
    let _ = tx.send(Msg::Done {
        truncated: budget.truncated,
        skipped_large,
    });
}

/// Groups matches by line.
fn hits(doc: &Document, found: &[Range<ByteOffset>]) -> Vec<Hit> {
    let mut out: Vec<Hit> = Vec::new();
    for m in found {
        let line = doc.line_of(m.start);
        let line_start = doc.line_start(line).0;
        let line_end = doc.line_end(line).0;
        let range = m.start.0 - line_start..m.end.0.min(line_end) - line_start;
        if let Some(last) = out.last_mut().filter(|h| h.line == line) {
            last.ranges.push(range);
            continue;
        }
        out.push(Hit {
            line,
            text: doc.slice(ByteOffset(line_start)..ByteOffset(line_end)),
            ranges: vec![range.clone()],
            start: m.start,
            end: m.end,
            column: range.start,
        });
    }
    for h in &mut out {
        shorten(h);
    }
    out
}

/// Cuts a long line down to a window around its first match.
pub(crate) fn shorten(h: &mut Hit) {
    if h.text.len() <= MAX_LINE_BYTES {
        return;
    }
    let floor = |s: &str, mut i: usize| {
        while !s.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let first = h.ranges[0].start;
    let from = floor(&h.text, first.saturating_sub(MAX_LINE_BYTES / 4));
    let to = floor(&h.text, (from + MAX_LINE_BYTES).min(h.text.len()));
    let head = if from > 0 { "…" } else { "" };
    let tail = if to < h.text.len() { "…" } else { "" };
    let shift = head.len();
    h.text = format!("{head}{}{tail}", &h.text[from..to]);
    h.ranges = h
        .ranges
        .iter()
        .filter(|r| r.end > from && r.start < to)
        .map(|r| r.start.max(from) - from + shift..r.end.min(to) - from + shift)
        .collect();
}

#[cfg(test)]
mod tests {
    use super::*;
    use ochrepad_core::SearchOptions;

    fn run(pattern: &str, docs: &[&str]) -> Search {
        let mut r = SearchResults::default();
        let jobs = docs
            .iter()
            .enumerate()
            .map(|(i, t)| Job {
                source: Source::Buffer(BufferId(i as u64)),
                name: format!("doc {i}"),
                doc: Document::new(t),
                revision: 0,
                selections: None,
            })
            .collect();
        let query = SearchQuery::new(pattern, SearchOptions::default());
        r.start(query, "test".into(), jobs);
        while r.poll() {
            std::thread::yield_now();
        }
        r.searches.pop_front().unwrap()
    }

    #[test]
    fn matches_are_grouped_by_document_and_line() {
        let s = run("ab", &["ab ab\nx\nab", "none", "cab"]);
        assert!(s.done && !s.truncated);
        assert_eq!(s.matches, 4);
        let names: Vec<&str> = s.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["doc 0", "doc 2"]);
        let lines: Vec<(usize, Vec<(usize, usize)>)> = s.files[0]
            .hits
            .iter()
            .map(|h| (h.line, h.ranges.iter().map(|r| (r.start, r.end)).collect()))
            .collect();
        assert_eq!(lines, vec![(0, vec![(0, 2), (3, 5)]), (2, vec![(0, 2)])]);
        assert_eq!(s.files[1].hits[0].column, 1);
    }

    #[test]
    fn long_lines_show_a_window_and_counts_stop_at_the_limit() {
        let long = format!("{}needle{}", "x".repeat(1000), "y".repeat(1000));
        let s = run("needle", &[&long]);
        let h = &s.files[0].hits[0];
        assert!(h.text.len() < 400 && h.text.starts_with('…') && h.text.ends_with('…'));
        assert_eq!(&h.text[h.ranges[0].clone()], "needle");

        let many = "a".repeat(MAX_MATCHES + 5);
        let s = run("a", &[&many]);
        assert!(s.truncated);
        assert_eq!(s.matches, MAX_MATCHES);
    }
}
