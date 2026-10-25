//! Where completions come from (PLAN_completion C1–C2): the words of the
//! open documents, counted on a thread once typing pauses, and the words
//! near the caret, read fresh for each request so a word typed a moment ago
//! is offered at once. `Completion::source` lends them to the editor
//! widget as a `CompletionSource`.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use ochrepad_core::complete::{self, Candidate, Kind, Source};
use ochrepad_core::{BufferId, Document, Editor};

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

/// The words of one document, with how often each occurs.
type Words = Arc<complete::WordIndex>;

struct DocWords {
    /// The revision counted (or being counted).
    revision: Option<u64>,
    words: Words,
    /// The revision last seen, and when it was first seen.
    seen: (u64, f64),
    /// Characters besides letters, digits and `_` in this language's words.
    word_chars: Vec<char>,
}

/// A counting job and its result.
type Job = (BufferId, u64, Document, Vec<char>);
type Done = (BufferId, u64, complete::WordIndex);

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
        "PHP" | "Shell-Unix-Generic" | "Bourne Again Shell (bash)" | "PowerShell" => vec!['$'],
        _ => Vec::new(),
    }
}

impl Completion {
    /// Once a frame: counts the words of documents that changed, after a
    /// pause, on the worker thread; drops closed documents. Returns whether
    /// it is waiting for something (the app repaints a little later).
    pub fn tick(&mut self, editor: &Editor, now: f64, ctx: &egui::Context) -> bool {
        if let Some(rx) = &self.done {
            while let Ok((buffer, revision, words)) = rx.try_recv() {
                if let Some(d) = self.docs.get_mut(&buffer)
                    && d.revision == Some(revision)
                {
                    d.words = Arc::new(words);
                }
            }
        }
        self.docs.retain(|b, _| editor.buffer(*b).is_some());
        let mut waiting = false;
        let mut jobs = Vec::new();
        for b in editor.buffers() {
            let revision = b.revision();
            let chars = word_chars(b.language.as_deref());
            let d = self.docs.entry(b.id()).or_insert_with(|| DocWords {
                revision: None,
                words: Arc::default(),
                // Counted at once the first time.
                seen: (revision, f64::NEG_INFINITY),
                word_chars: chars.clone(),
            });
            if d.word_chars != chars {
                d.word_chars = chars;
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
            jobs.push((b.id(), revision, b.doc().clone(), d.word_chars.clone()));
        }
        for job in jobs {
            let (buffer, revision) = (job.0, job.1);
            if self.send(job, ctx)
                && let Some(d) = self.docs.get_mut(&buffer)
            {
                d.revision = Some(revision);
            }
        }
        waiting
    }

    /// Hands a job to the worker, starting it the first time.
    fn send(&mut self, job: Job, ctx: &egui::Context) -> bool {
        if self.jobs.is_none() {
            let (tx, jobs) = channel::<Job>();
            let (done_tx, done) = channel::<Done>();
            let ctx = ctx.clone();
            let spawned = std::thread::Builder::new()
                .name("ochrepad-words".into())
                .spawn(move || {
                    while let Ok((buffer, revision, doc, chars)) = jobs.recv() {
                        let text = doc.to_string();
                        let mut words: HashMap<String, u32> = HashMap::new();
                        for w in complete::words(&text, &chars, MIN_WORD) {
                            *words.entry(w.to_owned()).or_default() += 1;
                        }
                        let index = complete::WordIndex::new(words);
                        if done_tx.send((buffer, revision, index)).is_err() {
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
        self.docs.get(&buffer).map(|d| d.words.as_ref())
    }

    /// The source the editor of `buffer` asks.
    pub fn source(&mut self) -> Sources<'_> {
        Sources { completion: self }
    }
}

/// The app's completion sources, lent to one editor for a frame.
pub struct Sources<'a> {
    completion: &'a mut Completion,
}

impl CompletionSource for Sources<'_> {
    fn candidates(&mut self, req: &CompletionRequest<'_>) -> Vec<Candidate> {
        let c = &*self.completion;
        let chars = c
            .docs
            .get(&req.buffer)
            .map(|d| d.word_chars.clone())
            .unwrap_or_default();
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
                if l == line && w == req.prefix {
                    continue;
                }
                if !matches(w) {
                    continue;
                }
                let boost = 0.6 * (1.0 - distance);
                let e = out
                    .entry(w.to_owned())
                    .or_insert_with(|| Candidate::new(w, Kind::Word, Source::Word));
                e.boost = e.boost.max(boost);
            }
        }
        // The rest of this document and the other open ones, as counted
        // last.
        for (buffer, d) in &c.docs {
            let own = *buffer == req.buffer;
            let most = d.words.most().max(1) as f32;
            for (w, count) in d.words.matching(&pattern) {
                let frequent = (count as f32).ln_1p() / most.ln_1p();
                let boost = if own { 0.25 } else { 0.1 } + 0.15 * frequent;
                let e = out
                    .entry(w.to_owned())
                    .or_insert_with(|| Candidate::new(w, Kind::Word, Source::Word));
                e.boost = e.boost.max(boost);
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
