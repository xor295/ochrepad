//! The filter bar (PLAN_v0.3.0 P6): Search > Filter Lines shows only the
//! lines of a document that match a pattern, or only those that do not.
//!
//! Each tab has its own filter, and the bar shows the active tab's. The
//! lines are found on a background thread over a copy of the text
//! (`ochrepad_core::filter::shown_lines`); the view keeps its old filter
//! until the new one is ready. Edits shift the filter at once (lines an
//! edit touches show), and filter again after a pause.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};

use egui::text::{CCursor, CCursorRange};
use egui::{Id, Key, RichText, TextEdit, Ui};
use ochrepad_core::{Editor, SearchOptions, SearchQuery, Searcher, ViewId};

/// Quiet time after typing in the field, or after an edit to the document,
/// before filtering again.
const PAUSE_SECS: f64 = 0.3;

/// What the bar asks the app to do after drawing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilterRequest {
    /// Remove the filter, close the bar and give the editor the keyboard.
    Close,
}

/// A search running on another thread.
struct Job {
    query: SearchQuery,
    keep: bool,
    revision: u64,
    cancel: Arc<AtomicBool>,
    rx: Receiver<Option<Vec<usize>>>,
}

/// One tab's filter.
struct TabFilter {
    pattern: String,
    options: SearchOptions,
    /// Show the matching lines (else the others).
    keep: bool,
    /// When the query last changed, for the pause before filtering.
    changed_at: f64,
    /// The document revision last seen, and since when.
    seen: (u64, f64),
    /// What the view's filter was made from.
    applied: Option<(SearchQuery, bool, u64)>,
    job: Option<Job>,
    error: Option<String>,
    focus_field: bool,
}

impl TabFilter {
    fn query(&self) -> SearchQuery {
        SearchQuery::new(self.pattern.clone(), self.options.clone())
    }
}

#[derive(Default)]
pub struct FilterBar {
    tabs: HashMap<ViewId, TabFilter>,
}

fn field_id(view: ViewId) -> Id {
    Id::new(("ochrepad-filter-field", view.0))
}

impl FilterBar {
    /// Whether `view` has a filter (and so shows the bar).
    pub fn is_open(&self, view: ViewId) -> bool {
        self.tabs.contains_key(&view)
    }

    /// The pattern of `view`'s filter.
    pub fn pattern(&self, view: ViewId) -> Option<&str> {
        self.tabs.get(&view).map(|t| t.pattern.as_str())
    }

    /// Whether a search is still running for `view`.
    pub fn is_busy(&self, view: ViewId) -> bool {
        self.tabs.get(&view).is_some_and(|t| t.job.is_some())
    }

    /// Opens the bar for `view` (Alt+Shift+L), or puts the keyboard in its
    /// field if it is open. A one-line selection becomes the pattern.
    pub fn open(&mut self, editor: &Editor, view: ViewId, now: f64) {
        let selected = editor.view(view).and_then(|v| {
            let sel = v.selections.primary();
            let text = editor.buffer(v.buffer())?.doc().slice(sel.range());
            (!sel.is_empty() && !text.contains('\n')).then_some(text)
        });
        let revision = editor
            .view(view)
            .and_then(|v| editor.buffer(v.buffer()))
            .map_or(0, |b| b.revision());
        let tab = self.tabs.entry(view).or_insert_with(|| TabFilter {
            pattern: String::new(),
            options: SearchOptions::default(),
            keep: true,
            changed_at: now,
            seen: (revision, now),
            applied: None,
            job: None,
            error: None,
            focus_field: true,
        });
        if let Some(text) = selected {
            tab.pattern = text;
            tab.changed_at = now - PAUSE_SECS;
        }
        tab.focus_field = true;
    }

    /// Removes `view`'s filter and closes its bar.
    pub fn close(&mut self, editor: &mut Editor, view: ViewId) {
        if let Some(tab) = self.tabs.remove(&view)
            && let Some(job) = tab.job
        {
            job.cancel.store(true, Ordering::Relaxed);
        }
        editor.set_filter(view, None);
    }

    /// Per frame: starts searches when the query or the text changed and
    /// things went quiet, and applies finished ones. Returns whether a
    /// search is running (the app keeps repainting).
    pub fn tick(&mut self, editor: &mut Editor, now: f64) -> bool {
        self.tabs.retain(|view, _| editor.view(*view).is_some());
        let mut busy = false;
        for (&view, tab) in &mut self.tabs {
            let Some(buffer) = editor.view(view).map(|v| v.buffer()) else {
                continue;
            };
            let Some(revision) = editor.buffer(buffer).map(|b| b.revision()) else {
                continue;
            };
            if tab.seen.0 != revision {
                tab.seen = (revision, now);
            }
            // A finished search.
            if let Some(job) = &tab.job
                && let Ok(result) = job.rx.try_recv()
            {
                let job = tab.job.take().expect("checked above");
                if let Some(shown) = result
                    && job.revision == revision
                {
                    editor.set_filter(view, Some(&shown));
                    tab.applied = Some((job.query, job.keep, revision));
                }
            }
            let query = tab.query();
            if tab.pattern.is_empty() {
                tab.error = None;
                if tab.applied.take().is_some() {
                    editor.set_filter(view, None);
                }
                continue;
            }
            let wanted = (query.clone(), tab.keep, revision);
            let quiet = now - tab.changed_at >= PAUSE_SECS && now - tab.seen.1 >= PAUSE_SECS;
            let stale_job = tab
                .job
                .as_ref()
                .is_some_and(|j| (&j.query, j.keep, j.revision) != (&wanted.0, wanted.1, wanted.2));
            if stale_job
                && quiet
                && let Some(job) = tab.job.take()
            {
                job.cancel.store(true, Ordering::Relaxed);
            }
            if tab.applied.as_ref() != Some(&wanted) && tab.job.is_none() && quiet {
                if let Err(e) = Searcher::new(&query) {
                    tab.error = Some(e.0);
                    continue;
                }
                tab.error = None;
                let Some(doc) = editor.buffer(buffer).map(|b| b.doc().clone()) else {
                    continue;
                };
                let (tx, rx) = channel();
                let cancel = Arc::new(AtomicBool::new(false));
                let stop = cancel.clone();
                let (q, keep) = (query.clone(), tab.keep);
                std::thread::spawn(move || {
                    let result = Searcher::new(&q).ok().and_then(|s| {
                        ochrepad_core::filter::shown_lines(&doc, &s, keep, &|| {
                            stop.load(Ordering::Relaxed)
                        })
                    });
                    let _ = tx.send(result);
                });
                tab.job = Some(Job {
                    query,
                    keep: tab.keep,
                    revision,
                    cancel,
                    rx,
                });
            }
            busy |= tab.job.is_some() || tab.applied.as_ref() != Some(&wanted);
        }
        busy
    }

    /// The bar for `view`.
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        editor: &Editor,
        view: ViewId,
        now: f64,
    ) -> Vec<FilterRequest> {
        let mut requests = Vec::new();
        let Some(tab) = self.tabs.get_mut(&view) else {
            return requests;
        };
        let field_width = (ui.available_width() * 0.35).clamp(120.0, 320.0);
        ui.horizontal_wrapped(|ui| {
            ui.label("Filter:");
            let mut output = TextEdit::singleline(&mut tab.pattern)
                .id(field_id(view))
                .desired_width(field_width)
                .hint_text("lines with this text")
                .show(ui);
            if std::mem::take(&mut tab.focus_field) {
                output.response.request_focus();
                let end = CCursor::new(tab.pattern.chars().count());
                output
                    .state
                    .cursor
                    .set_char_range(Some(CCursorRange::two(CCursor::new(0), end)));
                output.state.store(ui.ctx(), field_id(view));
            }
            let field = output.response;
            if field.changed() {
                tab.changed_at = now;
            }
            if field.lost_focus() {
                let (enter, esc) =
                    ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
                if enter {
                    // Filter now, without waiting for the pause.
                    tab.changed_at = now - PAUSE_SECS;
                    field.request_focus();
                } else if esc {
                    requests.push(FilterRequest::Close);
                }
            }
            let mut changed = false;
            changed |= ui
                .radio_value(&mut tab.keep, true, "Show matching")
                .on_hover_text("Show only the lines with a match")
                .changed();
            changed |= ui
                .radio_value(&mut tab.keep, false, "Hide matching")
                .on_hover_text("Hide the lines with a match")
                .changed();
            ui.separator();
            changed |= ui
                .checkbox(&mut tab.options.match_case, "Match case")
                .changed();
            changed |= ui
                .checkbox(&mut tab.options.whole_word, "Whole word")
                .changed();
            changed |= ui
                .checkbox(&mut tab.options.regex, "Regular expression")
                .changed();
            if changed {
                tab.changed_at = now - PAUSE_SECS;
            }
            ui.separator();
            let lines = editor
                .view(view)
                .and_then(|v| editor.buffer(v.buffer()).map(|b| (v, b)))
                .map(|(v, b)| {
                    let total = b.doc().line_count();
                    (total - v.folds().filter_hidden_count(), total)
                });
            if let Some(e) = &tab.error {
                ui.label(RichText::new(e).color(ui.visuals().error_fg_color));
            } else if tab.job.is_some() {
                ui.spinner();
                ui.label("Filtering…");
            } else if let Some((shown, total)) = lines.filter(|_| tab.applied.is_some()) {
                ui.label(format!("{} of {} lines", group(shown), group(total)));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("×")
                    .on_hover_text("Remove the filter (Escape)")
                    .clicked()
                {
                    requests.push(FilterRequest::Close);
                }
            });
        });
        requests
    }
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

#[cfg(test)]
mod tests {
    #[test]
    fn numbers_are_grouped_by_thousands() {
        assert_eq!(super::group(0), "0");
        assert_eq!(super::group(999), "999");
        assert_eq!(super::group(1000), "1,000");
        assert_eq!(super::group(1234567), "1,234,567");
    }
}
