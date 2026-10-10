//! The side-by-side compare view (PLAN M12): read-only, rows aligned by the
//! core line diff (filler rows opposite insertions and deletions), word-level
//! highlights on changed lines, one shared scroll position, change
//! navigation and summary counts. Text is drawn with the editor's own line
//! layout and painter.

use std::collections::HashMap;
use std::ops::Range;

use egui::{Color32, FontId, Id, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use ochrepad_core::diff::{LineDiff, RowKind, diff_lines, diff_words};
use ochrepad_core::{BufferId, Document, Editor};

use crate::editor_view::paint_line_text;
use crate::layout::{LayoutCache, Styling};

/// Quiet time after an edit before the diff is recomputed.
pub const RECOMPUTE_DEBOUNCE_SECS: f64 = 0.3;
const STRIP_W: f32 = 14.0;
const DIVIDER_W: f32 = 4.0;
const PAD: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CompareId(pub u64);

/// What one side of a comparison shows.
pub enum CompareSide {
    /// An open buffer (follows its edits).
    Buffer(BufferId),
    /// The buffer's file as it is on disk.
    Saved { buffer: BufferId, doc: Document },
}

impl CompareSide {
    pub fn buffer(&self) -> BufferId {
        match self {
            CompareSide::Buffer(b) | CompareSide::Saved { buffer: b, .. } => *b,
        }
    }

    fn doc<'a>(&'a self, editor: &'a Editor) -> Option<&'a Document> {
        match self {
            CompareSide::Buffer(b) => editor.buffer(*b).map(|b| b.doc()),
            CompareSide::Saved { doc, .. } => Some(doc),
        }
    }

    fn revision(&self, editor: &Editor) -> u64 {
        match self {
            CompareSide::Buffer(b) => editor.buffer(*b).map_or(0, |b| b.revision()),
            CompareSide::Saved { .. } => 0,
        }
    }

    pub fn label(&self, name: &dyn Fn(BufferId) -> String) -> String {
        match self {
            CompareSide::Buffer(b) => name(*b),
            CompareSide::Saved { buffer, .. } => format!("{} (saved)", name(*buffer)),
        }
    }

    /// Layout-cache key: buffers by revision; a saved copy never changes.
    fn cache_key(&self, editor: &Editor) -> (BufferId, u64) {
        match self {
            CompareSide::Buffer(b) => (*b, self.revision(editor)),
            CompareSide::Saved { .. } => (BufferId(u64::MAX), 0),
        }
    }
}

/// Changed byte ranges on the left and right line of a changed row.
type WordRanges = (Vec<Range<usize>>, Vec<Range<usize>>);

pub struct Compare {
    pub left: CompareSide,
    pub right: CompareSide,
    pub diff: LineDiff,
    computed: (u64, u64),
    /// Newest revisions seen and when, for the recompute debounce.
    seen: Option<((u64, u64), f64)>,
    /// First visible row (fractional).
    pub scroll: f64,
    x_off: f32,
    /// Index into `diff.hunks` of the change navigated to.
    pub current: Option<usize>,
    words: HashMap<usize, WordRanges>,
    caches: [LayoutCache; 2],
    rows_visible: usize,
}

impl Compare {
    pub fn new(editor: &Editor, left: CompareSide, right: CompareSide) -> Self {
        let mut c = Self {
            left,
            right,
            diff: LineDiff::default(),
            computed: (u64::MAX, u64::MAX),
            seen: None,
            scroll: 0.0,
            x_off: 0.0,
            current: None,
            words: HashMap::new(),
            caches: [LayoutCache::default(), LayoutCache::default()],
            rows_visible: 30,
        };
        c.recompute(editor);
        c
    }

    pub fn involves(&self, buffer: BufferId) -> bool {
        self.left.buffer() == buffer || self.right.buffer() == buffer
    }

    fn recompute(&mut self, editor: &Editor) {
        let (Some(a), Some(b)) = (self.left.doc(editor), self.right.doc(editor)) else {
            return;
        };
        self.diff = diff_lines(a, b);
        self.words.clear();
        self.computed = (self.left.revision(editor), self.right.revision(editor));
        if self.current.is_some_and(|c| c >= self.diff.hunks.len()) {
            self.current = None;
        }
    }

    /// Recomputes after edits have paused; returns whether one is pending.
    fn refresh(&mut self, editor: &Editor, now: f64) -> Option<f64> {
        let revs = (self.left.revision(editor), self.right.revision(editor));
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

    fn go_to(&mut self, hunk: usize) {
        self.current = Some(hunk);
        let start = self.diff.hunks[hunk].start;
        self.scroll = start.saturating_sub(self.rows_visible / 3) as f64;
    }

    pub fn next_change(&mut self) {
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
                .position(|h| h.start as f64 >= self.scroll)
                .unwrap_or(0),
        };
        self.go_to(next);
    }

    pub fn previous_change(&mut self) {
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
                .rposition(|h| (h.start as f64) < self.scroll)
                .unwrap_or(0),
        };
        self.go_to(prev);
    }

    fn word_ranges(&mut self, row: usize, a: &Document, b: &Document) -> &WordRanges {
        let r = &self.diff.rows[row];
        let (l, rr) = (r.left, r.right);
        self.words.entry(row).or_insert_with(|| match (l, rr) {
            (Some(l), Some(rr)) => diff_words(&a.line_text(l), &b.line_text(rr)),
            _ => (Vec::new(), Vec::new()),
        })
    }

    /// The names, the summary and the previous/next buttons.
    fn toolbar(&mut self, ui: &mut Ui, name: &dyn Fn(BufferId) -> String) {
        ui.horizontal(|ui| {
            ui.label(format!(
                "{}  |  {}",
                self.left.label(name),
                self.right.label(name)
            ));
            ui.separator();
            ui.label(self.summary());
            ui.separator();
            if ui
                .button("Previous change")
                .on_hover_text("Shift+F8")
                .clicked()
            {
                self.previous_change();
            }
            if ui.button("Next change").on_hover_text("F8").clicked() {
                self.next_change();
            }
        });
    }

    /// Draws the compare view filling `ui`.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        editor: &Editor,
        name: &dyn Fn(BufferId) -> String,
        id: CompareId,
    ) {
        let now = ui.input(|i| i.time);
        if let Some(wait) = self.refresh(editor, now) {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| self.toolbar(ui, name));
        let (Some(a), Some(b)) = (self.left.doc(editor), self.right.doc(editor)) else {
            ui.label("A compared file was closed.");
            return;
        };
        let (a, b) = (a.clone(), b.clone());
        let rect = ui.available_rect_before_wrap();
        ui.advance_cursor_after_rect(rect);
        let response = ui.interact(
            rect,
            Id::new(("ochrepad-compare", id.0)),
            Sense::click_and_drag(),
        );

        let font = FontId::monospace(14.0);
        let (cell_w, line_h) = ui.fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
        let rows = self.diff.rows.len();
        let strip = Rect::from_min_max(Pos2::new(rect.max.x - STRIP_W, rect.min.y), rect.max);
        let half_w = ((rect.width() - STRIP_W - DIVIDER_W) / 2.0).max(10.0);
        let halves = [
            Rect::from_min_size(rect.min, Vec2::new(half_w, rect.height())),
            Rect::from_min_size(
                Pos2::new(rect.min.x + half_w + DIVIDER_W, rect.min.y),
                Vec2::new(half_w, rect.height()),
            ),
        ];
        self.rows_visible = ((rect.height() / line_h).floor() as usize).max(1);
        let max_scroll = rows.saturating_sub(self.rows_visible) as f64;

        // Wheel scrolls both sides together.
        if response.contains_pointer() {
            let d = ui.input(|i| i.smooth_scroll_delta);
            if d != Vec2::ZERO {
                self.scroll -= (d.y / line_h) as f64;
                self.x_off = (self.x_off - d.x).max(0.0);
                ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
            }
        }
        // The overview strip doubles as a scrollbar.
        if let Some(p) = response.interact_pointer_pos()
            && (response.dragged() || response.clicked())
            && strip.contains(p)
        {
            let t = ((p.y - strip.min.y) / strip.height()).clamp(0.0, 1.0) as f64;
            self.scroll = t * rows as f64 - self.rows_visible as f64 / 2.0;
        }
        self.scroll = self.scroll.clamp(0.0, max_scroll);

        let visuals = ui.visuals().clone();
        // Row tints (changed is blue, so it never looks like the ochre
        // selection), and stronger versions of the same hues for changed words
        // and the overview strip. (Built from straight RGB: Color32 stores
        // premultiplied values, so a weak tint cannot be made strong again.)
        let alpha = if visuals.dark_mode { 45 } else { 60 };
        let hue = |kind: RowKind| match kind {
            RowKind::Removed => (225, 70, 70),
            RowKind::Added => (60, 185, 90),
            _ => (80, 145, 235),
        };
        let tint = |kind: RowKind, a: u8| {
            let (r, g, b) = hue(kind);
            Color32::from_rgba_unmultiplied(r, g, b, a)
        };
        let (removed, added, changed) = (
            tint(RowKind::Removed, alpha),
            tint(RowKind::Added, alpha),
            tint(RowKind::Changed, alpha),
        );
        let filler = visuals.faint_bg_color;
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, visuals.extreme_bg_color);

        let first = self.scroll.floor() as usize;
        let y0 = rect.min.y - self.scroll.fract() as f32 * line_h;
        let last = (first + self.rows_visible + 2).min(rows);
        let current = self.current.and_then(|c| self.diff.hunks.get(c)).cloned();
        let tab_width = editor.settings.tab_width;
        let text_color = visuals.text_color();

        for (side, half) in halves.iter().enumerate() {
            let doc = if side == 0 { &a } else { &b };
            let digits = doc.line_count().to_string().len().max(3);
            let gutter_w = (digits + 1) as f32 * cell_w + PAD;
            let gutter = Rect::from_min_size(half.min, Vec2::new(gutter_w, half.height()));
            let text_rect = Rect::from_min_max(Pos2::new(gutter.max.x, half.min.y), half.max);
            painter.rect_filled(gutter, 0.0, visuals.extreme_bg_color);
            let text_painter = painter.with_clip_rect(text_rect.intersect(painter.clip_rect()));
            let gutter_painter = painter.with_clip_rect(gutter);
            let x0 = text_rect.min.x + PAD - self.x_off;
            let key = if side == 0 {
                self.left.cache_key(editor)
            } else {
                self.right.cache_key(editor)
            };
            for row in first..last {
                let y = y0 + (row - first) as f32 * line_h;
                let r = self.diff.rows[row].clone();
                let line = if side == 0 { r.left } else { r.right };
                let row_rect = Rect::from_min_size(
                    Pos2::new(text_rect.min.x, y),
                    Vec2::new(text_rect.width(), line_h),
                );
                let bg = match (r.kind, line) {
                    (_, None) => Some(filler),
                    (RowKind::Equal, _) => None,
                    (RowKind::Removed, _) => Some(removed),
                    (RowKind::Added, _) => Some(added),
                    (RowKind::Changed, _) => Some(changed),
                };
                if let Some(bg) = bg {
                    text_painter.rect_filled(row_rect, 0.0, bg);
                }
                if current.as_ref().is_some_and(|h| h.contains(&row)) {
                    gutter_painter.rect_filled(
                        Rect::from_min_size(
                            Pos2::new(gutter.max.x - 3.0, y),
                            Vec2::new(3.0, line_h),
                        ),
                        0.0,
                        visuals.selection.bg_fill,
                    );
                }
                let Some(line) = line else { continue };
                let layout = self.caches[side].get(
                    doc,
                    key,
                    line,
                    cell_w,
                    tab_width,
                    text_color,
                    Styling::Any,
                );
                if r.kind == RowKind::Changed {
                    let words = self.word_ranges(row, &a, &b);
                    let words = if side == 0 { &words.0 } else { &words.1 };
                    for w in words {
                        let span = Rect::from_min_max(
                            Pos2::new(x0 + layout.x_of(w.start), y),
                            Pos2::new(x0 + layout.x_of(w.end), y + line_h),
                        );
                        text_painter.rect_filled(span, 2.0, tint(RowKind::Changed, 140));
                    }
                }
                paint_line_text(&text_painter, &layout, x0, y, &font, line_h);
                gutter_painter.text(
                    Pos2::new(gutter.max.x - cell_w, y),
                    egui::Align2::RIGHT_TOP,
                    (line + 1).to_string(),
                    font.clone(),
                    visuals.weak_text_color(),
                );
            }
            self.caches[side].retain_near(first, last.max(first + 1));
        }

        // Divider and the overview strip with a marker per change.
        painter.rect_filled(
            Rect::from_min_size(
                Pos2::new(rect.min.x + half_w, rect.min.y),
                Vec2::new(DIVIDER_W, rect.height()),
            ),
            0.0,
            visuals.widgets.noninteractive.bg_stroke.color,
        );
        painter.rect_filled(strip, 0.0, visuals.faint_bg_color);
        let total = rows.max(1) as f32;
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
        let view_top = strip.min.y + strip.height() * first as f32 / total;
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

    /// Comparisons that show `buffer`.
    pub fn involving(&self, buffer: BufferId) -> Vec<CompareId> {
        self.map
            .iter()
            .filter(|(_, c)| c.involves(buffer))
            .map(|(id, _)| *id)
            .collect()
    }
}
