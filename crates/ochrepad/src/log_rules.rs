//! Highlight rules for logs (PLAN_v0.3.0 P6): lines or matches coloured,
//! or lines dimmed, by pattern, in files whose name matches a glob (by
//! default `*.log`). The rules live in the settings; this compiles them and
//! draws them through the editor's [`RowStyle`] hook, on visible lines
//! only.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;

use egui::Color32;
use ochrepad_core::{ByteOffset, Document, RowPos, SearchOptions, SearchQuery, Searcher};
use serde::{Deserialize, Serialize};

use crate::editor_view::RowStyle;

/// How a rule marks what it matches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuleStyle {
    /// Tints the whole line.
    #[default]
    Line,
    /// Tints each match.
    Match,
    /// Dims the line's text.
    Dim,
}

impl RuleStyle {
    pub const ALL: [RuleStyle; 3] = [RuleStyle::Line, RuleStyle::Match, RuleStyle::Dim];

    pub fn name(self) -> &'static str {
        match self {
            RuleStyle::Line => "Whole line",
            RuleStyle::Match => "Match only",
            RuleStyle::Dim => "Dim the line",
        }
    }
}

/// One highlight rule.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HighlightRule {
    pub pattern: String,
    pub regex: bool,
    pub match_case: bool,
    /// sRGB.
    pub color: [u8; 3],
    pub style: RuleStyle,
}

impl Default for HighlightRule {
    fn default() -> Self {
        Self {
            pattern: String::new(),
            regex: false,
            match_case: true,
            color: [220, 160, 40],
            style: RuleStyle::Line,
        }
    }
}

/// The rules a new installation starts with: errors red, warnings amber,
/// debug and trace output dimmed.
pub fn default_rules() -> Vec<HighlightRule> {
    let rule = |pattern: &str, color, style| HighlightRule {
        pattern: pattern.to_owned(),
        regex: true,
        match_case: true,
        color,
        style,
    };
    vec![
        rule(
            r"\b(ERROR|FATAL|CRITICAL|SEVERE)\b",
            [230, 70, 70],
            RuleStyle::Line,
        ),
        rule(r"\b(WARN|WARNING)\b", [235, 165, 40], RuleStyle::Line),
        rule(
            r"\b(DEBUG|TRACE|VERBOSE)\b",
            [128, 128, 128],
            RuleStyle::Dim,
        ),
    ]
}

/// Whether `name` matches the glob `pattern` (`*` any run, `?` one
/// character), ignoring case.
pub fn glob_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    // Iterative matching with backtracking to the last `*`.
    let (mut i, mut j) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while j < n.len() {
        if i < p.len() && (p[i] == '?' || p[i] == n[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some((i, j));
            i += 1;
        } else if let Some((si, sj)) = star {
            i = si + 1;
            j = sj + 1;
            star = Some((si, sj + 1));
        } else {
            return false;
        }
    }
    p[i..].iter().all(|&c| c == '*')
}

/// The rules ready to use, and the files they apply to.
#[derive(Default)]
pub struct LogRules {
    rules: Vec<(Searcher, RuleStyle, [u8; 3])>,
    globs: Vec<String>,
    /// Rules whose pattern does not compile: (index, error).
    pub errors: Vec<(usize, String)>,
}

impl LogRules {
    /// Compiles `rules` for files matching any of the `;`- or
    /// `,`-separated globs in `files`.
    pub fn new(rules: &[HighlightRule], files: &str) -> Self {
        let mut out = LogRules {
            globs: files
                .split([';', ','])
                .map(str::trim)
                .filter(|g| !g.is_empty())
                .map(str::to_owned)
                .collect(),
            ..LogRules::default()
        };
        for (i, r) in rules.iter().enumerate() {
            if r.pattern.is_empty() {
                continue;
            }
            let query = SearchQuery::new(
                r.pattern.clone(),
                SearchOptions {
                    regex: r.regex,
                    match_case: r.match_case,
                    ..SearchOptions::default()
                },
            );
            match Searcher::new(&query) {
                Ok(s) => out.rules.push((s, r.style, r.color)),
                Err(e) => out.errors.push((i, e.0)),
            }
        }
        out
    }

    /// Whether the rules apply to a file named `name`.
    pub fn applies_to(&self, name: &str) -> bool {
        !self.rules.is_empty() && self.globs.iter().any(|g| glob_match(g, name))
    }
}

/// How one line looks.
#[derive(Clone, Default)]
struct LineLook {
    background: Option<Color32>,
    spans: Vec<(Range<usize>, Color32)>,
    dim: bool,
}

/// The rules applied to one document for one frame (lines are looked at as
/// they are drawn, and remembered for the frame).
pub struct LogStyle<'a> {
    rules: &'a LogRules,
    doc: &'a Document,
    dark: bool,
    lines: RefCell<HashMap<usize, LineLook>>,
}

impl<'a> LogStyle<'a> {
    pub fn new(rules: &'a LogRules, doc: &'a Document, dark: bool) -> Self {
        Self {
            rules,
            doc,
            dark,
            lines: RefCell::default(),
        }
    }

    fn look(&self, line: usize) -> LineLook {
        if let Some(look) = self.lines.borrow().get(&line) {
            return look.clone();
        }
        let mut look = LineLook::default();
        if line < self.doc.line_count() {
            let start = self.doc.line_start(line);
            let end = self.doc.line_end(line);
            let (line_alpha, match_alpha) = if self.dark { (45, 110) } else { (50, 120) };
            for (searcher, style, [r, g, b]) in &self.rules.rules {
                let matches = searcher.find_all(self.doc, start..end);
                if matches.is_empty() {
                    continue;
                }
                match style {
                    // The first whole-line rule that matches wins.
                    RuleStyle::Line if look.background.is_none() => {
                        look.background =
                            Some(Color32::from_rgba_unmultiplied(*r, *g, *b, line_alpha));
                    }
                    RuleStyle::Line => {}
                    RuleStyle::Match => {
                        let color = Color32::from_rgba_unmultiplied(*r, *g, *b, match_alpha);
                        look.spans.extend(
                            matches
                                .iter()
                                .filter(|m| m.end > m.start)
                                .map(|m| (offset(m.start, start)..offset(m.end, start), color)),
                        );
                    }
                    RuleStyle::Dim => look.dim = true,
                }
            }
        }
        self.lines.borrow_mut().insert(line, look.clone());
        look
    }
}

fn offset(pos: ByteOffset, line_start: ByteOffset) -> usize {
    pos.0 - line_start.0
}

impl RowStyle for LogStyle<'_> {
    fn background(&self, row: RowPos, blank: bool) -> Option<Color32> {
        if blank {
            return None;
        }
        self.look(row.line).background
    }

    fn spans(&self, line: usize) -> Vec<(Range<usize>, Color32)> {
        self.look(line).spans
    }

    fn text_opacity(&self, line: usize) -> f32 {
        if self.look(line).dim { 0.5 } else { 1.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_match_names() {
        assert!(glob_match("*.log", "app.log"));
        assert!(glob_match("*.log", "APP.LOG"));
        assert!(!glob_match("*.log", "app.log.1"));
        assert!(glob_match("*.log.*", "app.log.1"));
        assert!(glob_match("app-??.txt", "app-12.txt"));
        assert!(!glob_match("app-??.txt", "app-1.txt"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("a*b*c", "a-x-b-y-c"));
        assert!(!glob_match("a*b*c", "a-x-c"));
    }

    #[test]
    fn rules_colour_dim_and_apply_by_file_name() {
        let rules = LogRules::new(&default_rules(), "*.log; *.log.*");
        assert!(rules.errors.is_empty());
        assert!(rules.applies_to("server.log") && rules.applies_to("server.log.2"));
        assert!(!rules.applies_to("notes.txt"));
        let doc = Document::new("INFO ok\nERROR bad\nDEBUG noise\nWARN hmm");
        let style = LogStyle::new(&rules, &doc, true);
        let bg = |l| style.background(RowPos::new(l, 0), false);
        assert!(bg(0).is_none());
        assert!(bg(1).is_some() && bg(3).is_some() && bg(1) != bg(3));
        assert_eq!(style.text_opacity(2), 0.5);
        assert_eq!(style.text_opacity(1), 1.0);
        let bad = LogRules::new(
            &[HighlightRule {
                pattern: "(".into(),
                regex: true,
                ..HighlightRule::default()
            }],
            "*",
        );
        assert_eq!(bad.errors.len(), 1);
    }

    #[test]
    fn match_rules_tint_each_match() {
        let rules = LogRules::new(
            &[HighlightRule {
                pattern: "id=\\d+".into(),
                regex: true,
                style: RuleStyle::Match,
                ..HighlightRule::default()
            }],
            "*",
        );
        let doc = Document::new("a id=1 b id=22");
        let style = LogStyle::new(&rules, &doc, false);
        let spans: Vec<Range<usize>> = style.spans(0).into_iter().map(|s| s.0).collect();
        assert_eq!(spans, vec![2..6, 9..14]);
    }
}
