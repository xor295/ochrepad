//! The open folder's words and definitions, for completion (PLAN_completion
//! C3). A thread walks the folder (ignore files respected), reads each file
//! up to [`MAX_FILE`] and counts its words, then finds the definitions the
//! syntax marks (slower, so after every file's words). It sends a
//! [`Snapshot`] when it has something new, keeps the index up to date
//! (a file saved in ochrepad at once, the rest by a cheap look at sizes and
//! times every [`RESCAN_SECS`]), and hands back a cache for the data folder
//! so reopening the folder is quick.
//!
//! The cache is only compared with what the walk finds: a path in it is
//! never read, so a damaged or tampered cache cannot make ochrepad open
//! anything.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant, SystemTime};

use ochrepad_core::Document;
use ochrepad_core::complete::{self, WordIndex};
use ochrepad_syntax::{Registry, SymbolKind};
use serde::{Deserialize, Serialize};

/// Files larger than this are left out.
pub const MAX_FILE: u64 = 2 << 20;
/// Definitions are looked for in files up to this size (larger ones are
/// mostly generated, and parsing is the slow part).
pub const MAX_DEFINITIONS_FILE: u64 = 256 << 10;
/// At most this many threads look for definitions (one core is left free).
const MAX_WORKERS: usize = 4;
/// At most this many files are indexed.
pub const MAX_FILES: usize = 20_000;
/// How often files changed by other programs are looked for.
pub const RESCAN_SECS: u64 = 30;
/// A cache larger than this is not written.
const MAX_CACHE: usize = 64 << 20;
const CACHE_VERSION: u32 = 1;
/// Words shorter than this are not kept.
const MIN_WORD: usize = 3;

/// A definition found in the folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    pub kind: SymbolKind,
    /// The file, relative to the folder, with `/` between names.
    pub file: String,
    pub line: usize,
}

/// What the index knows, as one value the app can hold while the thread
/// goes on.
#[derive(Debug, Default)]
pub struct Snapshot {
    pub words: WordIndex,
    /// The defined names (with how many definitions each has).
    pub names: WordIndex,
    /// The first definition of each name.
    pub definitions: HashMap<String, Definition>,
    pub files: usize,
    /// The folder had more files than [`MAX_FILES`].
    pub truncated: bool,
}

/// How far the thread has got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    Words { done: usize, total: usize },
    Definitions { done: usize, total: usize },
    Ready,
}

enum Command {
    /// A file was saved (or created) in ochrepad.
    Saved(PathBuf),
    /// Look for changes now (the window came back into focus).
    Rescan,
}

enum Update {
    Progress(Progress),
    Snapshot(Arc<Snapshot>),
    /// The cache to write, as bytes.
    Cache(Vec<u8>),
}

/// How many of the best words, and of the best names, a query answers.
pub const QUERY_LIMIT: usize = 200;

/// The folder's best matches for a pattern (see [`FolderIndex::ask`]).
pub struct FolderMatches {
    pub pattern: String,
    /// The snapshot they were found in.
    pub snapshot: Arc<Snapshot>,
    /// Words, with how often each occurs.
    pub words: Vec<(String, u32)>,
    /// Defined names, with their first definition.
    pub names: Vec<(String, Definition)>,
}

/// The index of one folder, as the app holds it.
pub struct FolderIndex {
    root: PathBuf,
    commands: Sender<Command>,
    updates: Receiver<Update>,
    pub snapshot: Option<Arc<Snapshot>>,
    pub progress: Progress,
    cache_file: Option<PathBuf>,
    cache_out: Option<Vec<u8>>,
    /// Set when this is dropped: the thread stops, even in a long pass.
    stop: Arc<AtomicBool>,
    /// Patterns for the query thread, and its answers: matching a large
    /// folder's words can take longer than a frame.
    queries: Sender<(Arc<Snapshot>, String)>,
    answers: Receiver<FolderMatches>,
    /// The latest answer, and the pattern asked last.
    pub matches: Option<FolderMatches>,
    asked: Option<(String, Arc<Snapshot>)>,
    /// Counts answers (the completion popup asks again when it changes).
    pub answered: u64,
}

impl Drop for FolderIndex {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl FolderIndex {
    /// Starts indexing `root`, with the cache at `cache_file` (read here on
    /// the thread, written by the app through its worker).
    pub fn open(
        root: PathBuf,
        cache_file: Option<PathBuf>,
        registry: Arc<Registry>,
        ctx: egui::Context,
    ) -> Self {
        let (commands, command_rx) = channel();
        let (update_tx, updates) = channel();
        let (thread_root, thread_cache) = (root.clone(), cache_file.clone());
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let (queries, query_rx) = channel::<(Arc<Snapshot>, String)>();
        let (answer_tx, answers) = channel();
        let query_ctx = ctx.clone();
        let _ = std::thread::Builder::new()
            .name("ochrepad-folder-query".into())
            .spawn(move || {
                // Only the latest question matters.
                while let Ok(mut q) = query_rx.recv() {
                    while let Ok(newer) = query_rx.try_recv() {
                        q = newer;
                    }
                    if answer_tx.send(answer(q.0, q.1)).is_err() {
                        return;
                    }
                    query_ctx.request_repaint();
                }
            });
        let _ = std::thread::Builder::new()
            .name("ochrepad-folder-index".into())
            .spawn(move || {
                let mut indexer = Indexer {
                    root: thread_root,
                    registry,
                    files: HashMap::new(),
                    interner: Interner::default(),
                    truncated: false,
                    updates: update_tx,
                    ctx,
                    stop: thread_stop,
                };
                indexer.run(thread_cache.as_deref(), &command_rx);
            });
        Self {
            root,
            commands,
            updates,
            snapshot: None,
            progress: Progress::Words { done: 0, total: 0 },
            cache_file,
            cache_out: None,
            stop,
            queries,
            answers,
            matches: None,
            asked: None,
            answered: 0,
        }
    }

    /// Asks for the folder's best matches of `pattern` (answered on a
    /// thread; [`Self::poll`] takes the answer). Asking again for the same
    /// pattern in the same snapshot does nothing.
    pub fn ask(&mut self, pattern: &str) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        if self
            .asked
            .as_ref()
            .is_some_and(|(p, s)| p == pattern && Arc::ptr_eq(s, snapshot))
        {
            return;
        }
        let _ = self.queries.send((snapshot.clone(), pattern.to_owned()));
        self.asked = Some((pattern.to_owned(), snapshot.clone()));
    }

    /// Whether the latest answer is for `pattern` in the current snapshot.
    pub fn answered_for(&self, pattern: &str) -> bool {
        self.matches.as_ref().is_some_and(|m| {
            m.pattern == pattern
                && self
                    .snapshot
                    .as_ref()
                    .is_some_and(|s| Arc::ptr_eq(s, &m.snapshot))
        })
    }

    /// Whether a question is still unanswered.
    pub fn waiting(&self) -> bool {
        self.asked.as_ref().is_some_and(|(pattern, snapshot)| {
            !self
                .matches
                .as_ref()
                .is_some_and(|m| &m.pattern == pattern && Arc::ptr_eq(&m.snapshot, snapshot))
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Takes what the thread sent. Returns whether anything changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(u) = self.updates.try_recv() {
            changed = true;
            match u {
                Update::Progress(p) => self.progress = p,
                Update::Snapshot(s) => self.snapshot = Some(s),
                Update::Cache(bytes) => self.cache_out = Some(bytes),
            }
        }
        while let Ok(m) = self.answers.try_recv() {
            self.matches = Some(m);
            self.answered += 1;
            changed = true;
        }
        changed
    }

    /// A file in the folder was saved: index it again now.
    pub fn saved(&self, path: &Path) {
        let _ = self.commands.send(Command::Saved(path.to_path_buf()));
    }

    /// Looks for files changed by other programs now.
    pub fn rescan(&self) {
        let _ = self.commands.send(Command::Rescan);
    }

    /// The cache to write, and where (once per time the thread sends one).
    pub fn take_cache(&mut self) -> Option<(PathBuf, Vec<u8>)> {
        let bytes = self.cache_out.take()?;
        Some((self.cache_file.clone()?, bytes))
    }
}

/// The query thread's work: the best words and names for `pattern`.
fn answer(snapshot: Arc<Snapshot>, pattern: String) -> FolderMatches {
    let prepared = complete::Pattern::new(&pattern);
    let words = snapshot
        .words
        .best(&prepared, QUERY_LIMIT)
        .into_iter()
        .map(|(w, c)| (w.to_owned(), c))
        .collect();
    let names = snapshot
        .names
        .best(&prepared, QUERY_LIMIT)
        .into_iter()
        .filter_map(|(n, _)| Some((n.to_owned(), snapshot.definitions.get(n)?.clone())))
        .collect();
    FolderMatches {
        pattern,
        snapshot,
        words,
        names,
    }
}

/// Words are stored once, as numbers in each file's list.
#[derive(Default)]
struct Interner {
    words: Vec<String>,
    ids: HashMap<String, u32>,
}

impl Interner {
    fn id(&mut self, w: &str) -> u32 {
        if let Some(&id) = self.ids.get(w) {
            return id;
        }
        let id = self.words.len() as u32;
        self.words.push(w.to_owned());
        self.ids.insert(w.to_owned(), id);
        id
    }
}

/// What one file holds.
struct FileEntry {
    size: u64,
    modified: (u64, u32),
    /// (word id, count).
    words: Vec<(u32, u32)>,
    /// `None` until looked for.
    definitions: Option<Vec<(String, SymbolKind, usize)>>,
}

struct Indexer {
    root: PathBuf,
    registry: Arc<Registry>,
    /// By path relative to the root, with `/`.
    files: HashMap<String, FileEntry>,
    interner: Interner,
    truncated: bool,
    updates: Sender<Update>,
    ctx: egui::Context,
    stop: Arc<AtomicBool>,
}

/// The definitions in the file at `path` (none for a file too large, not
/// text, or in a language without them); `None` when `stop` was set.
fn definitions_of(
    registry: &Registry,
    path: &Path,
    stop: &AtomicBool,
) -> Option<Vec<(String, SymbolKind, usize)>> {
    if stop.load(Ordering::Relaxed) {
        return None;
    }
    let small = std::fs::metadata(path).is_ok_and(|m| m.len() <= MAX_DEFINITIONS_FILE);
    let Some(decoded) = small.then(|| ochrepad_fileio::load(path).ok()).flatten() else {
        return Some(Vec::new());
    };
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    let first = decoded.text.lines().next().unwrap_or("");
    let language = registry.detect(name.as_deref(), first);
    if language == ochrepad_syntax::PLAIN_TEXT {
        return Some(Vec::new());
    }
    let doc = Document::new(&decoded.text);
    let symbols = registry.symbols(&language, &doc, &|| stop.load(Ordering::Relaxed))?;
    Some(
        symbols
            .into_iter()
            .filter(|s| s.kind != SymbolKind::Heading)
            .map(|s| {
                let name = s.name.strip_prefix("impl ").unwrap_or(&s.name).to_owned();
                (name, s.kind, s.line)
            })
            .filter(|(name, _, _)| complete::is_identifier(name, &[]))
            .collect(),
    )
}

fn stamp(meta: &std::fs::Metadata) -> (u64, u32) {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or((0, 0), |d| (d.as_secs(), d.subsec_nanos()))
}

fn kind_code(k: SymbolKind) -> u8 {
    match k {
        SymbolKind::Function => 0,
        SymbolKind::Type => 1,
        SymbolKind::Module => 2,
        SymbolKind::Heading => 3,
    }
}

fn kind_of(code: u8) -> Option<SymbolKind> {
    Some(match code {
        0 => SymbolKind::Function,
        1 => SymbolKind::Type,
        2 => SymbolKind::Module,
        3 => SymbolKind::Heading,
        _ => return None,
    })
}

/// The cache file's contents.
#[derive(Serialize, Deserialize)]
struct Cache {
    version: u32,
    words: Vec<String>,
    files: Vec<CachedFile>,
}

#[derive(Serialize, Deserialize)]
struct CachedFile {
    path: String,
    size: u64,
    secs: u64,
    nanos: u32,
    /// Word ids and counts, alternating.
    words: Vec<u32>,
    /// (name, kind, line); absent when not looked for yet.
    definitions: Option<Vec<(String, u8, usize)>>,
}

impl Indexer {
    fn send(&self, u: Update) {
        if self.updates.send(u).is_ok() {
            self.ctx.request_repaint();
        }
    }

    fn run(&mut self, cache_file: Option<&Path>, commands: &Receiver<Command>) {
        let cached = cache_file.map(|p| self.load_cache(p)).unwrap_or_default();
        self.full_pass(cached);
        loop {
            match commands.recv_timeout(Duration::from_secs(RESCAN_SECS)) {
                Ok(Command::Saved(path)) => {
                    if let Some(rel) = self.relative(&path) {
                        self.index_file(&rel, &path);
                        self.find_definitions(&rel);
                        self.publish();
                    }
                }
                Ok(Command::Rescan) | Err(RecvTimeoutError::Timeout) => {
                    if self.rescan() {
                        self.publish();
                        self.send_cache();
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    /// `path` relative to the root, with `/`, if it is inside it.
    fn relative(&self, path: &Path) -> Option<String> {
        let rel = path.strip_prefix(&self.root).ok()?;
        let parts: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        (!parts.is_empty()).then(|| parts.join("/"))
    }

    /// The cached entries, keyed by path (checked: a damaged cache is
    /// dropped as a whole).
    fn load_cache(&mut self, file: &Path) -> HashMap<String, FileEntry> {
        let Ok(Some(cache)) = ochrepad_fileio::load_json::<Cache>(file) else {
            return HashMap::new();
        };
        if cache.version != CACHE_VERSION {
            return HashMap::new();
        }
        let ids: Vec<u32> = cache.words.iter().map(|w| self.interner.id(w)).collect();
        let mut out = HashMap::new();
        for f in cache.files {
            if f.words.len() % 2 != 0 {
                return HashMap::new();
            }
            let mut words = Vec::with_capacity(f.words.len() / 2);
            for pair in f.words.chunks(2) {
                let Some(&id) = ids.get(pair[0] as usize) else {
                    return HashMap::new();
                };
                words.push((id, pair[1]));
            }
            let definitions = f.definitions.map(|d| {
                d.into_iter()
                    .filter_map(|(name, k, line)| Some((name, kind_of(k)?, line)))
                    .collect()
            });
            out.insert(
                f.path,
                FileEntry {
                    size: f.size,
                    modified: (f.secs, f.nanos),
                    words,
                    definitions,
                },
            );
        }
        out
    }

    /// The files the walk finds now: (relative path, absolute path, size,
    /// time).
    fn walk(&mut self) -> Vec<(String, PathBuf, u64, (u64, u32))> {
        let options = ochrepad_fileio::tree::WalkOptions {
            subfolders: true,
            include_ignored: false,
            filters: Vec::new(),
        };
        let Ok(walk) = ochrepad_fileio::tree::walk_files(&self.root, &options) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        self.truncated = false;
        for path in walk {
            if found.len() >= MAX_FILES {
                self.truncated = true;
                break;
            }
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            if !meta.is_file() || meta.len() > MAX_FILE {
                continue;
            }
            if let Some(rel) = self.relative(&path) {
                found.push((rel, path, meta.len(), stamp(&meta)));
            }
        }
        found
    }

    /// The first pass: every file's words (from the cache when unchanged),
    /// then every file's definitions.
    fn full_pass(&mut self, mut cached: HashMap<String, FileEntry>) {
        let found = self.walk();
        let total = found.len();
        let mut last_sent = Instant::now();
        for (done, (rel, path, size, modified)) in found.iter().enumerate() {
            if self.stopped() {
                return;
            }
            match cached.remove(rel) {
                Some(e) if e.size == *size && e.modified == *modified => {
                    self.files.insert(rel.clone(), e);
                }
                _ => self.index_file(rel, path),
            }
            if last_sent.elapsed() > Duration::from_millis(250) {
                last_sent = Instant::now();
                self.send(Update::Progress(Progress::Words { done, total }));
            }
        }
        self.publish();
        let pending: Vec<(String, PathBuf)> = self
            .files
            .iter()
            .filter(|(_, e)| e.definitions.is_none())
            .map(|(rel, _)| (rel.clone(), self.path_of(rel)))
            .collect();
        let total = pending.len();
        let workers = std::thread::available_parallelism()
            .map_or(1, |n| n.get().saturating_sub(1))
            .clamp(1, MAX_WORKERS);
        let next = AtomicUsize::new(0);
        let (tx, rx) = channel();
        let (registry, stop) = (self.registry.clone(), self.stop.clone());
        std::thread::scope(|scope| {
            for _ in 0..workers {
                let (tx, next, pending) = (tx.clone(), &next, &pending);
                let (registry, stop) = (&registry, &stop);
                scope.spawn(move || {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some((rel, path)) = pending.get(i) else {
                            break;
                        };
                        let Some(found) = definitions_of(registry, path, stop) else {
                            break;
                        };
                        if tx.send((rel, found)).is_err() {
                            break;
                        }
                    }
                });
            }
            drop(tx);
            let mut last_published = Instant::now();
            for (done, (rel, found)) in rx.iter().enumerate() {
                if let Some(e) = self.files.get_mut(rel) {
                    e.definitions = Some(found);
                }
                if last_sent.elapsed() > Duration::from_millis(250) {
                    last_sent = Instant::now();
                    self.send(Update::Progress(Progress::Definitions { done, total }));
                }
                if last_published.elapsed() > Duration::from_secs(2) {
                    last_published = Instant::now();
                    self.publish();
                }
            }
        });
        if self.stopped() {
            return;
        }
        self.publish();
        self.send(Update::Progress(Progress::Ready));
        self.send_cache();
    }

    /// Reads `path` and counts its words (its definitions are looked for
    /// later). A file that cannot be read as text is kept with no words.
    fn index_file(&mut self, rel: &str, path: &Path) {
        let Ok(meta) = std::fs::metadata(path) else {
            self.files.remove(rel);
            return;
        };
        if !meta.is_file() || meta.len() > MAX_FILE {
            self.files.remove(rel);
            return;
        }
        let mut counts: HashMap<u32, u32> = HashMap::new();
        if let Ok(decoded) = ochrepad_fileio::load(path) {
            for w in complete::words(&decoded.text, &[], MIN_WORD) {
                *counts.entry(self.interner.id(w)).or_default() += 1;
            }
        }
        self.files.insert(
            rel.to_owned(),
            FileEntry {
                size: meta.len(),
                modified: stamp(&meta),
                words: counts.into_iter().collect(),
                definitions: None,
            },
        );
    }

    /// The definitions of `rel` (read again: the words pass does not keep
    /// the text).
    fn find_definitions(&mut self, rel: &str) {
        let path = self.path_of(rel);
        if let Some(found) = definitions_of(&self.registry, &path, &self.stop)
            && let Some(e) = self.files.get_mut(rel)
        {
            e.definitions = Some(found);
        }
    }

    /// `rel`'s absolute path.
    fn path_of(&self, rel: &str) -> PathBuf {
        rel.split('/')
            .fold(self.root.clone(), |p, part| p.join(part))
    }

    /// The app dropped the index (the folder closed, or the app quit).
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Looks for files added, changed or removed since; returns whether any
    /// were.
    fn rescan(&mut self) -> bool {
        let found = self.walk();
        let mut changed = false;
        let mut present = std::collections::HashSet::new();
        for (rel, path, size, modified) in found {
            present.insert(rel.clone());
            let same = self
                .files
                .get(&rel)
                .is_some_and(|e| e.size == size && e.modified == modified);
            if !same {
                self.index_file(&rel, &path);
                self.find_definitions(&rel);
                changed = true;
            }
        }
        let before = self.files.len();
        self.files.retain(|rel, _| present.contains(rel));
        changed || self.files.len() != before
    }

    /// Sends a snapshot of everything indexed.
    fn publish(&self) {
        let mut totals: HashMap<u32, u32> = HashMap::new();
        let mut names: HashMap<String, u32> = HashMap::new();
        let mut definitions: HashMap<String, Definition> = HashMap::new();
        let mut rels: Vec<&String> = self.files.keys().collect();
        rels.sort();
        for rel in rels {
            let e = &self.files[rel];
            for &(id, count) in &e.words {
                *totals.entry(id).or_default() += count;
            }
            for (name, kind, line) in e.definitions.iter().flatten() {
                *names.entry(name.clone()).or_default() += 1;
                definitions
                    .entry(name.clone())
                    .or_insert_with(|| Definition {
                        kind: *kind,
                        file: rel.clone(),
                        line: *line,
                    });
            }
        }
        let words = WordIndex::new(
            totals
                .into_iter()
                .filter_map(|(id, c)| Some((self.interner.words.get(id as usize)?.clone(), c))),
        );
        self.send(Update::Snapshot(Arc::new(Snapshot {
            words,
            names: WordIndex::new(names),
            definitions,
            files: self.files.len(),
            truncated: self.truncated,
        })));
    }

    /// Sends the cache's bytes (unless they would be too large).
    fn send_cache(&self) {
        // Only the words still used, renumbered.
        let mut used: HashMap<u32, u32> = HashMap::new();
        let mut words = Vec::new();
        let mut files = Vec::with_capacity(self.files.len());
        for (rel, e) in &self.files {
            let mut flat = Vec::with_capacity(e.words.len() * 2);
            for &(id, count) in &e.words {
                let new = *used.entry(id).or_insert_with(|| {
                    words.push(self.interner.words[id as usize].clone());
                    (words.len() - 1) as u32
                });
                flat.push(new);
                flat.push(count);
            }
            files.push(CachedFile {
                path: rel.clone(),
                size: e.size,
                secs: e.modified.0,
                nanos: e.modified.1,
                words: flat,
                definitions: e.definitions.as_ref().map(|d| {
                    d.iter()
                        .map(|(n, k, l)| (n.clone(), kind_code(*k), *l))
                        .collect()
                }),
            });
        }
        let cache = Cache {
            version: CACHE_VERSION,
            words,
            files,
        };
        if let Ok(bytes) = serde_json::to_vec(&cache)
            && bytes.len() <= MAX_CACHE
        {
            self.send(Update::Cache(bytes));
        }
    }
}

/// The cache file for `root` in `dir`: named by a hash of the folder's
/// path, so no part of the path is used as a file name.
pub fn cache_file(dir: &Path, root: &Path) -> PathBuf {
    let key = ochrepad_core::sha256_hex(&Document::new(&root.to_string_lossy()));
    dir.join(format!("{key}.json"))
}
