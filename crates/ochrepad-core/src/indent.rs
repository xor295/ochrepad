//! Indentation: detecting how a file is indented, and converting its
//! leading whitespace between tabs and spaces.

use std::collections::HashMap;

use crate::buffer::Indent;
use crate::changeset::{ChangeSet, Edit};
use crate::history::EditKind;
use crate::position::ByteOffset;
use crate::resolve::Resolved;
use crate::selection::SelectionSet;
use crate::text::Document;

/// Lines looked at when detecting.
const DETECT_LINES: usize = 10_000;

/// What the leading whitespace of a document says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Detected {
    /// Spaces or tabs, if any line is indented.
    pub use_spaces: Option<bool>,
    /// The indent width, if the space-indented lines show one (2, 4 or 8).
    pub width: Option<usize>,
    /// Some lines are indented with tabs and others with (two or more)
    /// spaces.
    pub mixed: bool,
}

impl Detected {
    /// `base` with what was detected applied.
    pub fn apply(&self, base: Indent) -> Indent {
        Indent {
            use_spaces: self.use_spaces.unwrap_or(base.use_spaces),
            width: self.width.unwrap_or(base.width),
        }
    }
}

/// Looks at the first lines of `doc`: tabs or spaces, and for spaces the
/// most common step between indent levels.
pub fn detect(doc: &Document) -> Detected {
    let (mut tabs, mut spaces) = (0usize, 0usize);
    let mut steps: HashMap<usize, usize> = HashMap::new();
    let mut prev = 0usize;
    for line in 0..doc.line_count().min(DETECT_LINES) {
        let text = doc.line_text(line);
        if text.trim().is_empty() {
            continue;
        }
        let lead: &str = &text[..text.len() - text.trim_start_matches([' ', '\t']).len()];
        if lead.starts_with('\t') {
            tabs += 1;
        } else if lead.len() >= 2 && lead.bytes().all(|b| b == b' ') {
            // One leading space is usually a comment's ` *`, not indentation.
            spaces += 1;
        }
        if lead.bytes().all(|b| b == b' ') {
            let n = lead.len();
            if n > prev {
                *steps.entry(n - prev).or_default() += 1;
            }
            prev = n;
        }
    }
    let use_spaces = (tabs + spaces > 0).then_some(spaces > tabs);
    let width = (use_spaces == Some(true))
        .then(|| {
            [4, 2, 8]
                .into_iter()
                .max_by_key(|w| steps.get(w).copied().unwrap_or(0))
                .filter(|w| steps.get(w).copied().unwrap_or(0) > 0)
        })
        .flatten();
    Detected {
        use_spaces,
        width,
        mixed: tabs > 0 && spaces > 0,
    }
}

/// Rewrites every line's leading whitespace as spaces, or as tabs (plus
/// spaces for a remainder narrower than a tab), keeping its width.
pub(crate) fn convert(
    doc: &Document,
    sels: &SelectionSet,
    to_spaces: bool,
    width: usize,
) -> Resolved {
    let width = width.max(1);
    let mut edits = Vec::new();
    for line in 0..doc.line_count() {
        let text = doc.line_text(line);
        let lead_len = text.len() - text.trim_start_matches([' ', '\t']).len();
        if lead_len == 0 {
            continue;
        }
        let mut cols = 0;
        for b in text[..lead_len].bytes() {
            cols = if b == b'\t' {
                cols + width - cols % width
            } else {
                cols + 1
            };
        }
        let new = if to_spaces {
            " ".repeat(cols)
        } else {
            format!("{}{}", "\t".repeat(cols / width), " ".repeat(cols % width))
        };
        if new != text[..lead_len] {
            let start = doc.line_start(line);
            edits.push(Edit::new(start, ByteOffset(start.0 + lead_len), new));
        }
    }
    let changes = ChangeSet::new(doc.len(), edits);
    if changes.is_empty() {
        return Resolved::Selections(sels.clone());
    }
    let after = sels.map(|s| crate::selection::Selection {
        anchor: changes.map_position(s.anchor, crate::changeset::Bias::After),
        head: changes.map_position(s.head, crate::changeset::Bias::After),
        goal: None,
    });
    Resolved::Edit {
        changes,
        after,
        kind: EditKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect_text(t: &str) -> Detected {
        detect(&Document::new(t))
    }

    #[test]
    fn detects_spaces_and_their_width() {
        let py = "def f():\n    if x:\n        y()\n    return 1\n";
        assert_eq!(
            detect_text(py),
            Detected {
                use_spaces: Some(true),
                width: Some(4),
                mixed: false
            }
        );
        let js = "a {\n  b {\n    c;\n  }\n}\n";
        assert_eq!(detect_text(js).width, Some(2));
    }

    #[test]
    fn detects_tabs_and_mixed_files() {
        let go = "func f() {\n\tif x {\n\t\ty()\n\t}\n\t/*\n\t * doc\n\t */\n}\n";
        assert_eq!(
            detect_text(go),
            Detected {
                use_spaces: Some(false),
                width: None,
                mixed: false
            }
        );
        let mixed = "def f():\n    a()\n\tb()\n";
        assert!(detect_text(mixed).mixed);
        assert_eq!(detect_text("x\ny\n"), Detected::default());
    }
}
