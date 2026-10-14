//! Syntax highlighting: syntect with two-face's extra syntaxes and themes,
//! plus an incremental per-buffer highlighter with checkpoints.
//!
//! The highlighter keeps a `(ParseState, HighlightState)` checkpoint about
//! every [`CHECKPOINT_INTERVAL`] lines. An edit invalidates checkpoints from
//! the edited line on; a background pass re-highlights forward from the last
//! good checkpoint and stops as soon as a recomputed checkpoint equals the
//! old one past every edit. The caller runs that pass in small time slices.

pub mod comments;

pub use comments::comment_syntax;

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ochrepad_core::{Document, LineEdit};
use syntect::highlighting::{FontStyle, HighlightIterator, HighlightState, Highlighter, Theme};
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};
use two_face::theme::{EmbeddedLazyThemeSet, EmbeddedThemeName};

/// Lines between checkpoints.
pub const CHECKPOINT_INTERVAL: usize = 128;
/// A visible line further than this past the nearest checkpoint is
/// highlighted approximately, from a fresh parse state, until the background
/// pass reaches it.
pub const MAX_CATCH_UP: usize = CHECKPOINT_INTERVAL;
/// The background pass keeps the spans of up to this many lines, so jumping
/// around a highlighted file does not re-parse from checkpoints.
pub const MAX_CACHED_LINES: usize = 100_000;
/// Files larger than this are not highlighted (PLAN M4).
pub const DEFAULT_SIZE_LIMIT: usize = 10 * 1024 * 1024;
/// Lines longer than this (in bytes) are not highlighted: they show in the
/// default colour, and the parser's state carries over them as if they were
/// not there. Parsing a megabyte of minified code takes seconds.
pub const MAX_HIGHLIGHT_LINE: usize = 20_000;
pub const PLAIN_TEXT: &str = "Plain Text";

/// A styled byte range within one line (never including its `\n`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    /// Straight (unmultiplied) RGBA.
    pub color: [u8; 4],
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

pub type LineSpans = Arc<[Span]>;

/// All syntaxes and themes, loaded once.
pub struct Registry {
    syntaxes: SyntaxSet,
    themes: EmbeddedLazyThemeSet,
}

impl Registry {
    pub fn load() -> Self {
        Self {
            syntaxes: two_face::syntax::extra_newlines(),
            themes: two_face::theme::extra(),
        }
    }

    /// Names of every selectable language, sorted.
    pub fn language_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .syntaxes
            .syntaxes()
            .iter()
            .filter(|s| !s.hidden)
            .map(|s| s.name.clone())
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        names.dedup();
        names
    }

    /// Language by file extension (or whole file name, e.g. `Makefile`),
    /// then by the first line (shebang, mode line), else plain text.
    pub fn detect(&self, file_name: Option<&str>, first_line: &str) -> String {
        let by_name = file_name.and_then(|name| {
            Path::new(name)
                .extension()
                .and_then(|e| e.to_str())
                .and_then(|e| self.syntaxes.find_syntax_by_extension(e))
                .or_else(|| self.syntaxes.find_syntax_by_extension(name))
        });
        by_name
            .or_else(|| self.syntaxes.find_syntax_by_first_line(first_line))
            .map_or_else(|| PLAIN_TEXT.to_owned(), |s| s.name.clone())
    }

    pub fn theme_names() -> Vec<&'static str> {
        EmbeddedLazyThemeSet::theme_names()
            .iter()
            .map(|t| t.as_name())
            .collect()
    }

    pub fn default_theme(dark: bool) -> &'static str {
        if dark {
            EmbeddedThemeName::OneHalfDark.as_name()
        } else {
            EmbeddedThemeName::OneHalfLight.as_name()
        }
    }

    fn theme(&self, name: &str) -> &Theme {
        let id = EmbeddedLazyThemeSet::theme_names()
            .iter()
            .find(|t| t.as_name() == name)
            .copied()
            .unwrap_or(EmbeddedThemeName::OneHalfDark);
        self.themes.get(id)
    }

    fn syntax(&self, name: &str) -> &SyntaxReference {
        self.syntaxes
            .find_syntax_by_name(name)
            .unwrap_or_else(|| self.syntaxes.find_syntax_plain_text())
    }
}

#[derive(Clone)]
struct State {
    line: usize,
    parse: ParseState,
    highlight: HighlightState,
}

/// Incremental highlighter for one buffer.
pub struct BufferHighlighter {
    syntax: String,
    theme: String,
    /// Sorted by line; always has one at line 0. Those before the pass
    /// position are verified; those at or after it are stale.
    checkpoints: Vec<State>,
    /// The background pass: the state at the start of `pass.line`. `None`
    /// when every line is up to date.
    pass: Option<State>,
    /// A stale checkpoint at or before this line cannot end the pass: the
    /// text after it has changed, or (when an edit interrupted a pass) the
    /// checkpoints before it belong to a newer text than those after it.
    dirty_until: usize,
    /// Whether some earlier pass reached the end of the document, so stale
    /// checkpoints cover everything after the current pass. Without that the
    /// pass must not stop early.
    covered_all: bool,
    /// Spans by line: final before the pass position; after it, spans from
    /// before the latest edits, shifted to their new lines (usually still
    /// right, and proven right if the pass converges).
    cache: LineCache,
    /// Spans of shown lines the pass has not verified yet.
    tentative: HashMap<usize, LineSpans>,
    error: Option<String>,
    interval: usize,
    max_catch_up: usize,
    /// Lines highlighted by `work`, for tests and diagnostics.
    pub lines_processed: u64,
}

impl BufferHighlighter {
    pub fn new(reg: &Registry, syntax: &str, theme: &str) -> Self {
        let mut h = Self {
            syntax: syntax.to_owned(),
            theme: theme.to_owned(),
            checkpoints: Vec::new(),
            pass: None,
            dirty_until: 0,
            covered_all: false,
            cache: LineCache::default(),
            tentative: HashMap::new(),
            error: None,
            interval: CHECKPOINT_INTERVAL,
            max_catch_up: MAX_CATCH_UP,
            lines_processed: 0,
        };
        h.reset(reg);
        h
    }

    /// Test hook: smaller checkpoint spacing and catch-up distance exercise
    /// convergence and approximate highlighting on small documents.
    #[doc(hidden)]
    pub fn set_limits(&mut self, interval: usize, max_catch_up: usize) {
        self.interval = interval.max(1);
        self.max_catch_up = max_catch_up;
    }

    #[doc(hidden)]
    pub fn debug_state(&self) -> String {
        format!(
            "checkpoints={:?} pass={:?} dirty_until={} covered_all={} cached={:?}",
            self.checkpoints.iter().map(|c| c.line).collect::<Vec<_>>(),
            self.pass.as_ref().map(|p| p.line),
            self.dirty_until,
            self.covered_all,
            {
                (0..self.cache.lines.len())
                    .filter(|&l| self.cache.get(l).is_some())
                    .collect::<Vec<_>>()
            }
        )
    }

    pub fn syntax(&self) -> &str {
        &self.syntax
    }

    pub fn theme(&self) -> &str {
        &self.theme
    }

    pub fn set_syntax(&mut self, reg: &Registry, syntax: &str) {
        if self.syntax != syntax {
            self.syntax = syntax.to_owned();
            self.reset(reg);
        }
    }

    pub fn set_theme(&mut self, reg: &Registry, theme: &str) {
        if self.theme != theme {
            self.theme = theme.to_owned();
            self.reset(reg);
        }
    }

    /// A parse error stops highlighting for this buffer; the text stays plain.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Whether the background pass still has work to do.
    pub fn is_busy(&self) -> bool {
        self.pass.is_some()
    }

    fn reset(&mut self, reg: &Registry) {
        let hl = Highlighter::new(reg.theme(&self.theme));
        let start = State {
            line: 0,
            parse: ParseState::new(reg.syntax(&self.syntax)),
            highlight: HighlightState::new(&hl, ScopeStack::new()),
        };
        self.checkpoints = vec![start.clone()];
        self.pass = Some(start);
        self.dirty_until = 0;
        self.covered_all = false;
        self.cache.clear();
        self.tentative.clear();
        self.error = None;
    }

    fn fresh_state(&self, reg: &Registry, line: usize) -> State {
        let hl = Highlighter::new(reg.theme(&self.theme));
        State {
            line,
            parse: ParseState::new(reg.syntax(&self.syntax)),
            highlight: HighlightState::new(&hl, ScopeStack::new()),
        }
    }

    /// The last checkpoint at or before `line` (there is always one at 0).
    fn checkpoint_before(&self, line: usize) -> &State {
        let i = self.checkpoints.partition_point(|c| c.line <= line);
        &self.checkpoints[i.saturating_sub(1)]
    }

    /// Updates checkpoints and caches for edits reported by the buffer.
    pub fn apply_edits(&mut self, edits: &[LineEdit]) {
        for e in edits {
            let start = e.start;
            self.tentative.retain(|&l, _| l < start);
            if e.is_everything() {
                self.cache.truncate(start);
                self.checkpoints.retain(|c| c.line <= start);
                self.covered_all = false;
                self.dirty_until = usize::MAX;
                if self.pass.as_ref().is_none_or(|p| p.line > start) {
                    self.pass = Some(self.checkpoint_before(start).clone());
                }
                continue;
            }
            self.cache.replace_lines(start, e.old_end, e.new_end);
            let delta = e.new_end as isize - e.old_end as isize;
            let shift = |line: usize| (line as isize + delta) as usize;
            // Checkpoints inside the replaced lines are gone; later ones move.
            self.checkpoints
                .retain(|c| c.line <= start || c.line > e.old_end);
            for c in &mut self.checkpoints {
                if c.line > e.old_end {
                    c.line = shift(c.line);
                }
            }
            if self.dirty_until != usize::MAX {
                if self.dirty_until > e.old_end {
                    self.dirty_until = shift(self.dirty_until);
                } else if self.dirty_until > start {
                    self.dirty_until = e.new_end;
                }
                self.dirty_until = self.dirty_until.max(e.new_end);
            }
            match &self.pass {
                Some(p) if p.line <= start => {}
                pass => {
                    // An interrupted pass had already rewritten the checkpoints
                    // before its position for the previous text, while those
                    // after it are older still: do not stop before it.
                    if let Some(p) = pass {
                        let at = if p.line > e.old_end {
                            shift(p.line)
                        } else {
                            e.new_end + 1
                        };
                        if self.dirty_until != usize::MAX {
                            self.dirty_until = self.dirty_until.max(at.saturating_sub(1));
                        }
                    }
                    self.pass = Some(self.checkpoint_before(start).clone());
                }
            }
        }
    }

    /// Runs the background pass for up to `budget`. Lines in `viewport` that
    /// it passes are cached. Returns whether work remains.
    pub fn work(
        &mut self,
        reg: &Registry,
        doc: &Document,
        viewport: Range<usize>,
        budget: Duration,
    ) -> bool {
        let deadline = Instant::now() + budget;
        // The clock is cheap next to parsing even a short line.
        self.work_while(reg, doc, viewport, |_| Instant::now() < deadline)
    }

    /// Like `work`, but processes at most `max_lines` lines.
    pub fn work_lines(
        &mut self,
        reg: &Registry,
        doc: &Document,
        viewport: Range<usize>,
        max_lines: usize,
    ) -> bool {
        self.work_while(reg, doc, viewport, |done| done < max_lines)
    }

    fn work_while(
        &mut self,
        reg: &Registry,
        doc: &Document,
        viewport: Range<usize>,
        mut keep_going: impl FnMut(usize) -> bool,
    ) -> bool {
        let Some(mut s) = self.pass.take() else {
            return false;
        };
        let syntax_set = &reg.syntaxes;
        let hl = Highlighter::new(reg.theme(&self.theme));
        let line_count = doc.line_count();
        let mut done = 0;
        loop {
            if s.line >= line_count {
                self.checkpoints.retain(|c| c.line < line_count);
                self.cache.truncate(line_count);
                self.finish();
                return false;
            }
            let i = self.checkpoints.partition_point(|c| c.line < s.line);
            match self.checkpoints.get_mut(i) {
                Some(c) if c.line == s.line => {
                    if self.covered_all
                        && s.line > self.dirty_until
                        && c.parse == s.parse
                        && c.highlight == s.highlight
                    {
                        // Converged: everything after here is unchanged.
                        self.finish();
                        return false;
                    }
                    c.parse = s.parse.clone();
                    c.highlight = s.highlight.clone();
                }
                _ => {
                    let prev = if i > 0 {
                        self.checkpoints[i - 1].line
                    } else {
                        0
                    };
                    if s.line - prev >= self.interval {
                        self.checkpoints.insert(i, s.clone());
                    }
                }
            }
            match highlight_line(&mut s, doc, syntax_set, &hl) {
                Ok(spans) => {
                    self.tentative.remove(&s.line);
                    if self.cache.count < MAX_CACHED_LINES || viewport.contains(&s.line) {
                        self.cache.insert(s.line, spans);
                    } else {
                        self.cache.remove(s.line);
                    }
                }
                Err(e) => {
                    self.error = Some(e);
                    self.pass = None;
                    return false;
                }
            }
            s.line += 1;
            done += 1;
            self.lines_processed += 1;
            if !keep_going(done) {
                self.pass = Some(s);
                return true;
            }
        }
    }

    fn finish(&mut self) {
        self.covered_all = true;
        self.pass = None;
        self.dirty_until = 0;
        // Some tentative spans were approximations; recompute exactly.
        self.tentative.clear();
    }

    /// Spans for each line in `range` (clamped to the document). Lines the
    /// pass has not verified yet are highlighted tentatively from the nearest
    /// checkpoint, or from a fresh state if that is more than
    /// [`MAX_CATCH_UP`] lines back.
    pub fn line_spans(
        &mut self,
        reg: &Registry,
        doc: &Document,
        range: Range<usize>,
    ) -> Vec<Option<LineSpans>> {
        let range = range.start.min(doc.line_count())..range.end.min(doc.line_count());
        let mut out: Vec<Option<LineSpans>> = range
            .clone()
            .map(|l| {
                self.cache
                    .get(l)
                    .or_else(|| self.tentative.get(&l))
                    .cloned()
            })
            .collect();
        if self.error.is_some() {
            return out;
        }
        let Some(first_missing) = range.clone().find(|l| out[l - range.start].is_none()) else {
            return out;
        };
        let verified_before = self.pass.as_ref().map_or(usize::MAX, |p| p.line);
        let cp = self.checkpoint_before(first_missing);
        // Only lines the pass has not reached may be approximated; before it
        // the nearest checkpoint is reliable and close.
        let (mut s, exact) =
            if first_missing >= verified_before && first_missing - cp.line > self.max_catch_up {
                (self.fresh_state(reg, first_missing), false)
            } else {
                (cp.clone(), cp.line < verified_before)
            };
        let hl = Highlighter::new(reg.theme(&self.theme));
        while s.line < range.end {
            let line = s.line;
            match highlight_line(&mut s, doc, &reg.syntaxes, &hl) {
                Ok(spans) => {
                    if line >= range.start && out[line - range.start].is_none() {
                        if exact && line < verified_before {
                            self.cache.insert(line, spans.clone());
                        } else {
                            self.tentative.insert(line, spans.clone());
                        }
                        out[line - range.start] = Some(spans);
                    }
                }
                Err(e) => {
                    self.error = Some(e);
                    self.pass = None;
                    return out;
                }
            }
            s.line += 1;
        }
        if self.tentative.len() > 10_000 {
            self.tentative.retain(|l, _| range.contains(l));
        }
        out
    }
}

/// Spans indexed by line, cheap to splice when lines are inserted or removed.
#[derive(Default)]
struct LineCache {
    lines: Vec<Option<LineSpans>>,
    /// Number of `Some` entries.
    count: usize,
}

impl LineCache {
    fn get(&self, line: usize) -> Option<&LineSpans> {
        self.lines.get(line).and_then(Option::as_ref)
    }

    fn insert(&mut self, line: usize, spans: LineSpans) {
        if line >= self.lines.len() {
            self.lines.resize(line + 1, None);
        }
        if self.lines[line].replace(spans).is_none() {
            self.count += 1;
        }
    }

    fn remove(&mut self, line: usize) {
        if let Some(slot) = self.lines.get_mut(line)
            && slot.take().is_some()
        {
            self.count -= 1;
        }
    }

    fn clear(&mut self) {
        self.lines.clear();
        self.count = 0;
    }

    fn truncate(&mut self, len: usize) {
        if len < self.lines.len() {
            self.count -= self.lines[len..].iter().filter(|l| l.is_some()).count();
            self.lines.truncate(len);
        }
    }

    /// Lines `start..=old_end` became `start..=new_end`: drop the old ones,
    /// leave the new ones empty and shift the rest.
    fn replace_lines(&mut self, start: usize, old_end: usize, new_end: usize) {
        if start >= self.lines.len() {
            return;
        }
        let end = (old_end + 1).min(self.lines.len());
        let removed = self
            .lines
            .splice(start..end, std::iter::repeat_n(None, new_end - start + 1))
            .filter(Option::is_some)
            .count();
        self.count -= removed;
    }
}

/// Highlights `s.line` and advances the states past it.
fn highlight_line(
    s: &mut State,
    doc: &Document,
    syntax_set: &SyntaxSet,
    hl: &Highlighter<'_>,
) -> Result<LineSpans, String> {
    if doc.line_end(s.line).0 - doc.line_start(s.line).0 > MAX_HIGHLIGHT_LINE {
        return Ok(Vec::new().into());
    }
    let mut text = doc.line_text(s.line);
    let content_len = text.len();
    text.push('\n');
    let ops = s
        .parse
        .parse_line(&text, syntax_set)
        .map_err(|e| format!("line {}: {e}", s.line + 1))?;
    let mut spans: Vec<Span> = Vec::new();
    let mut pos = 0;
    for (style, piece) in HighlightIterator::new(&mut s.highlight, &ops, &text, hl) {
        let start = pos;
        pos += piece.len();
        let end = pos.min(content_len);
        if end <= start {
            continue;
        }
        let c = style.foreground;
        let span = Span {
            start,
            end,
            color: [c.r, c.g, c.b, c.a],
            bold: style.font_style.contains(FontStyle::BOLD),
            italic: style.font_style.contains(FontStyle::ITALIC),
            underline: style.font_style.contains(FontStyle::UNDERLINE),
        };
        match spans.last_mut() {
            Some(last)
                if last.end == start
                    && (last.color, last.bold, last.italic, last.underline)
                        == (span.color, span.bold, span.italic, span.underline) =>
            {
                last.end = end;
            }
            _ => spans.push(span),
        }
    }
    Ok(spans.into())
}
