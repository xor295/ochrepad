//! The filter bar (PLAN_v0.3.0 P6, P8): Search > Filter Lines shows only
//! the lines of a document that match a pattern, or only those that do not.
//!
//! Each tab has its own filter, and the bar shows the focused tab's. In an
//! editor the lines are found on a background thread over a copy of the
//! text (`ochrepad_core::filter::shown_lines`); the view keeps its old
//! filter until the new one is ready. Edits shift the filter at once (lines
//! an edit touches show), and filter again after a pause. In a viewer the
//! viewer searches the file itself and fills its view in as lines are found
//! (`ViewerState::set_filter`).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};

use egui::text::{CCursor, CCursorRange};
use egui::{Id, Key, RichText, TextEdit, Ui};
use ochrepad_core::{Editor, SearchOptions, SearchQuery, Searcher, ViewId};

use crate::viewer::{ViewerId, Viewers};

/// Quiet time after typing in the field, or after an edit to the document,
/// before filtering again.
const PAUSE_SECS: f64 = 0.3;

/// The tab a filter belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FilterTarget {
    Editor(ViewId),
    Viewer(ViewerId),
}

impl From<ViewId> for FilterTarget {
    fn from(v: ViewId) -> Self {
        FilterTarget::Editor(v)
    }
}

impl From<ViewerId> for FilterTarget {
    fn from(v: ViewerId) -> Self {
        FilterTarget::Viewer(v)
    }
}

/// What the bar asks the app to do after drawing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilterRequest {
    /// Remove the filter, close the bar and give the tab the keyboard.
    Close,
}

/// An editor's search running on another thread.
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
    /// The document revision last seen, and since when (editors).
    seen: (u64, f64),
    /// What the filter was made from (the revision is 0 for a viewer).
    applied: Option<(SearchQuery, bool, u64)>,
    job: Option<Job>,
    error: Option<String>,
    focus_field: bool,
}

impl TabFilter {
    fn new(now: f64, revision: u64) -> Self {
        Self {
            pattern: String::new(),
            options: SearchOptions::default(),
            keep: true,
            changed_at: now,
            seen: (revision, now),
            applied: None,
            job: None,
            error: None,
            focus_field: true,
        }
    }

    fn query(&self) -> SearchQuery {
        SearchQuery::new(self.pattern.clone(), self.options.clone())
    }
}

#[derive(Default)]
pub struct FilterBar {
    tabs: HashMap<FilterTarget, TabFilter>,
}

fn field_id(target: FilterTarget) -> Id {
    Id::new(("ochrepad-filter-field", target))
}

impl FilterBar {
    /// Whether the tab has a filter (and so shows the bar).
    pub fn is_open(&self, target: impl Into<FilterTarget>) -> bool {
        self.tabs.contains_key(&target.into())
    }

    /// The pattern of the tab's filter.
    pub fn pattern(&self, target: impl Into<FilterTarget>) -> Option<&str> {
        self.tabs.get(&target.into()).map(|t| t.pattern.as_str())
    }

    /// Whether a search is still running for an editor tab.
    pub fn is_busy(&self, target: impl Into<FilterTarget>) -> bool {
        self.tabs
            .get(&target.into())
            .is_some_and(|t| t.job.is_some())
    }

    /// Opens the bar for a tab (Alt+Shift+L), or puts the keyboard in its
    /// field if it is open. A one-line selection becomes the pattern.
    pub fn open(&mut self, editor: &Editor, viewers: &Viewers, target: FilterTarget, now: f64) {
        let (selected, revision) = match target {
            FilterTarget::Editor(view) => (
                editor.view(view).and_then(|v| {
                    let sel = v.selections.primary();
                    let text = editor.buffer(v.buffer())?.doc().slice(sel.range());
                    (!sel.is_empty() && !text.contains('\n')).then_some(text)
                }),
                editor
                    .view(view)
                    .and_then(|v| editor.buffer(v.buffer()))
                    .map_or(0, |b| b.revision()),
            ),
            FilterTarget::Viewer(id) => (
                viewers
                    .get(id)
                    .and_then(|v| v.state.selected_line_text(&v.file)),
                0,
            ),
        };
        let tab = self
            .tabs
            .entry(target)
            .or_insert_with(|| TabFilter::new(now, revision));
        if let Some(text) = selected {
            tab.pattern = text;
            tab.changed_at = now - PAUSE_SECS;
        }
        tab.focus_field = true;
    }

    /// Removes the tab's filter and closes its bar.
    pub fn close(&mut self, editor: &mut Editor, viewers: &mut Viewers, target: FilterTarget) {
        if let Some(tab) = self.tabs.remove(&target)
            && let Some(job) = tab.job
        {
            job.cancel.store(true, Ordering::Relaxed);
        }
        match target {
            FilterTarget::Editor(view) => editor.set_filter(view, None),
            FilterTarget::Viewer(id) => {
                if let Some(v) = viewers.get_mut(id) {
                    let _ = v.state.set_filter(&v.file, None);
                }
            }
        }
    }

    /// Per frame: starts searches when the query or the text changed and
    /// things went quiet, and applies finished ones. Returns whether a
    /// search is running (the app keeps repainting).
    pub fn tick(&mut self, editor: &mut Editor, viewers: &mut Viewers, now: f64) -> bool {
        self.tabs.retain(|target, _| match *target {
            FilterTarget::Editor(view) => editor.view(view).is_some(),
            FilterTarget::Viewer(id) => viewers.get(id).is_some(),
        });
        let mut busy = false;
        for (&target, tab) in &mut self.tabs {
            busy |= match target {
                FilterTarget::Editor(view) => tick_editor(tab, editor, view, now),
                FilterTarget::Viewer(id) => tick_viewer(tab, viewers, id, now),
            };
        }
        busy
    }

    /// The bar for a tab.
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        editor: &Editor,
        viewers: &Viewers,
        target: FilterTarget,
        now: f64,
    ) -> Vec<FilterRequest> {
        let mut requests = Vec::new();
        let Some(tab) = self.tabs.get_mut(&target) else {
            return requests;
        };
        let field_width = (ui.available_width() * 0.35).clamp(120.0, 320.0);
        ui.horizontal_wrapped(|ui| {
            ui.label("Filter:");
            let mut output = TextEdit::singleline(&mut tab.pattern)
                .id(field_id(target))
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
                output.state.store(ui.ctx(), field_id(target));
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
            if let Some(e) = &tab.error {
                ui.label(RichText::new(e).color(ui.visuals().error_fg_color));
            } else {
                match target {
                    FilterTarget::Editor(view) => editor_status(ui, tab, editor, view),
                    FilterTarget::Viewer(id) => viewer_status(ui, tab, viewers, id),
                }
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

/// An editor tab's filter, per frame (see [`FilterBar::tick`]).
fn tick_editor(tab: &mut TabFilter, editor: &mut Editor, view: ViewId, now: f64) -> bool {
    let Some(buffer) = editor.view(view).map(|v| v.buffer()) else {
        return false;
    };
    let Some(revision) = editor.buffer(buffer).map(|b| b.revision()) else {
        return false;
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
        return false;
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
            return false;
        }
        tab.error = None;
        let Some(doc) = editor.buffer(buffer).map(|b| b.doc().clone()) else {
            return false;
        };
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let (q, keep) = (query.clone(), tab.keep);
        std::thread::spawn(move || {
            let result = Searcher::new(&q).ok().and_then(|s| {
                ochrepad_core::filter::shown_lines(&doc, &s, keep, &|| stop.load(Ordering::Relaxed))
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
    tab.job.is_some() || tab.applied.as_ref() != Some(&wanted)
}

/// A viewer tab's filter, per frame: the viewer searches by itself once
/// the query is set.
fn tick_viewer(tab: &mut TabFilter, viewers: &mut Viewers, id: ViewerId, now: f64) -> bool {
    let Some(v) = viewers.get_mut(id) else {
        return false;
    };
    let query = tab.query();
    if tab.pattern.is_empty() {
        tab.error = None;
        if tab.applied.take().is_some() {
            let _ = v.state.set_filter(&v.file, None);
        }
        return false;
    }
    let wanted = (query.clone(), tab.keep, 0);
    if tab.applied.as_ref() != Some(&wanted) && now - tab.changed_at >= PAUSE_SECS {
        match v.state.set_filter(&v.file, Some((query, tab.keep))) {
            Ok(()) => {
                tab.error = None;
                tab.applied = Some(wanted);
            }
            Err(e) => tab.error = Some(e),
        }
    }
    v.state
        .filter_progress(&v.file)
        .is_some_and(|(_, part)| part < 1.0)
}

fn editor_status(ui: &mut Ui, tab: &TabFilter, editor: &Editor, view: ViewId) {
    let lines = editor
        .view(view)
        .and_then(|v| editor.buffer(v.buffer()).map(|b| (v, b)))
        .map(|(v, b)| {
            let total = b.doc().line_count();
            (total - v.folds().filter_hidden_count(), total)
        });
    if tab.job.is_some() {
        ui.spinner();
        ui.label("Filtering…");
    } else if let Some((shown, total)) = lines.filter(|_| tab.applied.is_some()) {
        ui.label(format!("{} of {} lines", group(shown), group(total)));
    }
}

fn viewer_status(ui: &mut Ui, tab: &TabFilter, viewers: &Viewers, id: ViewerId) {
    if tab.applied.is_none() {
        return;
    }
    let Some(v) = viewers.get(id) else {
        return;
    };
    let Some((rows, part)) = v.state.filter_progress(&v.file) else {
        return;
    };
    let total = v.file.progress().lines;
    if part < 1.0 {
        ui.spinner();
        ui.label(format!(
            "{} lines so far, {:.0}% searched",
            group(rows as usize),
            part * 100.0
        ));
    } else {
        ui.label(format!(
            "{} of {} lines",
            group(rows as usize),
            group(total as usize)
        ));
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
