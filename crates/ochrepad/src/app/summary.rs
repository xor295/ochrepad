//! View > Summary, as in Notepad++: the current document's path, when its
//! file was created and last written, and how many characters, words and
//! lines it has (and the selection). Counted once, when the dialog opens.

use std::time::SystemTime;

use egui::{Id, Modal, RichText, Ui};
use ochrepad_core::LineEnding;
use unicode_segmentation::UnicodeSegmentation;

use super::OchrepadApp;

/// Words are not counted in documents larger than this (it would hold up
/// the window for a moment).
const MAX_WORD_COUNT_BYTES: usize = 16 << 20;

pub(super) struct SummaryDialog {
    rows: Vec<(&'static str, String)>,
}

/// `1234567` as `1,234,567`.
fn group(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A time as `2026-10-14 07:46:05 UTC` (no time zone data is at hand, so
/// it is not shown as local time).
pub(crate) fn utc(t: SystemTime) -> Option<String> {
    let secs = t.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs();
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    Some(format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    ))
}

impl OchrepadApp {
    /// View > Summary: counts the current document and shows the dialog.
    pub fn show_summary(&mut self) {
        let name = self.file_name();
        let b = self.buffer();
        let doc = b.doc();
        let text_len = doc.len().0;
        let lines = doc.line_count();
        let breaks = lines - 1;
        let chars = doc.to_char_idx(doc.len()).0 - breaks;
        let mut rows = Vec::new();
        match &b.meta.path {
            Some(path) => {
                rows.push(("Full path", path.display().to_string()));
                if let Ok(meta) = std::fs::metadata(path) {
                    if let Some(t) = meta.created().ok().and_then(utc) {
                        rows.push(("Created", t));
                    }
                    if let Some(t) = meta.modified().ok().and_then(utc) {
                        rows.push(("Modified", t));
                    }
                    rows.push((
                        "Size on disk",
                        format!("{} bytes", group(meta.len() as usize)),
                    ));
                }
            }
            None => rows.push(("Name", format!("{name} (not saved yet)"))),
        }
        rows.push(("Characters (without line endings)", group(chars)));
        let words = if text_len <= MAX_WORD_COUNT_BYTES {
            // By line: no word goes over a line break.
            let count: usize = (0..lines)
                .map(|l| doc.line_text(l).unicode_words().count())
                .sum();
            group(count)
        } else {
            "too large to count".to_owned()
        };
        rows.push(("Words", words));
        rows.push(("Lines", group(lines)));
        // The text as UTF-8 with the document's line endings.
        let ending = match b.meta.line_ending {
            LineEnding::CrLf => 2,
            LineEnding::Lf | LineEnding::Cr => 1,
        };
        rows.push((
            "Document length",
            format!("{} bytes", group(text_len - breaks + breaks * ending)),
        ));
        if let Some(v) = self.editor.view(self.active) {
            let selected: Vec<_> = v
                .selections
                .ranges()
                .iter()
                .filter(|s| !s.is_empty())
                .collect();
            if !selected.is_empty() {
                let (mut c, mut bytes, mut l) = (0, 0, 0);
                for s in selected {
                    c += doc.to_char_idx(s.end()).0 - doc.to_char_idx(s.start()).0;
                    bytes += s.end().0 - s.start().0;
                    let (first, mut last) = (doc.line_of(s.start()), doc.line_of(s.end()));
                    // Whole lines selected end at the start of the next one.
                    if last > first && s.end() == doc.line_start(last) {
                        last -= 1;
                    }
                    l += last + 1 - first;
                }
                rows.push((
                    "Selection",
                    format!(
                        "{} characters, {} bytes (UTF-8), on {} lines",
                        group(c),
                        group(bytes),
                        group(l)
                    ),
                ));
            }
        }
        self.summary = Some(SummaryDialog { rows });
    }

    pub(super) fn summary_dialog(&mut self, ui: &mut Ui) {
        let Some(d) = &self.summary else {
            return;
        };
        let mut close = false;
        let resp = Modal::new(Id::new("ochrepad-summary")).show(ui.ctx(), |ui| {
            ui.label(RichText::new("Summary").strong());
            ui.add_space(8.0);
            egui::Grid::new("ochrepad-summary-grid")
                .num_columns(2)
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    for (name, value) in &d.rows {
                        ui.label(RichText::new(*name).weak());
                        ui.label(value);
                        ui.end_row();
                    }
                });
            ui.add_space(8.0);
            let button = ui.button("Close");
            // Keep the keyboard here, so typing does not reach the editor.
            if ui.memory(|m| m.focused().is_none()) {
                button.request_focus();
            }
            close = button.clicked();
        });
        if close || resp.should_close() {
            self.summary = None;
            self.view_state_mut().request_focus();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn times_are_written_out_in_utc() {
        let t = |secs| super::utc(UNIX_EPOCH + Duration::from_secs(secs)).unwrap();
        assert_eq!(t(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(t(951_782_400), "2000-02-29 00:00:00 UTC");
        assert_eq!(t(1_791_963_965), "2026-10-14 07:46:05 UTC");
    }

    #[test]
    fn numbers_are_grouped() {
        assert_eq!(super::group(1234567), "1,234,567");
        assert_eq!(super::group(12), "12");
    }
}
