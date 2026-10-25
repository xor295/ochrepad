//! The side-by-side compare view (PLAN M12, made editable in 0.2.0): each
//! side is an editor view of its document, so it can be edited like any tab.
//! The core line diff aligns the sides: each side's view gets [`Padding`],
//! blank rows opposite lines only the other side has. The diff is
//! recomputed when edits pause; the sides scroll together; changed rows and
//! words are tinted; there is change navigation, summary counts and an
//! overview strip.
//!
//! A side showing a file as saved on disk is a read-only buffer this view
//! owns.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use egui::{Color32, Pos2, Rect, Sense, Stroke, Ui, UiBuilder, Vec2};
use ochrepad_core::diff::{LineDiff, RowKind, Side, diff_lines, diff_words};
use ochrepad_core::{BufferId, Document, Editor, FileMeta, RowPos, ViewId};

use crate::editor_view::{Decorations, EditorViewState, LineStyler, RowStyle, editor_view};
use crate::highlight::Highlighting;
use crate::keymap::Keymap;

/// Quiet time after an edit before the diff is recomputed.
pub const RECOMPUTE_DEBOUNCE_SECS: f64 = 0.3;
const STRIP_W: f32 = 14.0;
const DIVIDER_W: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CompareId(pub u64);

/// What one side of a comparison shows.
pub enum CompareSide {
    /// An open buffer (edits in either place show in both).
    Buffer(BufferId),
    /// The buffer's file as it is on disk (read-only).
    Saved { buffer: BufferId, doc: Document },
}

impl CompareSide {
    /// The open buffer this side is about.
    pub fn buffer(&self) -> BufferId {
        match self {
            CompareSide::Buffer(b) | CompareSide::Saved { buffer: b, .. } => *b,
        }
    }

    pub fn label(&self, name: &dyn Fn(BufferId) -> String) -> String {
        match self {
            CompareSide::Buffer(b) => name(*b),
            CompareSide::Saved { buffer, .. } => format!("{} (saved)", name(*buffer)),
        }
    }
}

/// Changed byte ranges on the left and right line of a changed row.
type WordRanges = (Vec<Range<usize>>, Vec<Range<usize>>);

const SIDES: [Side; 2] = [Side::Left, Side::Right];

pub struct Compare {
    pub left: CompareSide,
    pub right: CompareSide,
    /// The buffer each side shows (a saved copy is one this view made).
    shown: [BufferId; 2],
    /// The editor view of each side.
    views: [ViewId; 2],
    states: [EditorViewState; 2],
    /// A buffer this comparison made (a saved copy), removed with it.
    owned: Option<BufferId>,
    pub diff: LineDiff,
    /// The diff row of each line, per side.
    line_rows: [Vec<usize>; 2],
    computed: (u64, u64),
    /// Newest revisions seen and when, for the recompute debounce.
    seen: Option<((u64, u64), f64)>,
    /// The diff row (fractional) both sides were last scrolled to.
    synced: f64,
    /// Index into `diff.hunks` of the change navigated to.
    pub current: Option<usize>,
    words: HashMap<usize, WordRanges>,
    rows_visible: usize,
}

impl Compare {
    /// A comparison of `left` and `right`; makes an auxiliary view for each
    /// side (and a read-only buffer for a saved copy).
    pub fn new(editor: &mut Editor, left: CompareSide, right: CompareSide) -> Self {
        let mut owned = None;
        let mut show = |side: &CompareSide| match side {
            CompareSide::Buffer(b) => *b,
            CompareSide::Saved { doc, .. } => {
                let b = editor.add_buffer(&doc.to_string(), FileMeta::default());
                if let Some(buf) = editor.buffer_mut(b) {
                    buf.read_only = true;
                }
                owned = Some(b);
                b
            }
        };
        let shown = [show(&left), show(&right)];
        let views = shown.map(|b| editor.add_aux_view(b).expect("buffer exists"));
        let mut states = [EditorViewState::default(), EditorViewState::default()];
        for s in &mut states {
            s.fold_margin = false;
            s.word_wrap = false;
            s.document_map = false;
        }
        let mut c = Self {
            left,
            right,
            shown,
            views,
            states,
            owned,
            diff: LineDiff::default(),
            line_rows: [Vec::new(), Vec::new()],
            computed: (u64::MAX, u64::MAX),
            seen: None,
            synced: 0.0,
            current: None,
            words: HashMap::new(),
            rows_visible: 30,
        };
        c.recompute(editor);
        c
    }

    /// The buffer a saved copy is shown in, if this comparison made one.
    pub fn owned_buffer(&self) -> Option<BufferId> {
        self.owned
    }

    /// The editor view of the left (0) or right (1) side.
    pub fn view(&self, side: usize) -> ViewId {
        self.views[side]
    }

    pub fn involves(&self, buffer: BufferId) -> bool {
        self.left.buffer() == buffer || self.right.buffer() == buffer
    }

    /// Removes the side views and the saved copy from `editor`.
    pub fn close(&self, editor: &mut Editor) {
        for v in self.views {
            editor.remove_view(v);
        }
        if let Some(b) = self.owned {
            editor.remove_buffer(b);
        }
    }

    fn revisions(&self, editor: &Editor) -> (u64, u64) {
        let rev = |b: BufferId| editor.buffer(b).map_or(0, |b| b.revision());
        (rev(self.shown[0]), rev(self.shown[1]))
    }

    fn recompute(&mut self, editor: &mut Editor) {
        let (Some(a), Some(b)) = (
            editor.buffer(self.shown[0]).map(|b| b.doc()),
            editor.buffer(self.shown[1]).map(|b| b.doc()),
        ) else {
            return;
        };
        self.diff = diff_lines(a, b);
        self.words.clear();
        self.computed = self.revisions(editor);
        for (i, side) in SIDES.into_iter().enumerate() {
            self.line_rows[i] = self.diff.line_rows(side);
            if let Some(v) = editor.view_mut(self.views[i]) {
                v.padding = Some(Arc::new(self.diff.padding(side)));
            }
        }
        if self.current.is_some_and(|c| c >= self.diff.hunks.len()) {
            self.current = None;
        }
        // Keep both sides on the same rows under the new alignment.
        self.scroll_both(editor, self.synced);
    }

    /// Recomputes after edits have paused; returns how long until the
    /// next check, if one is pending.
    fn refresh(&mut self, editor: &mut Editor, now: f64) -> Option<f64> {
        let revs = self.revisions(editor);
        if revs == self.computed {
            return None;
        }
        if self.seen.is_none_or(|(r, _)| r != revs) {
            self.seen = Some((revs, now));
        }
        let since = self.seen.map_or(0.0, |(_, t)| now - t);
        if since >= RECOMPUTE_DEBOUNCE_SECS {
            self.recompute(editor);
            None
        } else {
            Some(RECOMPUTE_DEBOUNCE_SECS - since)
        }
    }

    pub fn summary(&self) -> String {
        let s = self.diff.stats;
        if self.diff.hunks.is_empty() {
            "No differences".to_owned()
        } else {
            format!(
                "{} added, {} removed, {} changed",
                s.added, s.removed, s.changed
            )
        }
    }

    /// The diff row (fractional) at the top of a side's view.
    fn top_row(&self, editor: &Editor, side: usize) -> f64 {
        let Some(v) = editor.view(self.views[side]) else {
            return 0.0;
        };
        let s = v.scroll;
        let before = v
            .padding
            .as_ref()
            .and_then(|p| p.before.get(s.first_line).copied())
            .unwrap_or(0);
        let rows = &self.line_rows[side];
        let at = match rows.get(s.first_line) {
            Some(&r) => (r + s.first_sub) as f64 - before as f64,
            None => self.diff.rows.len() as f64,
        };
        at + s.line_fraction as f64
    }

    /// Scrolls a side so that diff row `row` (fractional) is at the top.
    fn scroll_side(&self, editor: &mut Editor, side: usize, row: f64) {
        let rows = &self.line_rows[side];
        let whole = row.max(0.0).floor() as usize;
        let Some(v) = editor.view_mut(self.views[side]) else {
            return;
        };
        let before = |line: usize| {
            v.padding
                .as_ref()
                .and_then(|p| p.before.get(line).copied())
                .unwrap_or(0)
        };
        // The first line on or after the row; the rows before it down to
        // the row are its padding.
        let line = rows.partition_point(|&r| r < whole);
        let (line, sub) = match rows.get(line) {
            Some(&r) => (line, before(line).saturating_sub(r - whole)),
            None => match rows.last() {
                Some(&r) => (rows.len() - 1, before(rows.len() - 1) + 1 + (whole - r - 1)),
                None => (0, 0),
            },
        };
        v.scroll.first_line = line;
        v.scroll.first_sub = sub;
        v.scroll.line_fraction = (row - row.floor()).max(0.0) as f32;
    }

    fn scroll_both(&mut self, editor: &mut Editor, row: f64) {
        let max = self.diff.rows.len().saturating_sub(self.rows_visible) as f64;
        let row = row.clamp(0.0, max.max(0.0));
        self.scroll_side(editor, 0, row);
        self.scroll_side(editor, 1, row);
        self.synced = row;
    }

    fn go_to(&mut self, editor: &mut Editor, hunk: usize) {
        self.current = Some(hunk);
        let start = self.diff.hunks[hunk].start;
        self.scroll_both(editor, start.saturating_sub(self.rows_visible / 3) as f64);
    }

    pub fn next_change(&mut self, editor: &mut Editor) {
        let n = self.diff.hunks.len();
        if n == 0 {
            return;
        }
        let next = match self.current {
            Some(c) => (c + 1).min(n - 1),
            None => self
                .diff
                .hunks
                .iter()
                .position(|h| h.start as f64 >= self.synced)
                .unwrap_or(0),
        };
        self.go_to(editor, next);
    }

    pub fn previous_change(&mut self, editor: &mut Editor) {
        let n = self.diff.hunks.len();
        if n == 0 {
            return;
        }
        let prev = match self.current {
            Some(c) => c.saturating_sub(1),
            None => self
                .diff
                .hunks
                .iter()
                .rposition(|h| (h.start as f64) < self.synced)
                .unwrap_or(0),
        };
        self.go_to(editor, prev);
    }

    /// Word differences of the changed rows from `first` on, for drawing.
    fn ensure_words(&mut self, editor: &Editor, rows: Range<usize>) {
        let (Some(a), Some(b)) = (
            editor.buffer(self.shown[0]).map(|b| b.doc()),
            editor.buffer(self.shown[1]).map(|b| b.doc()),
        ) else {
            return;
        };
        for row in rows.start..rows.end.min(self.diff.rows.len()) {
            let r = &self.diff.rows[row];
            if r.kind != RowKind::Changed || self.words.contains_key(&row) {
                continue;
            }
            if let (Some(l), Some(rr)) = (r.left, r.right)
                && l < a.line_count()
                && rr < b.line_count()
            {
                self.words
                    .insert(row, diff_words(&a.line_text(l), &b.line_text(rr)));
            }
        }
    }

    /// The names, the summary and the previous/next buttons.
    fn toolbar(&mut self, ui: &mut Ui, editor: &mut Editor, labels: &[String; 2]) {
        ui.horizontal(|ui| {
            ui.label(format!("{}  |  {}", labels[0], labels[1]));
            ui.separator();
            ui.label(self.summary());
            ui.separator();
            if ui
                .button("Previous change")
                .on_hover_text("Shift+F8")
                .clicked()
            {
                self.previous_change(editor);
            }
            if ui.button("Next change").on_hover_text("F8").clicked() {
                self.next_change(editor);
            }
        });
    }

    /// Draws the compare view filling `ui`. `labels` name the two sides.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut Ui,
        editor: &mut Editor,
        highlighting: &mut Highlighting,
        keymap: &Keymap,
        labels: [String; 2],
        look: &crate::editor_view::Look,
        find_highlight: Option<&ochrepad_core::Searcher>,
    ) {
        let now = ui.input(|i| i.time);
        if let Some(wait) = self.refresh(editor, now) {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| self.toolbar(ui, editor, &labels));
        if self.shown.iter().any(|b| editor.buffer(*b).is_none()) {
            ui.label("A compared file was closed.");
            return;
        }
        let rect = ui.available_rect_before_wrap();
        ui.advance_cursor_after_rect(rect);
        let strip = Rect::from_min_max(Pos2::new(rect.max.x - STRIP_W, rect.min.y), rect.max);
        let half_w = ((rect.width() - STRIP_W - DIVIDER_W) / 2.0).max(10.0);
        let halves = [
            Rect::from_min_size(rect.min, Vec2::new(half_w, rect.height())),
            Rect::from_min_size(
                Pos2::new(rect.min.x + half_w + DIVIDER_W, rect.min.y),
                Vec2::new(half_w, rect.height()),
            ),
        ];
        let line_h = ui.fonts_mut(|f| f.row_height(&egui::FontId::monospace(look.font_size)));
        self.rows_visible = ((rect.height() / line_h).floor() as usize).max(1);

        // The overview strip doubles as a scrollbar for both sides.
        let strip_resp = ui.interact(
            strip,
            ui.id().with("compare-strip"),
            Sense::click_and_drag(),
        );
        if let Some(p) = strip_resp.interact_pointer_pos()
            && (strip_resp.dragged() || strip_resp.clicked())
        {
            let t = ((p.y - strip.min.y) / strip.height()).clamp(0.0, 1.0) as f64;
            let row = t * self.diff.rows.len() as f64 - self.rows_visible as f64 / 2.0;
            self.scroll_both(editor, row);
        }

        let visuals = ui.visuals().clone();
        let first = self.synced.floor() as usize;
        self.ensure_words(editor, first..first + self.rows_visible + 2);
        for side in 0..2 {
            let style = SideStyle {
                diff: &self.diff,
                line_rows: &self.line_rows[side],
                side,
                words: &self.words,
                current: self.current.and_then(|c| self.diff.hunks.get(c)).cloned(),
                visuals: &visuals,
            };
            let state = &mut self.states[side];
            state.set_look(look);
            let mut styler = highlighting.styler(self.shown[side]);
            let label = format!(
                "Compare {}: {}",
                if side == 0 { "left" } else { "right" },
                labels[side]
            );
            ui.scope_builder(UiBuilder::new().max_rect(halves[side]), |ui| {
                editor_view(
                    ui,
                    editor,
                    self.views[side],
                    state,
                    keymap,
                    &label,
                    Decorations {
                        styler: styler.as_mut().map(|s| s as &mut dyn LineStyler),
                        find_highlight,
                        row_style: Some(&style),
                        changes: None,
                        token_styles: &[],
                        completion: None,
                        underlines: &[],
                    },
                );
            });
        }

        // Scroll together: whichever side moved since the last frame leads.
        let (l, r) = (self.top_row(editor, 0), self.top_row(editor, 1));
        if (l - self.synced).abs() > 1e-4 {
            self.scroll_side(editor, 1, l);
            self.synced = l;
            ui.ctx().request_repaint();
        } else if (r - self.synced).abs() > 1e-4 {
            self.scroll_side(editor, 0, r);
            self.synced = r;
            ui.ctx().request_repaint();
        }

        // Divider and the overview strip with a marker per change.
        let painter = ui.painter_at(rect);
        painter.rect_filled(
            Rect::from_min_size(
                Pos2::new(rect.min.x + half_w, rect.min.y),
                Vec2::new(DIVIDER_W, rect.height()),
            ),
            0.0,
            visuals.widgets.noninteractive.bg_stroke.color,
        );
        painter.rect_filled(strip, 0.0, visuals.faint_bg_color);
        let total = self.diff.rows.len().max(1) as f32;
        for h in &self.diff.hunks {
            let top = strip.min.y + strip.height() * h.start as f32 / total;
            let bottom = (strip.min.y + strip.height() * h.end as f32 / total).max(top + 2.0);
            let kind = self.diff.rows[h.start].kind;
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(strip.min.x + 2.0, top),
                    Pos2::new(strip.max.x - 2.0, bottom),
                ),
                0.0,
                tint(kind, 170),
            );
        }
        let view_top = strip.min.y + strip.height() * self.synced as f32 / total;
        let view_h = strip.height() * self.rows_visible as f32 / total;
        painter.rect_stroke(
            Rect::from_min_size(
                Pos2::new(strip.min.x + 1.0, view_top),
                Vec2::new(STRIP_W - 2.0, view_h.min(strip.height())),
            ),
            1.0,
            Stroke::new(1.0, visuals.text_color()),
            egui::StrokeKind::Inside,
        );
    }
}

/// Row tints: changed is blue, so it never looks like the ochre selection.
/// Built from straight RGB (Color32 is premultiplied, so a weak tint cannot
/// be made strong again).
fn tint(kind: RowKind, alpha: u8) -> Color32 {
    let (r, g, b) = crate::theme::diff_rgb(kind);
    Color32::from_rgba_unmultiplied(r, g, b, alpha)
}

/// One side's colours from the diff, for the editor widget.
struct SideStyle<'a> {
    diff: &'a LineDiff,
    line_rows: &'a [usize],
    side: usize,
    words: &'a HashMap<usize, WordRanges>,
    current: Option<Range<usize>>,
    visuals: &'a egui::Visuals,
}

impl RowStyle for SideStyle<'_> {
    fn background(&self, row: RowPos, blank: bool) -> Option<Color32> {
        if blank {
            return Some(self.visuals.faint_bg_color);
        }
        let d = *self.line_rows.get(row.line)?;
        let kind = self.diff.rows.get(d)?.kind;
        let alpha = if self.visuals.dark_mode { 45 } else { 60 };
        let in_current = self.current.as_ref().is_some_and(|h| h.contains(&d));
        let alpha = if in_current { alpha + 25 } else { alpha };
        (kind != RowKind::Equal).then(|| tint(kind, alpha))
    }

    fn spans(&self, line: usize) -> Vec<(Range<usize>, Color32)> {
        let color = tint(RowKind::Changed, 140);
        let spans = self
            .line_rows
            .get(line)
            .and_then(|d| self.words.get(d))
            .map(|w| {
                if self.side == 0 {
                    w.0.clone()
                } else {
                    w.1.clone()
                }
            })
            .unwrap_or_default();
        spans.into_iter().map(|r| (r, color)).collect()
    }
}

/// All open comparisons.
#[derive(Default)]
pub struct Compares {
    pub map: HashMap<CompareId, Compare>,
    next: u64,
}

impl Compares {
    pub fn add(&mut self, compare: Compare) -> CompareId {
        self.next += 1;
        let id = CompareId(self.next);
        self.map.insert(id, compare);
        id
    }

    pub fn get(&self, id: CompareId) -> Option<&Compare> {
        self.map.get(&id)
    }

    pub fn get_mut(&mut self, id: CompareId) -> Option<&mut Compare> {
        self.map.get_mut(&id)
    }

    /// Closes a comparison, removing its views (and saved copy).
    pub fn remove(&mut self, id: CompareId, editor: &mut Editor) {
        if let Some(c) = self.map.remove(&id) {
            c.close(editor);
        }
    }

    /// Comparisons that show `buffer`.
    pub fn involving(&self, buffer: BufferId) -> Vec<CompareId> {
        self.map
            .iter()
            .filter(|(_, c)| c.involves(buffer))
            .map(|(id, _)| *id)
            .collect()
    }
}
