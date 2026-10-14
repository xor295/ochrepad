//! The viewer for files too large to edit (PLAN_v0.3.0 P7, section 3.2): a
//! read-only widget over a [`BigFile`]. It draws the lines on screen with
//! the same [`LineLayout`] and [`paint_line_text`] the editor uses (one
//! layout for painting and hit testing), scrolls by itself (no
//! `ScrollArea`), and supports selecting with the mouse and keyboard,
//! copying (up to [`MAX_COPY`]), Go to Line and searching. Searches run on a
//! background thread over the file read in pieces; the index of lines grows
//! in the background too, and the scrollbar and line count follow it.
//!
//! The lines shown are a [`Shown`] list: every line, the lines a filter
//! keeps, or every line but those it hides (PLAN_v0.3.0 P8). A filter's
//! lines are found by a background search that sends them as it goes, so
//! the view fills in while a large file is searched, and goes on with what
//! the file gains when it grows.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, channel};

use egui::{
    Align2, Color32, CursorIcon, Event, EventFilter, FontId, Id, Key, Modifiers, Pos2, Rect,
    Response, Sense, Stroke, Ui, Vec2, WidgetInfo, WidgetType,
};
use ochrepad_core::{SearchQuery, Searcher};
use ochrepad_fileio::bigfile::{BigFile, Change};
use unicode_segmentation::UnicodeSegmentation;

use crate::editor_view::{Look, paint_line_text};
use crate::layout::LineLayout;

/// Bytes of a line that are shown; a longer line is cut, with a marker.
pub const MAX_LINE_BYTES: u64 = 10_000;
/// The most text Copy puts on the clipboard.
pub const MAX_COPY: u64 = 64 * 1024 * 1024;

const BLINK_HALF_PERIOD: f64 = 0.53;
const SCROLLBAR_W: f32 = 12.0;
const TEXT_PAD: f32 = 4.0;
const MULTI_CLICK_TIME: f64 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ViewerId(pub u64);

/// Which lines the viewer shows.
#[derive(Clone, Debug, Default)]
pub enum Shown {
    /// Every line.
    #[default]
    All,
    /// These lines, in order (a filter showing what matches).
    Lines(Arc<Vec<u64>>),
    /// Every line but these, in order (a filter hiding what matches), up to
    /// line `decided` while the search is still going (all lines once it
    /// is done: `None`).
    AllExcept {
        excluded: Arc<Vec<u64>>,
        decided: Option<u64>,
    },
}

impl Shown {
    /// Rows to show, given the lines known so far.
    pub fn rows(&self, lines: u64) -> u64 {
        match self {
            Shown::All => lines,
            Shown::Lines(v) => v.len() as u64,
            Shown::AllExcept { excluded, decided } => {
                let limit = decided.map_or(lines, |d| d.min(lines));
                limit - excluded.partition_point(|&l| l < limit) as u64
            }
        }
    }

    /// The line on row `row`.
    pub fn line(&self, row: u64) -> Option<u64> {
        match self {
            Shown::All => Some(row),
            Shown::Lines(v) => v.get(row as usize).copied(),
            Shown::AllExcept { excluded, .. } => {
                // Row `row` is line `row + j`, where `j` lines before it are
                // excluded: the first `j` with `excluded[j] - j > row`.
                let (mut lo, mut hi) = (0, excluded.len());
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    if excluded[mid] - mid as u64 <= row {
                        lo = mid + 1;
                    } else {
                        hi = mid;
                    }
                }
                Some(row + lo as u64)
            }
        }
    }

    /// The row of `line`, or of the nearest shown line before it.
    pub fn row_of(&self, line: u64) -> u64 {
        match self {
            Shown::All => line,
            Shown::Lines(v) => v.partition_point(|&l| l <= line).saturating_sub(1) as u64,
            Shown::AllExcept { excluded, .. } => {
                let before = excluded.partition_point(|&l| l < line);
                let hidden = excluded.get(before) == Some(&line);
                (line - before as u64).saturating_sub(u64::from(hidden))
            }
        }
    }
}

/// A place in the viewer: a line, and a byte offset into the line's text as
/// shown (decoded to UTF-8, and cut after [`MAX_LINE_BYTES`] of the file).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pos {
    pub line: u64,
    pub col: usize,
}

/// What happened, for the app to report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewerEvent {
    /// Find Next/Previous selected a match (after wrapping around).
    Found {
        wrapped: bool,
    },
    NotFound,
    /// Count finished.
    Count(u64),
    /// Copy put this many bytes on the clipboard; the selection was longer.
    Copied {
        bytes: usize,
        truncated: bool,
    },
    /// The file got shorter on disk, so it was read again.
    Reloaded,
}

/// A line as drawn: where it is in the file, and its layout.
struct ShownLine {
    /// File offsets of the line's text (without its line break).
    start: u64,
    end: u64,
    /// Where the next line starts (after the break), or `end` for the last.
    next: u64,
    layout: Arc<LineLayout>,
    /// Bytes of the line not shown.
    cut: u64,
}

enum TaskResult {
    Found { range: Range<u64>, wrapped: bool },
    NotFound,
    Count(u64),
}

/// A search running in the background.
struct Task {
    rx: Receiver<TaskResult>,
    stop: Arc<AtomicBool>,
}

impl Drop for Task {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// What a filter search sends: matching lines found since the last batch,
/// in order, and how far the search has looked (a file offset).
struct FilterBatch {
    lines: Vec<u64>,
    decided: u64,
}

/// A filter on a viewer (PLAN_v0.3.0 P8) and the search finding its lines.
pub struct ViewerFilter {
    pub query: SearchQuery,
    /// Show the matching lines (else the others).
    pub keep: bool,
    rx: Option<Receiver<FilterBatch>>,
    stop: Arc<AtomicBool>,
    /// How far the search has read, for progress.
    read: Arc<AtomicU64>,
    /// The end of what the current search covers.
    until: u64,
    /// The last matching line found (searches that go on with a grown file
    /// start at its last line, which may come again).
    last: Option<u64>,
}

impl Drop for ViewerFilter {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// A file source that remembers how far it has been read.
struct Tracked {
    inner: ochrepad_fileio::bigfile::FileSource,
    read: Arc<AtomicU64>,
}

impl ochrepad_core::search::ByteSource for Tracked {
    fn len(&self) -> u64 {
        self.inner.len()
    }

    fn read_at(&self, at: u64, buf: &mut [u8]) -> usize {
        let n = self.inner.read_at(at, buf);
        self.read.fetch_max(at + n as u64, Ordering::Relaxed);
        n
    }
}

/// Searches `file` from `from` for `query`'s lines, sending them in
/// batches about ten times a second.
fn spawn_filter(
    file: &BigFile,
    query: &SearchQuery,
    from: u64,
    stop: Arc<AtomicBool>,
    read: Arc<AtomicU64>,
) -> Result<Receiver<FilterBatch>, String> {
    let searcher = Searcher::new_raw(query).map_err(|e| e.0)?;
    let src = Tracked {
        inner: file.source(stop.clone()),
        read,
    };
    let finder = file.line_finder();
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("ochrepad-viewer-filter".into())
        .spawn(move || {
            use ochrepad_core::search::ByteSource;
            let mut batch = Vec::new();
            let mut sent = std::time::Instant::now();
            let mut hint = None;
            let mut last: Option<u64> = None;
            let len = src.len();
            for m in searcher.matches_in(&src, from..len) {
                let Some(first) = finder.line_of(m.start, hint, &stop) else {
                    return;
                };
                hint = Some((m.start, first));
                let end_line = if m.end > m.start + 1 {
                    match finder.line_of(m.end - 1, hint, &stop) {
                        Some(l) => l,
                        None => return,
                    }
                } else {
                    first
                };
                for l in first..=end_line {
                    if last.is_none_or(|p| l > p) {
                        batch.push(l);
                        last = Some(l);
                    }
                }
                if sent.elapsed().as_millis() >= 100 {
                    sent = std::time::Instant::now();
                    let lines = std::mem::take(&mut batch);
                    if tx
                        .send(FilterBatch {
                            lines,
                            decided: m.start,
                        })
                        .is_err()
                    {
                        return;
                    }
                }
            }
            if !stop.load(Ordering::Relaxed) {
                let _ = tx.send(FilterBatch {
                    lines: batch,
                    decided: len,
                });
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(rx)
}

/// Everything the widget keeps between frames.
pub struct ViewerState {
    pub shown: Shown,
    /// The filter, if one is on.
    pub filter: Option<ViewerFilter>,
    /// Follow (tail -f): when the file grows, scroll to its end.
    pub follow: bool,
    /// The top row, with the fraction of it scrolled past.
    top: f64,
    x_off: f32,
    pub anchor: Pos,
    pub head: Pos,
    /// The x to keep when moving up and down.
    goal_x: Option<f32>,
    lines: HashMap<u64, Arc<ShownLine>>,
    lines_key: Option<(u64, u32, usize, Color32)>,
    max_width: f32,
    dragging: bool,
    bar_drag: Option<f32>,
    last_press: Option<(f64, Pos2, u32)>,
    last_activity: f64,
    request_focus: bool,
    reveal: bool,
    task: Option<Task>,
    /// A match found by a search, waiting for the index to reach it.
    pending_select: Option<(Range<u64>, bool)>,
    /// Go to Line waiting for the index: (line, column).
    pending_go_to: Option<(u64, usize)>,
    pub events: Vec<ViewerEvent>,
}

impl Default for ViewerState {
    fn default() -> Self {
        Self {
            shown: Shown::All,
            filter: None,
            follow: false,
            top: 0.0,
            x_off: 0.0,
            anchor: Pos::default(),
            head: Pos::default(),
            goal_x: None,
            lines: HashMap::new(),
            lines_key: None,
            max_width: 0.0,
            dragging: false,
            bar_drag: None,
            last_press: None,
            last_activity: 0.0,
            request_focus: false,
            reveal: false,
            task: None,
            pending_select: None,
            pending_go_to: None,
            events: Vec::new(),
        }
    }
}

/// A large file open in a viewer tab.
pub struct Viewer {
    pub file: BigFile,
    pub state: ViewerState,
}

/// The open viewers.
#[derive(Default)]
pub struct Viewers {
    map: HashMap<ViewerId, Viewer>,
    next: u64,
}

impl Viewers {
    pub fn add(&mut self, file: BigFile) -> ViewerId {
        self.next += 1;
        let id = ViewerId(self.next);
        self.map.insert(
            id,
            Viewer {
                file,
                state: ViewerState::default(),
            },
        );
        id
    }

    pub fn get(&self, id: ViewerId) -> Option<&Viewer> {
        self.map.get(&id)
    }

    pub fn get_mut(&mut self, id: ViewerId) -> Option<&mut Viewer> {
        self.map.get_mut(&id)
    }

    pub fn remove(&mut self, id: ViewerId) -> Option<Viewer> {
        self.map.remove(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (ViewerId, &Viewer)> {
        self.map.iter().map(|(id, v)| (*id, v))
    }

    /// The viewer showing `path`, if one is open.
    pub fn find(&self, path: &std::path::Path) -> Option<ViewerId> {
        let wanted = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.map.iter().find_map(|(id, v)| {
            let p = v.file.path();
            (std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()) == wanted).then_some(*id)
        })
    }
}

/// The egui id of the viewer widget (it holds keyboard focus).
pub fn viewer_widget_id(id: ViewerId) -> Id {
    Id::new(("ochrepad-viewer", id.0))
}

impl ViewerState {
    /// Gives the widget the keyboard on the next frame.
    pub fn request_focus(&mut self) {
        self.request_focus = true;
    }

    /// The selection, start first.
    pub fn ordered(&self) -> (Pos, Pos) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    /// The first line on screen.
    pub fn top_line(&self) -> u64 {
        self.shown.line(self.top as u64).unwrap_or(0)
    }

    /// Scrolls so `line` is at the top (session restore).
    pub fn scroll_to_line(&mut self, line: u64) {
        self.top = self.shown.row_of(line) as f64;
    }

    /// Whether a search started here is still running.
    pub fn searching(&self) -> bool {
        self.task.is_some() || self.pending_select.is_some()
    }

    /// Stops a running search.
    pub fn stop_search(&mut self) {
        self.task = None;
        self.pending_select = None;
    }

    /// Moves the caret to `line` (1-based) and `column` (1-based, in
    /// characters), waiting for the index if it has not got that far.
    pub fn go_to_line(&mut self, line: u64, column: usize) {
        self.pending_go_to = Some((line.saturating_sub(1), column.saturating_sub(1)));
    }

    /// Whether Go to Line is waiting for the index.
    pub fn waiting_to_go(&self) -> Option<u64> {
        self.pending_go_to.map(|(l, _)| l + 1)
    }

    /// The line's layout and place, from the cache or the file.
    fn line(
        &mut self,
        file: &BigFile,
        line: u64,
        cell_w: f32,
        tab_width: usize,
        color: Color32,
    ) -> Option<Arc<ShownLine>> {
        let key = (file.generation(), cell_w.to_bits(), tab_width, color);
        if self.lines_key != Some(key) {
            self.lines.clear();
            self.lines_key = Some(key);
        }
        if let Some(l) = self.lines.get(&line) {
            return Some(l.clone());
        }
        let start = match line.checked_sub(1).and_then(|p| self.lines.get(&p)) {
            Some(prev) if prev.next > prev.end => prev.next,
            _ => file.line_start(line)?,
        };
        let end = file.line_end(start);
        let next = match file.bytes(end..end + 2) {
            [b'\r', b'\n', ..] => end + 2,
            [b'\n' | b'\r', ..] => end + 1,
            _ => end,
        };
        let mut shown_end = end.min(start + MAX_LINE_BYTES);
        if shown_end < end && file.decoder().is_utf8() {
            // Do not cut a character in two.
            for _ in 0..3 {
                match file.bytes(shown_end..shown_end + 1) {
                    [b] if b & 0xC0 == 0x80 => shown_end -= 1,
                    _ => break,
                }
            }
        }
        let text = file.decode(file.bytes(start..shown_end)).into_owned();
        let layout = Arc::new(LineLayout::build(text, cell_w, tab_width, color, None));
        let shown = Arc::new(ShownLine {
            start,
            end,
            next,
            layout,
            cut: end - shown_end,
        });
        self.lines.insert(line, shown.clone());
        Some(shown)
    }

    /// The file offset of `pos`.
    fn file_offset(&self, file: &BigFile, pos: Pos) -> Option<u64> {
        let start = match self.lines.get(&pos.line) {
            Some(l) => l.start,
            None => file.line_start(pos.line)?,
        };
        let raw = file.bytes(start..start + MAX_LINE_BYTES + 4);
        Some(start + file.decoder().byte_of_col(raw, pos.col) as u64)
    }

    /// The place of file offset `at`, if the index has reached it.
    fn pos_of(&self, file: &BigFile, at: u64) -> Option<Pos> {
        let line = file.line_of(at)?;
        let start = file.line_start(line)?;
        let upto = at.min(start + MAX_LINE_BYTES);
        let col = file.decode(file.bytes(start..upto)).len();
        Some(Pos { line, col })
    }

    /// Copies the selection (at most [`MAX_COPY`] bytes of the file), with
    /// line breaks as `\n` like the editor's.
    pub fn copy(&mut self, ctx: &egui::Context, file: &BigFile) {
        let (a, b) = self.ordered();
        if a == b {
            return;
        }
        let (Some(start), Some(end)) = (self.file_offset(file, a), self.file_offset(file, b))
        else {
            return;
        };
        let truncated = end - start > MAX_COPY;
        let bytes = file.read(start..end.min(start + MAX_COPY));
        let text = file.decode(&bytes);
        let text = ochrepad_fileio::normalize_line_endings(&text).into_owned();
        self.events.push(ViewerEvent::Copied {
            bytes: bytes.len(),
            truncated,
        });
        ctx.copy_text(text);
    }

    /// The selected text, if it is on one line (to search for).
    pub fn selected_line_text(&self, file: &BigFile) -> Option<String> {
        let (a, b) = self.ordered();
        if a == b || a.line != b.line {
            return None;
        }
        let line = self.lines.get(&a.line)?;
        line.layout
            .text
            .get(a.col..b.col)
            .map(str::to_owned)
            .or_else(|| {
                let start = self.file_offset(file, a)?;
                let end = self.file_offset(file, b)?;
                Some(file.decode(file.bytes(start..end)).into_owned())
            })
    }

    /// Selects the file bytes `range` (a search result) and scrolls to it,
    /// once the index has reached it.
    pub fn select_range(&mut self, range: Range<u64>) {
        self.pending_select = Some((range, false));
    }

    /// Find Next/Previous in the background, from the selection.
    pub fn start_find(
        &mut self,
        file: &BigFile,
        query: &SearchQuery,
        backward: bool,
    ) -> Result<(), String> {
        let searcher = Searcher::new_raw(query).map_err(|e| e.0)?;
        let (a, b) = self.ordered();
        let from_start = self.file_offset(file, a).unwrap_or(0);
        let from_end = self.file_offset(file, b).unwrap_or(from_start);
        let wrap = query.options.wrap;
        let stop = Arc::new(AtomicBool::new(false));
        let src = file.source(stop.clone());
        let (tx, rx) = channel();
        let work = move || {
            use ochrepad_core::search::ByteSource;
            let found = if backward {
                searcher
                    .find_prev_in(&src, from_start)
                    .map(|m| (m, false))
                    .or_else(|| {
                        wrap.then(|| searcher.find_prev_in(&src, src.len()))
                            .flatten()
                            .map(|m| (m, true))
                    })
            } else {
                let mut m = searcher.find_next_in(&src, from_end);
                // An empty match where the caret is: step on.
                if m.as_ref()
                    .is_some_and(|m| m.is_empty() && m.start == from_end && from_start == from_end)
                {
                    m = searcher.find_next_in(&src, from_end + 1);
                }
                m.map(|m| (m, false)).or_else(|| {
                    wrap.then(|| searcher.find_next_in(&src, 0))
                        .flatten()
                        .map(|m| (m, true))
                })
            };
            let _ = tx.send(match found {
                Some((range, wrapped)) => TaskResult::Found { range, wrapped },
                None => TaskResult::NotFound,
            });
        };
        std::thread::Builder::new()
            .name("ochrepad-viewer-find".into())
            .spawn(work)
            .map_err(|e| e.to_string())?;
        self.task = Some(Task { rx, stop });
        Ok(())
    }

    /// Counts the matches in the background.
    pub fn start_count(&mut self, file: &BigFile, query: &SearchQuery) -> Result<(), String> {
        let searcher = Searcher::new_raw(query).map_err(|e| e.0)?;
        let stop = Arc::new(AtomicBool::new(false));
        let src = file.source(stop.clone());
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("ochrepad-viewer-count".into())
            .spawn(move || {
                use ochrepad_core::search::ByteSource;
                let n = searcher.matches_in(&src, 0..src.len()).count() as u64;
                let _ = tx.send(TaskResult::Count(n));
            })
            .map_err(|e| e.to_string())?;
        self.task = Some(Task { rx, stop });
        Ok(())
    }

    /// Shows only the lines matching `query` (`keep`), or only the others,
    /// or every line again with `None`. The lines are found in the
    /// background; the view fills in as they come.
    pub fn set_filter(
        &mut self,
        file: &BigFile,
        filter: Option<(SearchQuery, bool)>,
    ) -> Result<(), String> {
        self.filter = None;
        let Some((query, keep)) = filter else {
            self.shown = Shown::All;
            return Ok(());
        };
        let stop = Arc::new(AtomicBool::new(false));
        let read = Arc::new(AtomicU64::new(0));
        let rx = spawn_filter(file, &query, 0, stop.clone(), read.clone())?;
        self.shown = if keep {
            Shown::Lines(Arc::default())
        } else {
            Shown::AllExcept {
                excluded: Arc::default(),
                decided: Some(0),
            }
        };
        self.filter = Some(ViewerFilter {
            query,
            keep,
            rx: Some(rx),
            stop,
            read,
            until: file.len(),
            last: None,
        });
        self.top = 0.0;
        Ok(())
    }

    /// The filter's progress: rows shown, and how much of the file it has
    /// searched (1 when done).
    pub fn filter_progress(&self, file: &BigFile) -> Option<(u64, f32)> {
        let f = self.filter.as_ref()?;
        let rows = self.shown.rows(file.progress().lines);
        let done = f.rx.is_none();
        let part = if done || f.until == 0 {
            1.0
        } else {
            (f.read.load(Ordering::Relaxed) as f64 / f.until as f64).min(0.99) as f32
        };
        Some((rows, part))
    }

    /// Takes in the filter search's lines; goes on searching what the file
    /// gained, or from the start after a reload.
    fn poll_filter(&mut self, file: &BigFile, change: Change) {
        let Some(f) = &mut self.filter else {
            return;
        };
        if change == Change::Reloaded {
            let filter = Some((f.query.clone(), f.keep));
            let _ = self.set_filter(file, filter);
            return;
        }
        let mut lines = Vec::new();
        let mut decided = None;
        let mut finished = false;
        if let Some(rx) = &f.rx {
            loop {
                match rx.try_recv() {
                    Ok(batch) => {
                        lines.extend(batch.lines);
                        finished |= batch.decided >= f.until;
                        decided = Some(batch.decided);
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        finished = true;
                        break;
                    }
                }
            }
        }
        // A search going on with a grown file may find its first line again.
        lines.retain(|&l| f.last.is_none_or(|p| l > p));
        if let Some(&l) = lines.last() {
            f.last = Some(l);
        }
        if finished {
            f.rx = None;
        }
        let searching = f.rx.is_some();
        let decided_line = decided.and_then(|d| file.line_of(d.min(file.len())));
        match &mut self.shown {
            Shown::Lines(v) => Arc::make_mut(v).extend(lines),
            Shown::AllExcept {
                excluded,
                decided: limit,
            } => {
                Arc::make_mut(excluded).extend(lines);
                if !searching {
                    *limit = None;
                } else if let Some(d) = decided_line {
                    *limit = Some(d);
                }
            }
            Shown::All => {}
        }
        // The file grew past what the search covered: look through what it
        // gained, from the start of the line that was last (it may have
        // grown).
        if !searching && file.len() > f.until {
            let end = f.until;
            let from = file
                .line_of(end.saturating_sub(1))
                .and_then(|l| file.line_start(l))
                .unwrap_or(end)
                .min(end);
            let stop = Arc::new(AtomicBool::new(false));
            let read = Arc::new(AtomicU64::new(from));
            if let Ok(rx) = spawn_filter(file, &f.query, from, stop.clone(), read.clone()) {
                f.rx = Some(rx);
                f.stop = stop;
                f.read = read;
                f.until = file.len();
                if let Shown::AllExcept { decided, .. } = &mut self.shown {
                    *decided = file.line_of(from);
                }
            }
        }
    }

    /// Takes in background results, and moves to a match or line the index
    /// has now reached.
    fn poll(&mut self, file: &BigFile, rows: u64) {
        if let Some(task) = &self.task {
            match task.rx.try_recv() {
                Ok(TaskResult::Found { range, wrapped }) => {
                    self.task = None;
                    self.pending_select = Some((range, wrapped));
                }
                Ok(TaskResult::NotFound) => {
                    self.task = None;
                    self.events.push(ViewerEvent::NotFound);
                }
                Ok(TaskResult::Count(n)) => {
                    self.task = None;
                    self.events.push(ViewerEvent::Count(n));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.task = None,
            }
        }
        if let Some((range, wrapped)) = self.pending_select.clone()
            && let (Some(a), Some(b)) =
                (self.pos_of(file, range.start), self.pos_of(file, range.end))
        {
            self.pending_select = None;
            self.anchor = a;
            self.head = b;
            self.goal_x = None;
            self.reveal = true;
            self.events.push(ViewerEvent::Found { wrapped });
        }
        if let Some((line, column)) = self.pending_go_to {
            let progress = file.progress();
            let line = if progress.done {
                line.min(progress.lines.saturating_sub(1))
            } else {
                line
            };
            if file.line_start(line).is_some() || (progress.done && rows == 0) {
                self.pending_go_to = None;
                let col = self.column_to_col(file, line, column);
                self.head = Pos { line, col };
                self.anchor = self.head;
                self.goal_x = None;
                self.reveal = true;
            }
        }
    }

    /// The byte offset in `line`'s shown text of the `column`-th character
    /// (0-based), or its end.
    fn column_to_col(&self, file: &BigFile, line: u64, column: usize) -> usize {
        let text = match self.lines.get(&line) {
            Some(l) => l.layout.text.clone(),
            None => {
                let Some(start) = file.line_start(line) else {
                    return 0;
                };
                let end = file.line_end(start).min(start + MAX_LINE_BYTES);
                file.decode(file.bytes(start..end)).into_owned()
            }
        };
        text.char_indices()
            .nth(column)
            .map_or(text.len(), |(i, _)| i)
    }
}

/// Geometry of one frame.
struct Frame {
    content: Rect,
    gutter: Rect,
    vbar: Rect,
    hbar: Rect,
    font: FontId,
    cell_w: f32,
    line_h: f32,
    rows_full: usize,
}

impl Frame {
    fn text_left(&self) -> f32 {
        self.content.min.x + TEXT_PAD
    }
}

/// Shows `viewer` filling the available space. `label` names the file for
/// accessibility; `highlight` marks the find panel's matches.
#[allow(clippy::too_many_arguments)]
pub fn viewer_ui(
    ui: &mut Ui,
    id: ViewerId,
    viewer: &mut Viewer,
    look: &Look,
    tab_width: usize,
    label: &str,
    highlight: Option<&Searcher>,
    rules: Option<&crate::log_rules::LogRules>,
) -> Response {
    let Viewer { file, state } = viewer;
    let rect = ui.available_rect_before_wrap();
    ui.advance_cursor_after_rect(rect);
    let widget = viewer_widget_id(id);
    let response = ui.interact(rect, widget, Sense::click_and_drag());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::TextEdit, true, label));
    let now = ui.input(|i| i.time);

    // The file may have changed on disk since the last frame: look before
    // reading any of it.
    let old_len = file.len();
    let change = file.refresh();
    match change {
        Change::Unchanged => {}
        Change::Grew => {
            // The last line may have grown.
            state.lines.retain(|_, l| l.next < old_len);
        }
        Change::Reloaded => {
            state.lines.clear();
            state.task = None;
            state.events.push(ViewerEvent::Reloaded);
        }
    }
    state.poll_filter(file, change);
    if state.filter.as_ref().is_some_and(|f| f.rx.is_some()) {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
    let progress = file.progress();
    if !progress.done {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
    let known = progress.lines;
    let rows = state.shown.rows(known);
    state.poll(file, rows);
    if state.task.is_some() || state.pending_select.is_some() || state.pending_go_to.is_some() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(30));
    }
    // Positions past the end (after a reload): back to the last line.
    let last_line = state.shown.line(rows.saturating_sub(1)).unwrap_or(0);
    for p in [&mut state.anchor, &mut state.head] {
        if p.line > last_line && progress.done {
            *p = Pos {
                line: last_line,
                col: 0,
            };
        }
    }

    let font = FontId::monospace(look.font_size);
    let (cell_w, line_h) = ui.fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
    let digits = progress.estimated_lines().max(1).to_string().len().max(4);
    let gutter_w = (digits + 1) as f32 * cell_w + TEXT_PAD;
    let body = Rect::from_min_max(
        rect.min,
        Pos2::new(rect.max.x - SCROLLBAR_W, rect.max.y - SCROLLBAR_W),
    );
    let gutter = Rect::from_min_max(body.min, Pos2::new(body.min.x + gutter_w, body.max.y));
    let content = Rect::from_min_max(Pos2::new(gutter.max.x, body.min.y), body.max);
    let f = Frame {
        vbar: Rect::from_min_max(
            Pos2::new(body.max.x, rect.min.y),
            Pos2::new(rect.max.x, body.max.y),
        ),
        hbar: Rect::from_min_max(
            Pos2::new(content.min.x, body.max.y),
            Pos2::new(body.max.x, rect.max.y),
        ),
        content,
        gutter,
        font,
        cell_w,
        line_h,
        rows_full: ((content.height() / line_h).floor() as usize).max(1),
    };
    let color = ui.visuals().text_color();
    let max_top = rows.saturating_sub(f.rows_full as u64) as f64;
    // Follow (tail -f): a file that grew shows its end.
    if state.follow && change == Change::Grew {
        state.top = max_top;
    }

    // --- Focus ------------------------------------------------------------------
    let pressed_here = ui.input(|i| i.pointer.primary_pressed()) && response.contains_pointer();
    if state.request_focus || pressed_here {
        response.request_focus();
        state.request_focus = false;
    }
    let focused = response.has_focus();
    if focused {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                widget,
                EventFilter {
                    tab: false,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            )
        });
    }

    // --- Keyboard -----------------------------------------------------------------
    if focused {
        for event in ui.input(|i| i.events.clone()) {
            match event {
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    if key_motion(state, file, &f, key, modifiers, rows, tab_width, color) {
                        state.last_activity = now;
                    }
                }
                Event::Copy => state.copy(ui.ctx(), file),
                _ => {}
            }
        }
    }

    // --- Wheel and scrollbars -----------------------------------------------------
    if response.contains_pointer() {
        let zoom = ui.input(|i| crate::editor_view::wheel_zoom_steps(&i.events));
        if zoom != 0.0 {
            ui.ctx().data_mut(|d| {
                *d.get_temp_mut_or_default::<f32>(crate::editor_view::zoom_id()) += zoom
            });
        }
        let d = ui.input(|i| i.smooth_scroll_delta);
        if d != Vec2::ZERO {
            state.top -= (d.y / line_h) as f64;
            state.x_off = (state.x_off - d.x).max(0.0);
            ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        }
    }
    let (pointer, down, primary_pressed) = ui.input(|i| {
        (
            i.pointer.interact_pos(),
            i.pointer.primary_down(),
            i.pointer.primary_pressed(),
        )
    });
    let h_max = (state.max_width + 4.0 * cell_w - f.content.width()).max(0.0);
    let thumb_h = if rows > 0 {
        (f.vbar.height() * (f.rows_full as f32 / rows as f32).min(1.0)).max(20.0)
    } else {
        f.vbar.height()
    }
    .min(f.vbar.height());
    let track = (f.vbar.height() - thumb_h).max(1.0);
    if primary_pressed
        && let Some(p) = pointer
        && f.vbar.contains(p)
    {
        let thumb_top = f.vbar.min.y + track * (state.top / max_top.max(1.0)) as f32;
        let grab = if (thumb_top..thumb_top + thumb_h).contains(&p.y) {
            p.y - thumb_top
        } else {
            thumb_h / 2.0
        };
        state.bar_drag = Some(grab);
    }
    if !down {
        state.bar_drag = None;
    }
    if let (Some(grab), Some(p)) = (state.bar_drag, pointer) {
        let t = ((p.y - grab - f.vbar.min.y) / track).clamp(0.0, 1.0);
        state.top = t as f64 * max_top;
    }

    // --- Mouse selection ----------------------------------------------------------
    if primary_pressed
        && state.bar_drag.is_none()
        && let Some(p) = pointer
        && response.contains_pointer()
        && (f.content.contains(p) || f.gutter.contains(p))
    {
        let count = match state.last_press {
            Some((t, lp, c)) if now - t < MULTI_CLICK_TIME && lp.distance(p) < 4.0 => c % 3 + 1,
            _ => 1,
        };
        state.last_press = Some((now, p, count));
        let shift = ui.input(|i| i.modifiers.shift);
        if let Some(hit) = hit_test(state, file, &f, p, rows, tab_width, color) {
            if f.gutter.contains(p) || count == 3 {
                select_line(state, file, &f, hit.line, tab_width, color);
            } else if count == 2 {
                select_word(state, file, &f, hit, tab_width, color);
            } else {
                state.head = hit;
                if !shift {
                    state.anchor = hit;
                }
            }
            state.goal_x = None;
            state.dragging = count == 1 && !f.gutter.contains(p);
            state.last_activity = now;
        }
    }
    if !down {
        state.dragging = false;
    }
    if state.dragging
        && let Some(p) = pointer
    {
        // Scroll when dragging past the edges.
        let dt = ui.input(|i| i.stable_dt) as f64;
        if p.y < f.content.min.y {
            state.top -= (4.0 + 20.0 * (f.content.min.y - p.y) as f64 / line_h as f64) * dt;
        } else if p.y > f.content.max.y {
            state.top += (4.0 + 20.0 * (p.y - f.content.max.y) as f64 / line_h as f64) * dt;
        }
        if !f.content.contains(p) {
            ui.ctx().request_repaint();
        }
        state.top = state.top.clamp(0.0, max_top);
        let clamped = p.clamp(f.content.min, f.content.max - Vec2::splat(0.5));
        if let Some(hit) = hit_test(state, file, &f, clamped, rows, tab_width, color) {
            state.head = hit;
            state.last_activity = now;
        }
    }

    // --- Keep the caret in view after keyboard moves and searches ------------------
    if state.reveal {
        state.reveal = false;
        let row = state.shown.row_of(state.head.line) as f64;
        if row < state.top {
            state.top = row;
        } else if row + 1.0 > state.top + f.rows_full as f64 {
            state.top = row + 1.0 - f.rows_full as f64;
        }
        if let Some(l) = state.line(file, state.head.line, cell_w, tab_width, color) {
            let x = l.layout.x_of(state.head.col);
            let visible = f.content.width() - TEXT_PAD - cell_w;
            if x < state.x_off {
                state.x_off = (x - 4.0 * cell_w).max(0.0);
            } else if x > state.x_off + visible {
                state.x_off = x - visible + 4.0 * cell_w;
            }
        }
    }
    state.top = state.top.clamp(0.0, max_top);
    state.x_off = state
        .x_off
        .clamp(0.0, h_max.max(state.x_off.min(state.max_width)));

    if response.hovered()
        && let Some(p) = ui.input(|i| i.pointer.hover_pos())
        && f.content.contains(p)
    {
        ui.ctx().set_cursor_icon(CursorIcon::Text);
    }

    paint(
        ui, state, file, &f, rows, max_top, h_max, focused, now, tab_width, color, highlight, rules,
    );
    response
}

/// The position under `p`, if any line is there.
fn hit_test(
    state: &mut ViewerState,
    file: &BigFile,
    f: &Frame,
    p: Pos2,
    rows: u64,
    tab_width: usize,
    color: Color32,
) -> Option<Pos> {
    if rows == 0 {
        return None;
    }
    let row = (state.top + ((p.y - f.content.min.y) / f.line_h) as f64).floor();
    let row = (row.max(0.0) as u64).min(rows - 1);
    let line = state.shown.line(row)?;
    let l = state.line(file, line, f.cell_w, tab_width, color)?;
    let x = p.x - f.text_left() + state.x_off;
    Some(Pos {
        line,
        col: l.layout.byte_at_x(x),
    })
}

fn select_line(
    state: &mut ViewerState,
    file: &BigFile,
    f: &Frame,
    line: u64,
    tab_width: usize,
    color: Color32,
) {
    let len = state
        .line(file, line, f.cell_w, tab_width, color)
        .map_or(0, |l| l.layout.text.len());
    state.anchor = Pos { line, col: 0 };
    state.head = Pos { line, col: len };
}

fn select_word(
    state: &mut ViewerState,
    file: &BigFile,
    f: &Frame,
    at: Pos,
    tab_width: usize,
    color: Color32,
) {
    let Some(l) = state.line(file, at.line, f.cell_w, tab_width, color) else {
        return;
    };
    let text = &l.layout.text;
    let (start, end) = text
        .split_word_bound_indices()
        .map(|(i, w)| (i, i + w.len()))
        .find(|&(s, e)| s <= at.col && at.col < e)
        .unwrap_or((at.col, at.col));
    state.anchor = Pos {
        line: at.line,
        col: start,
    };
    state.head = Pos {
        line: at.line,
        col: end,
    };
}

/// Applies a navigation key; returns whether it did anything.
#[allow(clippy::too_many_arguments)]
fn key_motion(
    state: &mut ViewerState,
    file: &BigFile,
    f: &Frame,
    key: Key,
    m: Modifiers,
    rows: u64,
    tab_width: usize,
    color: Color32,
) -> bool {
    if rows == 0 {
        return false;
    }
    let line_text = |state: &mut ViewerState, line: u64| {
        state
            .line(file, line, f.cell_w, tab_width, color)
            .map(|l| l.layout.clone())
    };
    let head = state.head;
    let row = state.shown.row_of(head.line);
    let page = f.rows_full.saturating_sub(1).max(1) as u64;
    let vertical = |state: &mut ViewerState, to_row: u64| -> Option<Pos> {
        let line = state.shown.line(to_row.min(rows - 1))?;
        let goal = match state.goal_x {
            Some(x) => x,
            None => {
                let x = line_text(state, head.line).map_or(0.0, |l| l.x_of(head.col));
                state.goal_x = Some(x);
                x
            }
        };
        let col = line_text(state, line).map_or(0, |l| l.byte_at_x(goal));
        Some(Pos { line, col })
    };
    let new = match key {
        Key::A if m.command => {
            state.anchor = Pos {
                line: state.shown.line(0).unwrap_or(0),
                col: 0,
            };
            let last = state.shown.line(rows - 1).unwrap_or(0);
            let len = line_text(state, last).map_or(0, |l| l.text.len());
            state.head = Pos {
                line: last,
                col: len,
            };
            return true;
        }
        Key::Escape => {
            state.anchor = state.head;
            return true;
        }
        Key::ArrowUp => vertical(state, row.saturating_sub(1)),
        Key::ArrowDown => vertical(state, row + 1),
        Key::PageUp => {
            state.top = (state.top - page as f64).max(0.0);
            vertical(state, row.saturating_sub(page))
        }
        Key::PageDown => {
            state.top += page as f64;
            vertical(state, row + page)
        }
        Key::Home if m.command => Some(Pos {
            line: state.shown.line(0).unwrap_or(0),
            col: 0,
        }),
        Key::End if m.command => {
            let last = state.shown.line(rows - 1).unwrap_or(0);
            let len = line_text(state, last).map_or(0, |l| l.text.len());
            Some(Pos {
                line: last,
                col: len,
            })
        }
        Key::Home => {
            // The first non-blank character, then the line's start.
            let text = line_text(state, head.line).map(|l| l.text.clone());
            let indent = text
                .as_deref()
                .map_or(0, |t| t.len() - t.trim_start_matches([' ', '\t']).len());
            Some(Pos {
                line: head.line,
                col: if head.col == indent { 0 } else { indent },
            })
        }
        Key::End => {
            let len = line_text(state, head.line).map_or(0, |l| l.text.len());
            Some(Pos {
                line: head.line,
                col: len,
            })
        }
        Key::ArrowLeft | Key::ArrowRight => {
            let right = key == Key::ArrowRight;
            let text = line_text(state, head.line).map(|l| l.text.clone());
            let text = text.unwrap_or_default();
            let moved = if m.command {
                let bounds: Vec<usize> = text
                    .split_word_bound_indices()
                    .filter(|(_, w)| !w.trim().is_empty())
                    .map(|(i, _)| i)
                    .chain(std::iter::once(text.len()))
                    .collect();
                if right {
                    bounds.into_iter().find(|&b| b > head.col)
                } else {
                    bounds.into_iter().rev().find(|&b| b < head.col)
                }
            } else if right {
                text.grapheme_indices(true)
                    .map(|(i, g)| i + g.len())
                    .find(|&b| b > head.col)
            } else {
                text.grapheme_indices(true)
                    .map(|(i, _)| i)
                    .rev()
                    .find(|&b| b < head.col)
            };
            match moved {
                Some(col) => Some(Pos {
                    line: head.line,
                    col,
                }),
                // Over the line break.
                None if right && row + 1 < rows => {
                    state.shown.line(row + 1).map(|line| Pos { line, col: 0 })
                }
                None if !right && row > 0 => {
                    let line = state.shown.line(row - 1);
                    line.map(|line| Pos {
                        line,
                        col: line_text(state, line).map_or(0, |l| l.text.len()),
                    })
                }
                None => Some(head),
            }
        }
        _ => None,
    };
    let Some(new) = new else {
        return false;
    };
    if !matches!(
        key,
        Key::ArrowUp | Key::ArrowDown | Key::PageUp | Key::PageDown
    ) {
        state.goal_x = None;
    }
    state.head = new;
    if !m.shift {
        state.anchor = new;
    }
    state.reveal = true;
    true
}

#[allow(clippy::too_many_arguments)]
fn paint(
    ui: &mut Ui,
    state: &mut ViewerState,
    file: &BigFile,
    f: &Frame,
    rows: u64,
    max_top: f64,
    h_max: f32,
    focused: bool,
    now: f64,
    tab_width: usize,
    color: Color32,
    highlight: Option<&Searcher>,
    rules: Option<&crate::log_rules::LogRules>,
) {
    let visuals = ui.visuals().clone();
    let painter = ui.painter().clone();
    let text_painter = painter.with_clip_rect(f.content.intersect(painter.clip_rect()));
    let gutter_painter = painter.with_clip_rect(f.gutter.intersect(painter.clip_rect()));
    painter.rect_filled(f.content, 0.0, visuals.extreme_bg_color);
    painter.rect_filled(f.gutter, 0.0, visuals.extreme_bg_color);
    painter.rect_filled(f.vbar.union(f.hbar), 0.0, visuals.extreme_bg_color);

    let accent = crate::theme::accent(&visuals);
    let sel_bg = visuals.selection.bg_fill;
    let match_bg = crate::theme::match_bg(&visuals);
    let weak = visuals.weak_text_color();
    let (sel_a, sel_b) = state.ordered();
    let first_row = state.top.floor() as u64;
    let frac = (state.top - state.top.floor()) as f32;
    let x0 = f.text_left() - state.x_off;
    let mut caret: Option<Pos2> = None;
    let mut drawn = Vec::new();
    for i in 0..=f.rows_full as u64 + 1 {
        let row = first_row + i;
        if row >= rows {
            break;
        }
        let Some(line) = state.shown.line(row) else {
            break;
        };
        let Some(l) = state.line(file, line, f.cell_w, tab_width, color) else {
            // Not indexed that far yet.
            break;
        };
        drawn.push(line);
        state.max_width = state.max_width.max(l.layout.width);
        let y = f.content.min.y + (i as f32 - frac) * f.line_h;
        let row_rect = Rect::from_min_size(
            Pos2::new(f.content.min.x, y),
            Vec2::new(f.content.width(), f.line_h),
        );
        if line == state.head.line {
            text_painter.rect_filled(row_rect, 0.0, visuals.faint_bg_color);
        }
        // Log highlight rules.
        let look = rules.map(|r| r.look_text(&l.layout.text, visuals.dark_mode));
        if let Some(bg) = look.as_ref().and_then(|k| k.background) {
            text_painter.rect_filled(row_rect, 0.0, bg);
        }
        let x_at = |col: usize| x0 + l.layout.x_of(col);
        for (span, c) in look.iter().flat_map(|k| k.spans.iter()) {
            let r = Rect::from_min_max(
                Pos2::new(x_at(span.start), y),
                Pos2::new(x_at(span.end), y + f.line_h),
            );
            text_painter.rect_filled(r, 2.0, *c);
        }
        // The selection on this line (and its line break).
        if sel_a != sel_b && sel_a.line <= line && line <= sel_b.line {
            let a = if line == sel_a.line { sel_a.col } else { 0 };
            let b = if line == sel_b.line {
                sel_b.col
            } else {
                l.layout.text.len()
            };
            let mut right = x_at(b);
            if line < sel_b.line {
                right += f.cell_w;
            }
            let r = Rect::from_min_max(Pos2::new(x_at(a), y), Pos2::new(right, y + f.line_h));
            text_painter.rect_filled(r, 0.0, sel_bg);
        }
        if let Some(s) = highlight {
            for m in s.find_all_bytes(l.layout.text.as_bytes()) {
                let r = Rect::from_min_max(
                    Pos2::new(x_at(m.start), y),
                    Pos2::new(x_at(m.end).max(x_at(m.start) + 2.0), y + f.line_h),
                );
                text_painter.rect_filled(r, 2.0, match_bg);
            }
        }
        if look.as_ref().is_some_and(|k| k.dim) {
            let mut dim = text_painter.clone();
            dim.multiply_opacity(0.5);
            paint_line_text(&dim, &l.layout, x0, y, &f.font, f.line_h);
        } else {
            paint_line_text(&text_painter, &l.layout, x0, y, &f.font, f.line_h);
        }
        // Control characters are drawn as boxes, as in the editor.
        for cell in &l.layout.specials {
            let r = Rect::from_min_size(
                Pos2::new(x0 + cell.x + 0.5, y + 1.5),
                Vec2::new(cell.width - 1.0, f.line_h - 3.0),
            );
            let warn = visuals.warn_fg_color;
            text_painter.rect_filled(
                r,
                2.0,
                Color32::from_rgba_unmultiplied(warn.r(), warn.g(), warn.b(), 80),
            );
        }
        if l.cut > 0 {
            paint_cut_marker(&text_painter, x0 + l.layout.width, y, f, l.cut, weak);
        }
        let number_color = if line == state.head.line {
            accent
        } else {
            weak
        };
        gutter_painter.text(
            Pos2::new(f.gutter.max.x - f.cell_w, y),
            Align2::RIGHT_TOP,
            (line + 1).to_string(),
            f.font.clone(),
            number_color,
        );
        if line == state.head.line {
            caret = Some(Pos2::new(x_at(state.head.col), y));
        }
    }
    if !drawn.is_empty() {
        let (lo, hi) = (drawn[0], drawn[drawn.len() - 1]);
        let span = hi - lo + 1;
        if state.lines.len() as u64 > 4 * span + 256 {
            state
                .lines
                .retain(|&k, _| k + span >= lo && k <= hi + span || k == state.head.line);
        }
    }
    if focused {
        let since = now - state.last_activity;
        if ((since / BLINK_HALF_PERIOD) as u64).is_multiple_of(2)
            && let Some(p) = caret
        {
            text_painter.line_segment(
                [p, p + Vec2::new(0.0, f.line_h)],
                Stroke::new(2.0, visuals.text_cursor.stroke.color),
            );
        }
        let to_toggle = BLINK_HALF_PERIOD - since.rem_euclid(BLINK_HALF_PERIOD);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(to_toggle));
    }

    // Scrollbars.
    let thumb = visuals.widgets.hovered.bg_fill;
    if rows as usize > f.rows_full {
        let h = (f.vbar.height() * f.rows_full as f32 / rows as f32)
            .max(20.0)
            .min(f.vbar.height());
        let t = (state.top / max_top.max(1.0)) as f32;
        let r = Rect::from_min_size(
            Pos2::new(f.vbar.min.x, f.vbar.min.y + (f.vbar.height() - h) * t),
            Vec2::new(SCROLLBAR_W, h),
        );
        painter.rect_filled(r.shrink(2.0), 3.0, thumb);
    }
    if h_max > 0.0 {
        let total = state.max_width + 4.0 * f.cell_w;
        let w = (f.hbar.width() * f.content.width() / total)
            .max(20.0)
            .min(f.hbar.width());
        let t = (state.x_off / h_max).min(1.0);
        let r = Rect::from_min_size(
            Pos2::new(f.hbar.min.x + (f.hbar.width() - w) * t, f.hbar.min.y),
            Vec2::new(w, SCROLLBAR_W),
        );
        painter.rect_filled(r.shrink(2.0), 3.0, thumb);
    }
}

/// The marker after a line too long to show in full: how much is left out.
fn paint_cut_marker(painter: &egui::Painter, x: f32, y: f32, f: &Frame, cut: u64, color: Color32) {
    let text = format!("… {} more", human_bytes(cut));
    let font = FontId::proportional(f.font.size * 0.7);
    let galley = painter.layout_no_wrap(text, font, color);
    let h = (f.line_h - 4.0).max(galley.size().y);
    let r = Rect::from_min_size(
        Pos2::new(x + f.cell_w, y + (f.line_h - h) / 2.0),
        Vec2::new(galley.size().x + 8.0, h),
    );
    painter.rect_filled(r, h / 2.0, color.gamma_multiply(0.2));
    painter.galley(r.center() - galley.size() / 2.0, galley, color);
}

/// "512 bytes", "12.3 KB", "4.0 GB".
pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if n < 1024 {
        return format!("{n} bytes");
    }
    let mut v = n as f64 / 1024.0;
    let mut unit = 0;
    while v >= 1024.0 && unit + 1 < UNITS.len() {
        v /= 1024.0;
        unit += 1;
    }
    format!("{v:.1} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(human_bytes(10), "10 bytes");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(human_bytes(4 * 1024 * 1024 * 1024), "4.0 GB");
    }

    #[test]
    fn shown_lists_map_rows_and_lines() {
        let all = Shown::All;
        assert_eq!((all.rows(10), all.line(3), all.row_of(7)), (10, Some(3), 7));
        let some = Shown::Lines(Arc::new(vec![2, 5, 9]));
        assert_eq!(some.rows(100), 3);
        assert_eq!(some.line(1), Some(5));
        assert_eq!(some.line(3), None);
        assert_eq!(some.row_of(5), 1);
        assert_eq!(
            some.row_of(6),
            1,
            "a hidden line maps to the shown one before it"
        );
        assert_eq!(some.row_of(0), 0);
    }

    #[test]
    fn all_but_some_lines_map_rows_and_lines() {
        // Lines 0..10 without 2, 3 and 7: rows show 0 1 4 5 6 8 9.
        let shown = Shown::AllExcept {
            excluded: Arc::new(vec![2, 3, 7]),
            decided: None,
        };
        assert_eq!(shown.rows(10), 7);
        let lines: Vec<u64> = (0..7).map(|r| shown.line(r).unwrap()).collect();
        assert_eq!(lines, vec![0, 1, 4, 5, 6, 8, 9]);
        for (row, &line) in lines.iter().enumerate() {
            assert_eq!(shown.row_of(line), row as u64);
        }
        assert_eq!(shown.row_of(3), 1, "a hidden line maps to the one before");
        assert_eq!(shown.row_of(7), 4);
        // While the search is going, only the decided lines count.
        let partial = Shown::AllExcept {
            excluded: Arc::new(vec![2, 3]),
            decided: Some(5),
        };
        assert_eq!(partial.rows(10), 3);
    }
}
