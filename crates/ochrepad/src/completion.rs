//! Where completions come from (PLAN_completion C1–C2): the words and the
//! definitions (what the Function List shows) of the open documents, found
//! on a thread once typing pauses; the words near the caret, read fresh for
//! each request so a word typed a moment ago is offered at once; and the
//! language's keywords. `Completion::source` lends them to the editor
//! widget as a `CompletionSource`.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use ochrepad_core::complete::{self, Candidate, Kind, Source};
use ochrepad_core::{BufferId, Document, Editor};
use ochrepad_syntax::{Registry, Symbol, SymbolKind};

use crate::completion_popup::{CompletionRequest, CompletionSource};

/// Quiet time after an edit before a document's words are counted again.
const PAUSE_SECS: f64 = 0.4;
/// Words shorter than this are not offered (they are quicker to type).
const MIN_WORD: usize = 3;
/// Lines either side of the caret read for each request.
const NEARBY_LINES: usize = 150;
/// Documents larger than this are not counted (their nearby lines still
/// are).
const MAX_DOC_BYTES: usize = 16 << 20;
/// Definitions are looked for in documents up to this size (the syntax
/// parser is slower than counting words).
const MAX_SYMBOL_BYTES: usize = 2 << 20;

/// What was found in one document.
#[derive(Default)]
struct Found {
    words: complete::WordIndex,
    definitions: Vec<Symbol>,
}

struct DocWords {
    /// The revision counted (or being counted).
    revision: Option<u64>,
    found: Arc<Found>,
    /// The revision last seen, and when it was first seen.
    seen: (u64, f64),
    /// The document's language, and the characters besides letters, digits
    /// and `_` its words have.
    language: Option<String>,
    word_chars: Vec<char>,
    /// The file's name, for the popup's details.
    name: Option<String>,
}

/// A counting job and its result.
struct Job {
    buffer: BufferId,
    revision: u64,
    doc: Document,
    word_chars: Vec<char>,
    language: Option<String>,
}
type Done = (BufferId, u64, Found);

/// Completion's sources, kept by the app.
#[derive(Default)]
pub struct Completion {
    docs: HashMap<BufferId, DocWords>,
    jobs: Option<Sender<Job>>,
    done: Option<Receiver<Done>>,
    /// What the user accepted lately, most recent last.
    accepted: Vec<String>,
}

/// Characters besides letters, digits and `_` that words have in
/// `language`.
pub fn word_chars(language: Option<&str>) -> Vec<char> {
    match language.unwrap_or("") {
        "CSS" | "SCSS" | "Sass" | "Less" | "Lisp" | "Clojure" | "Scheme" | "Racket" => vec!['-'],
        "PHP" | "Bourne Again Shell (bash)" => vec!['$'],
        _ => Vec::new(),
    }
}

impl Completion {
    /// Once a frame: counts the words of documents that changed, after a
    /// pause, on the worker thread; drops closed documents. Returns whether
    /// it is waiting for something (the app repaints a little later).
    pub fn tick(
        &mut self,
        editor: &Editor,
        registry: &Arc<Registry>,
        now: f64,
        ctx: &egui::Context,
    ) -> bool {
        if let Some(rx) = &self.done {
            while let Ok((buffer, revision, found)) = rx.try_recv() {
                if let Some(d) = self.docs.get_mut(&buffer)
                    && d.revision == Some(revision)
                {
                    d.found = Arc::new(found);
                }
            }
        }
        self.docs.retain(|b, _| editor.buffer(*b).is_some());
        let mut waiting = false;
        let mut jobs = Vec::new();
        for b in editor.buffers() {
            let revision = b.revision();
            let language = b.language.clone();
            let d = self.docs.entry(b.id()).or_insert_with(|| DocWords {
                revision: None,
                found: Arc::default(),
                // Counted at once the first time.
                seen: (revision, f64::NEG_INFINITY),
                language: None,
                word_chars: Vec::new(),
                name: None,
            });
            d.name = b
                .meta
                .path
                .as_deref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned());
            if d.language != language {
                d.word_chars = word_chars(language.as_deref());
                d.language = language;
                d.revision = None;
            }
            if d.revision == Some(revision) || b.doc().len().0 > MAX_DOC_BYTES {
                continue;
            }
            if d.seen.0 != revision {
                d.seen = (revision, now);
            }
            if now - d.seen.1 < PAUSE_SECS {
                waiting = true;
                continue;
            }
            jobs.push(Job {
                buffer: b.id(),
                revision,
                doc: b.doc().clone(),
                word_chars: d.word_chars.clone(),
                language: d.language.clone(),
            });
        }
        for job in jobs {
            let (buffer, revision) = (job.buffer, job.revision);
            if self.send(job, registry, ctx)
                && let Some(d) = self.docs.get_mut(&buffer)
            {
                d.revision = Some(revision);
            }
        }
        waiting
    }

    /// Hands a job to the worker, starting it the first time.
    fn send(&mut self, job: Job, registry: &Arc<Registry>, ctx: &egui::Context) -> bool {
        if self.jobs.is_none() {
            let (tx, jobs) = channel::<Job>();
            let (done_tx, done) = channel::<Done>();
            let ctx = ctx.clone();
            let registry = registry.clone();
            let spawned = std::thread::Builder::new()
                .name("ochrepad-words".into())
                .spawn(move || {
                    while let Ok(job) = jobs.recv() {
                        let found = find(&registry, &job);
                        if done_tx.send((job.buffer, job.revision, found)).is_err() {
                            break;
                        }
                        ctx.request_repaint();
                    }
                });
            if spawned.is_err() {
                return false;
            }
            self.jobs = Some(tx);
            self.done = Some(done);
        }
        self.jobs.as_ref().is_some_and(|tx| tx.send(job).is_ok())
    }

    /// The words counted for `buffer` (tests).
    pub fn words_of(&self, buffer: BufferId) -> Option<&complete::WordIndex> {
        self.docs.get(&buffer).map(|d| &d.found.words)
    }

    /// The source an editor asks.
    pub fn source(&mut self) -> Sources<'_> {
        Sources { completion: self }
    }
}

/// The words and definitions of a job's document (on the worker).
fn find(registry: &Registry, job: &Job) -> Found {
    let text = job.doc.to_string();
    let mut counts: HashMap<String, u32> = HashMap::new();
    for w in complete::words(&text, &job.word_chars, MIN_WORD) {
        *counts.entry(w.to_owned()).or_default() += 1;
    }
    let definitions = match &job.language {
        Some(language) if text.len() <= MAX_SYMBOL_BYTES => registry
            .symbols(language, &job.doc, &|| false)
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    Found {
        words: complete::WordIndex::new(counts),
        definitions,
    }
}

/// The app's completion sources, lent to one editor for a frame.
pub struct Sources<'a> {
    completion: &'a mut Completion,
}

impl CompletionSource for Sources<'_> {
    fn candidates(&mut self, req: &CompletionRequest<'_>) -> Vec<Candidate> {
        let c = &*self.completion;
        let own = c.docs.get(&req.buffer);
        let chars = own.map(|d| d.word_chars.clone()).unwrap_or_default();
        let pattern = complete::Pattern::new(req.prefix);
        // A cheap look; the popup ranks (and so fully matches) what is kept.
        let matches = |w: &str| pattern.is_empty() || pattern.might_match(w);
        let mut out: HashMap<String, Candidate> = HashMap::new();
        // Words near the caret, nearest first: the text as it is now.
        let line = req.doc.line_of(req.caret);
        let first = line.saturating_sub(NEARBY_LINES);
        let last = (line + NEARBY_LINES).min(req.doc.line_count() - 1);
        for l in first..=last {
            let text = req.doc.line_text(l);
            let distance = l.abs_diff(line) as f32 / NEARBY_LINES as f32;
            for w in complete::words(&text, &chars, MIN_WORD) {
                // The word being typed is not a suggestion for itself.
                if (l == line && w == req.prefix) || !matches(w) {
                    continue;
                }
                offer(
                    &mut out,
                    w,
                    Kind::Word,
                    Source::Word,
                    0.6 * (1.0 - distance),
                );
            }
        }
        // The rest of this document and the other open ones, as counted
        // last, and their definitions.
        for (buffer, d) in &c.docs {
            let is_own = *buffer == req.buffer;
            let most = d.found.words.most().max(1) as f32;
            for (w, count) in d.found.words.matching(&pattern) {
                let frequent = (count as f32).ln_1p() / most.ln_1p();
                let boost = if is_own { 0.25 } else { 0.1 } + 0.15 * frequent;
                offer(&mut out, w, Kind::Word, Source::Word, boost);
            }
            for s in &d.found.definitions {
                let kind = match s.kind {
                    SymbolKind::Function => Kind::Function,
                    SymbolKind::Type => Kind::Type,
                    SymbolKind::Module => Kind::Module,
                    SymbolKind::Heading => continue,
                };
                // Rust's impl blocks are listed as "impl Name".
                let name = s.name.strip_prefix("impl ").unwrap_or(&s.name);
                if !complete::is_identifier(name, &chars) || !matches(name) || name == req.prefix {
                    continue;
                }
                let e = offer(
                    &mut out,
                    name,
                    kind,
                    Source::Definition,
                    if is_own { 0.3 } else { 0.15 },
                );
                if e.detail.is_none() {
                    e.detail = if is_own {
                        Some(format!("line {}", s.line + 1))
                    } else {
                        d.name.clone()
                    };
                }
            }
        }
        // The language's keywords.
        if let Some(language) = own.and_then(|d| d.language.as_deref()) {
            for k in ochrepad_syntax::keywords(language) {
                if matches(k) && k != req.prefix {
                    offer(&mut out, k, Kind::Keyword, Source::Keyword, 0.2);
                }
            }
        }
        // Accepted lately: a little more likely.
        for (i, a) in c.accepted.iter().rev().enumerate().take(50) {
            if let Some(e) = out.get_mut(a) {
                e.boost = (e.boost + 0.3 * (1.0 - i as f32 / 50.0)).min(1.0);
            }
        }
        out.into_values().collect()
    }

    fn word_chars(&self, buffer: BufferId) -> &[char] {
        self.completion
            .docs
            .get(&buffer)
            .map_or(&[], |d| d.word_chars.as_slice())
    }

    fn accepted(&mut self, _buffer: BufferId, text: &str) {
        let a = &mut self.completion.accepted;
        a.retain(|t| t != text);
        a.push(text.to_owned());
        if a.len() > 500 {
            a.remove(0);
        }
    }
}

/// Adds `label` to `out`, or makes the one there more likely; a name's best
/// source wins (a definition over a plain word).
fn offer<'m>(
    out: &'m mut HashMap<String, Candidate>,
    label: &str,
    kind: Kind,
    source: Source,
    boost: f32,
) -> &'m mut Candidate {
    let e = out
        .entry(label.to_owned())
        .or_insert_with(|| Candidate::new(label, kind, source));
    if source < e.source {
        e.kind = kind;
        e.source = source;
    }
    e.boost = e.boost.max(boost);
    e
}
