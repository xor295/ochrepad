//! Glue between a buffer and its incremental highlighter: language detection,
//! the size limit, theme following light/dark mode, and time-sliced
//! background work.

use std::ops::Range;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ochrepad_core::{Buffer, Document};
use ochrepad_syntax::{BufferHighlighter, DEFAULT_SIZE_LIMIT, LineSpans, PLAIN_TEXT, Registry};

use crate::editor_view::LineStyler;

/// Background highlighting time per frame.
pub const FRAME_BUDGET: Duration = Duration::from_millis(4);

pub struct Highlighting {
    registry: Arc<Registry>,
    languages: Vec<String>,
    hl: Option<BufferHighlighter>,
    language: String,
    /// User's syntax theme; `None` follows light/dark mode.
    pub theme_override: Option<String>,
    pub size_limit: usize,
    too_large: bool,
}

impl Highlighting {
    pub fn new(registry: Arc<Registry>) -> Self {
        Self {
            languages: registry.language_names(),
            registry,
            hl: None,
            language: PLAIN_TEXT.to_owned(),
            theme_override: None,
            size_limit: DEFAULT_SIZE_LIMIT,
            too_large: false,
        }
    }

    pub fn languages(&self) -> &[String] {
        &self.languages
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    /// Whether the background pass still has work to do.
    pub fn is_busy(&self) -> bool {
        self.hl.as_ref().is_some_and(BufferHighlighter::is_busy)
    }

    /// The syntax theme in use, if highlighting is running.
    pub fn active_theme(&self) -> Option<&str> {
        self.hl.as_ref().map(BufferHighlighter::theme)
    }

    pub fn effective_theme(&self, dark: bool) -> String {
        self.theme_override
            .clone()
            .unwrap_or_else(|| Registry::default_theme(dark).to_owned())
    }

    /// Starts highlighting a (new or newly opened) buffer, detecting its
    /// language from the file name and first line.
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
        self.hl = None;
        self.set_language(buffer, &language, dark);
    }

    pub fn set_language(&mut self, buffer: &mut Buffer, language: &str, dark: bool) {
        buffer.language = Some(language.to_owned());
        self.language = language.to_owned();
        let theme = self.effective_theme(dark);
        match &mut self.hl {
            Some(hl) => hl.set_syntax(&self.registry, language),
            None => {
                self.hl = Some(BufferHighlighter::new(&self.registry, language, &theme));
            }
        }
        self.too_large = false;
    }

    /// Per frame: applies the buffer's line edits, follows the theme, enforces
    /// the size limit and runs a slice of background work. Returns whether
    /// more work remains (the caller should request a repaint).
    pub fn frame(&mut self, buffer: &mut Buffer, visible: Range<usize>, dark: bool) -> bool {
        let edits = buffer.take_line_edits();
        let too_large = buffer.doc().len().0 > self.size_limit;
        if too_large != self.too_large {
            self.too_large = too_large;
            self.hl = (!too_large).then(|| {
                BufferHighlighter::new(&self.registry, &self.language, &self.effective_theme(dark))
            });
            return false;
        }
        let theme = self.effective_theme(dark);
        let Some(hl) = &mut self.hl else {
            return false;
        };
        hl.apply_edits(&edits);
        hl.set_theme(&self.registry, &theme);
        hl.work(&self.registry, buffer.doc(), visible, FRAME_BUDGET)
    }

    /// Text for the status bar when highlighting is not running normally.
    pub fn problem(&self) -> Option<String> {
        if self.too_large {
            let limit = match self.size_limit {
                n if n >= 1024 * 1024 => format!("{} MB", n / (1024 * 1024)),
                n if n >= 1024 => format!("{} KB", n / 1024),
                n => format!("{n} bytes"),
            };
            return Some(format!("Highlighting off: file larger than {limit}"));
        }
        self.hl
            .as_ref()
            .and_then(BufferHighlighter::error)
            .map(|e| format!("Highlighting stopped: {e}"))
    }

    pub fn styler(&mut self) -> Option<Styler<'_>> {
        let hl = self.hl.as_mut()?;
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
