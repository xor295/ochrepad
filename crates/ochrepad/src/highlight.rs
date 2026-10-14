//! Glue between buffers and their incremental highlighters: language
//! detection, the size limit, theme following light/dark mode, and
//! time-sliced background work. One highlighter per buffer, shared by all of
//! the buffer's views.

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ochrepad_core::{Buffer, BufferId, Document};
use ochrepad_syntax::{BufferHighlighter, DEFAULT_SIZE_LIMIT, LineSpans, PLAIN_TEXT, Registry};

use crate::editor_view::LineStyler;

/// Background highlighting time per frame, shared by the visible buffers.
pub const FRAME_BUDGET: Duration = Duration::from_millis(4);

struct BufferState {
    hl: Option<BufferHighlighter>,
    language: String,
    too_large: bool,
}

pub struct Highlighting {
    registry: Arc<Registry>,
    languages: Vec<String>,
    buffers: HashMap<BufferId, BufferState>,
    /// User's syntax theme; `None` follows light/dark mode.
    pub theme_override: Option<String>,
    pub size_limit: usize,
}

impl Highlighting {
    /// The syntaxes and themes, shared with background work.
    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    pub fn new(registry: Arc<Registry>) -> Self {
        Self {
            languages: registry.language_names(),
            registry,
            buffers: HashMap::new(),
            theme_override: None,
            size_limit: DEFAULT_SIZE_LIMIT,
        }
    }

    pub fn languages(&self) -> &[String] {
        &self.languages
    }

    pub fn language(&self, buffer: BufferId) -> &str {
        self.buffers
            .get(&buffer)
            .map_or(PLAIN_TEXT, |b| b.language.as_str())
    }

    pub fn effective_theme(&self, dark: bool) -> String {
        self.theme_override
            .clone()
            .unwrap_or_else(|| Registry::default_theme(dark).to_owned())
    }

    /// The syntax theme in use for `buffer`, if it is being highlighted.
    pub fn active_theme(&self, buffer: BufferId) -> Option<&str> {
        self.buffers
            .get(&buffer)
            .and_then(|b| b.hl.as_ref())
            .map(BufferHighlighter::theme)
    }

    /// Whether any background pass still has work to do.
    pub fn is_busy(&self) -> bool {
        self.buffers
            .values()
            .any(|b| b.hl.as_ref().is_some_and(BufferHighlighter::is_busy))
    }

    /// Starts highlighting a (new, newly opened or renamed) buffer, detecting
    /// its language from the file name and first line.
    pub fn attach(&mut self, buffer: &mut Buffer, dark: bool) {
        let name = buffer
            .meta
            .path
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|n| n.to_str())
            .map(str::to_owned);
        let first_line = buffer.doc().line_text(0);
        let language = self.registry.detect(name.as_deref(), &first_line);
        let _ = buffer.take_line_edits();
        self.buffers.remove(&buffer.id());
        self.set_language(buffer, &language, dark);
    }

    pub fn detach(&mut self, buffer: BufferId) {
        self.buffers.remove(&buffer);
    }

    pub fn set_language(&mut self, buffer: &mut Buffer, language: &str, dark: bool) {
        buffer.language = Some(language.to_owned());
        buffer.comments = ochrepad_syntax::comment_syntax(language);
        buffer.fold_strategy = if language == "Markdown" {
            ochrepad_core::FoldStrategy::Markdown
        } else {
            ochrepad_core::FoldStrategy::Indent
        };
        let theme = self.effective_theme(dark);
        let state = self
            .buffers
            .entry(buffer.id())
            .or_insert_with(|| BufferState {
                hl: None,
                language: String::new(),
                too_large: false,
            });
        state.language = language.to_owned();
        state.too_large = false;
        match &mut state.hl {
            Some(hl) => hl.set_syntax(&self.registry, language),
            None => state.hl = Some(BufferHighlighter::new(&self.registry, language, &theme)),
        }
    }

    /// Per frame, for every buffer: applies its line edits, follows the
    /// theme and enforces the size limit. Buffers with a visible view get a
    /// slice of background work, prioritising their visible lines. Returns
    /// whether more work remains (the caller should request a repaint).
    pub fn frame<'a>(
        &mut self,
        buffers: impl Iterator<Item = &'a mut Buffer>,
        visible: &HashMap<BufferId, Range<usize>>,
        dark: bool,
    ) -> bool {
        let theme = self.effective_theme(dark);
        let budget = FRAME_BUDGET / visible.len().max(1) as u32;
        let mut busy = false;
        for buffer in buffers {
            let edits = buffer.take_line_edits();
            let Some(state) = self.buffers.get_mut(&buffer.id()) else {
                continue;
            };
            let too_large = buffer.doc().len().0 > self.size_limit;
            if too_large != state.too_large {
                state.too_large = too_large;
                state.hl = (!too_large)
                    .then(|| BufferHighlighter::new(&self.registry, &state.language, &theme));
                continue;
            }
            let Some(hl) = &mut state.hl else {
                continue;
            };
            hl.apply_edits(&edits);
            hl.set_theme(&self.registry, &theme);
            if let Some(lines) = visible.get(&buffer.id()) {
                busy |= hl.work(&self.registry, buffer.doc(), lines.clone(), budget);
            }
        }
        busy
    }

    /// Text for the status bar when highlighting is not running normally.
    pub fn problem(&self, buffer: BufferId) -> Option<String> {
        let state = self.buffers.get(&buffer)?;
        if state.too_large {
            let limit = match self.size_limit {
                n if n >= 1024 * 1024 => format!("{} MB", n / (1024 * 1024)),
                n if n >= 1024 => format!("{} KB", n / 1024),
                n => format!("{n} bytes"),
            };
            return Some(format!("Highlighting off: file larger than {limit}"));
        }
        state
            .hl
            .as_ref()
            .and_then(BufferHighlighter::error)
            .map(|e| format!("Highlighting stopped: {e}"))
    }

    pub fn styler(&mut self, buffer: BufferId) -> Option<Styler<'_>> {
        let hl = self.buffers.get_mut(&buffer)?.hl.as_mut()?;
        Some(Styler {
            registry: &self.registry,
            hl,
        })
    }
}

pub struct Styler<'a> {
    registry: &'a Registry,
    hl: &'a mut BufferHighlighter,
}

impl LineStyler for Styler<'_> {
    fn line_spans(&mut self, doc: &Document, lines: Range<usize>) -> Vec<Option<LineSpans>> {
        self.hl.line_spans(self.registry, doc, lines)
    }
}
