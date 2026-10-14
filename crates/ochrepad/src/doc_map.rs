//! The document map (PLAN_v0.2.0 N5): a zoomed-out strip of the document
//! beside the text, about 2 px per row and 1 px per cell. Each run of
//! non-blank text is a block in its highlight colour. It shows the same
//! rows as the editor (folded lines hidden, wrapped lines as their rows),
//! scrolls in proportion to the editor when the document is taller than the
//! strip, and marks the rows the editor shows.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use egui::{Color32, Mesh, Painter, Pos2, Rect, Vec2};
use ochrepad_core::{BufferId, DisplayMap, Document, RowPos};
use ochrepad_syntax::LineSpans;

/// Width of the strip.
pub const MAP_W: f32 = 100.0;
/// Height of one row.
pub const ROW_H: f32 = 2.0;
/// Width of one cell.
const COL_W: f32 = 1.0;
/// Opacity of the ink, so the map stays in the background.
const INK_ALPHA: u8 = 190;
/// Space between the strip's left edge and the text.
const PAD: f32 = 4.0;

/// The part of the document the map shows this frame.
pub struct MapView {
    /// The rows drawn, from the top of the strip.
    pub rows: Vec<RowPos>,
    /// Indices into `rows` of the rows the editor shows.
    pub viewport: Range<usize>,
    /// Height the viewport marker can move through, for dragging it.
    pub track: f32,
}

impl MapView {
    /// The map for an editor whose top is at scrollbar offset `offset` of
    /// `max_offset`, showing `top` and `rows_full` rows below it.
    pub fn new(
        rows: &DisplayMap,
        strip: Rect,
        top: RowPos,
        offset: f64,
        max_offset: f64,
        rows_full: usize,
    ) -> Self {
        let capacity = (strip.height() / ROW_H).floor().max(1.0) as usize;
        let total = rows.visible_line_count();
        // When the document is taller than the strip, the map scrolls so that
        // its top moves from the first to the last screenful in step with
        // the editor.
        let first = if total <= capacity || max_offset <= 0.0 {
            0.0
        } else {
            offset / max_offset * (total - capacity) as f64
        };
        let (map_top, _) = rows.pos_at_offset(first);
        let shown: Vec<RowPos> = rows.rows_from(map_top, capacity).collect();
        let start = shown.binary_search(&top).unwrap_or_else(|i| i);
        let viewport = start..(start + rows_full).min(shown.len().max(start));
        let height = (shown.len() as f32 * ROW_H).min(strip.height());
        Self {
            track: (height - rows_full as f32 * ROW_H).max(1.0),
            rows: shown,
            viewport,
        }
    }

    /// The viewport marker.
    pub fn viewport_rect(&self, strip: Rect) -> Rect {
        let y0 = strip.min.y + self.viewport.start as f32 * ROW_H;
        let h = self.viewport.len().max(1) as f32 * ROW_H;
        Rect::from_min_size(Pos2::new(strip.min.x, y0), Vec2::new(strip.width(), h))
    }

    /// The row under `y`, if the map shows one there.
    pub fn row_at(&self, strip: Rect, y: f32) -> Option<RowPos> {
        let i = ((y - strip.min.y) / ROW_H).floor();
        (i >= 0.0)
            .then(|| self.rows.get(i as usize).copied())
            .flatten()
    }
}

/// One line's ink: runs of non-blank cells with their colour.
struct MapLine {
    text: String,
    spans: Option<LineSpans>,
    runs: Arc<[(u32, u32, Color32)]>,
}

/// Lines' ink for one buffer, tab width and theme. An entry is reused while
/// its line's text and spans are unchanged, so an edit only redoes the lines
/// it touched.
#[derive(Default)]
pub struct MapCache {
    key: Option<(BufferId, usize, Color32)>,
    lines: HashMap<usize, MapLine>,
}

impl MapCache {
    fn runs(
        &mut self,
        doc: &Document,
        line: usize,
        spans: Option<&LineSpans>,
        plain: Color32,
        tab_width: usize,
    ) -> Arc<[(u32, u32, Color32)]> {
        let text = doc.line_text(line);
        if let Some(e) = self.lines.get(&line)
            && e.text == text
            && match (&e.spans, spans) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
        {
            return e.runs.clone();
        }
        let runs = ink(&text, spans.map(|s| &s[..]), plain, tab_width);
        self.lines.insert(
            line,
            MapLine {
                text,
                spans: spans.cloned(),
                runs: runs.clone(),
            },
        );
        runs
    }
}

/// Ink colour: the text colour, faded so the map stays in the background.
fn faded(c: Color32) -> Color32 {
    let [r, g, b, _] = c.to_srgba_unmultiplied();
    Color32::from_rgba_unmultiplied(r, g, b, INK_ALPHA)
}

/// Runs of non-blank cells of `text`, split where the colour changes.
fn ink(
    text: &str,
    spans: Option<&[ochrepad_syntax::Span]>,
    plain: Color32,
    tab_width: usize,
) -> Arc<[(u32, u32, Color32)]> {
    let bytes = text.as_bytes();
    let mut out: Vec<(u32, u32, Color32)> = Vec::new();
    let mut span = 0;
    for (byte, col, w) in ochrepad_core::position::cells(text, tab_width) {
        if matches!(bytes[byte], b' ' | b'\t') {
            continue;
        }
        let color = match spans {
            Some(s) => {
                while span < s.len() && s[span].end <= byte {
                    span += 1;
                }
                match s.get(span).filter(|sp| sp.start <= byte) {
                    Some(sp) => {
                        let [r, g, b, _] = sp.color;
                        Color32::from_rgba_unmultiplied(r, g, b, INK_ALPHA)
                    }
                    None => plain,
                }
            }
            None => plain,
        };
        let (col, end) = (col as u32, (col + w) as u32);
        match out.last_mut() {
            Some(last) if last.1 == col && last.2 == color => last.1 = end,
            _ => out.push((col, end, color)),
        }
    }
    out.into()
}

/// Paints the map: the ink of `view`'s rows and the viewport marker.
#[allow(clippy::too_many_arguments)]
pub fn paint(
    painter: &Painter,
    strip: Rect,
    view: &MapView,
    rows: &DisplayMap,
    cache: &mut MapCache,
    buffer: BufferId,
    spans_of: &dyn Fn(usize) -> Option<LineSpans>,
    visuals: &egui::Visuals,
    tab_width: usize,
    hovered: bool,
) {
    let plain = faded(visuals.weak_text_color());
    let key = (buffer, tab_width, plain);
    if cache.key != Some(key) {
        cache.key = Some(key);
        cache.lines.clear();
    }
    let doc = rows.doc();
    let mut mesh = Mesh::default();
    for (i, &row) in view.rows.iter().enumerate() {
        let spans = spans_of(row.line);
        let runs = cache.runs(doc, row.line, spans.as_ref(), plain, tab_width);
        let (start_col, indent) = rows.row_cols(row);
        let end_col = if rows.is_last_in_line(row) {
            u32::MAX
        } else {
            rows.row_cols(RowPos::new(row.line, row.sub + 1)).0 as u32
        };
        let y = strip.min.y + i as f32 * ROW_H;
        let x0 = strip.min.x + PAD + (indent as f32 - start_col as f32) * COL_W;
        for &(a, b, color) in runs.iter() {
            let (a, b) = (a.max(start_col as u32), b.min(end_col));
            if a >= b {
                continue;
            }
            let x = x0 + a as f32 * COL_W;
            if x >= strip.max.x {
                break;
            }
            let r = Rect::from_min_max(
                Pos2::new(x, y),
                Pos2::new((x0 + b as f32 * COL_W).min(strip.max.x), y + ROW_H * 0.75),
            );
            mesh.add_colored_rect(r, color);
        }
    }
    painter.with_clip_rect(strip).add(mesh);
    if cache.lines.len() > 4 * view.rows.len() + 256 {
        let lines: std::collections::HashSet<usize> = view.rows.iter().map(|r| r.line).collect();
        cache.lines.retain(|l, _| lines.contains(l));
    }

    let marker = view.viewport_rect(strip);
    let [r, g, b, _] = visuals.widgets.hovered.bg_fill.to_srgba_unmultiplied();
    let alpha = if hovered { 110 } else { 70 };
    painter.rect_filled(marker, 0.0, Color32::from_rgba_unmultiplied(r, g, b, alpha));
}
