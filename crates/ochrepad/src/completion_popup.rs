//! The completion popup (PLAN_completion C1): a list of candidates below
//! the caret while a word is typed, or after Ctrl+Space. The editor widget
//! asks a [`CompletionSource`] for candidates, ranks them with
//! `ochrepad_core::complete`, and lets the popup take the keys that move in
//! the list or accept from it. Accepting is `Action::Complete`, one undo
//! step like any edit.

use egui::{Align2, Color32, FontId, Id, Pos2, Rect, Sense, Ui, Vec2};
use ochrepad_core::complete::{self, Candidate, Kind, Ranked};
use ochrepad_core::{Action, BufferId, ByteOffset, Document};

/// The most rows shown at once.
const ROWS: usize = 10;
/// Candidates kept after ranking.
const KEEP: usize = 100;

/// What the widget asks for: the word typed so far at the primary caret.
pub struct CompletionRequest<'a> {
    pub buffer: BufferId,
    pub doc: &'a Document,
    pub caret: ByteOffset,
    /// Where the word being typed starts (`prefix` is the text from here to
    /// the caret).
    pub start: ByteOffset,
    pub prefix: &'a str,
    /// Asked for with Ctrl+Space rather than by typing.
    pub invoked: bool,
    /// The word comes right after a member separator (`items.`, `std::`,
    /// `node->`), in a language that has them.
    pub member: bool,
}

/// Where candidates come from (the app's documents, keywords, folder index,
/// language servers).
pub trait CompletionSource {
    /// Candidates for `req`, in any order; the popup ranks them.
    fn candidates(&mut self, req: &CompletionRequest<'_>) -> Vec<Candidate>;
    /// Characters besides letters, digits and `_` that words have in this
    /// buffer's language (`-` in CSS).
    fn word_chars(&self, _buffer: BufferId) -> &[char] {
        &[]
    }
    /// Whether this buffer's language has member separators (`.` in prose
    /// ends a sentence).
    fn members(&self, _buffer: BufferId) -> bool {
        false
    }
    /// Changes when the same request would now give other candidates (a
    /// server answered); the popup asks again.
    fn generation(&self) -> u64 {
        0
    }
    /// Whether more candidates may come for the last request (a thread or
    /// a server is still looking): an empty list then waits instead of
    /// closing, until the generation changes.
    fn waiting(&self) -> bool {
        false
    }
    /// The user accepted `text` for `prefix` (ranking learns from it).
    fn accepted(&mut self, _buffer: BufferId, _text: &str) {}
}

/// How completion behaves (the preferences).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompletionSettings {
    /// Opens by itself while typing.
    pub auto: bool,
    /// After this many characters of a word.
    pub min_chars: usize,
    /// Enter accepts (otherwise only Tab does, and Enter breaks the line).
    pub enter_accepts: bool,
}

impl Default for CompletionSettings {
    fn default() -> Self {
        Self {
            auto: true,
            min_chars: 2,
            enter_accepts: true,
        }
    }
}

/// The open popup.
pub struct Popup {
    /// The word's start when it opened; the popup closes when the caret
    /// leaves that word.
    start: ByteOffset,
    prefix: String,
    items: Vec<Ranked>,
    selected: usize,
    /// The first row shown.
    scroll: usize,
    invoked: bool,
    generation: u64,
}

/// What a key did to the popup.
pub enum KeyResult {
    /// Not the popup's key; the editor handles it.
    Pass,
    /// Taken (moved in the list, or closed it).
    Taken,
    /// Accept the selected item: replace the word's rest after the caret
    /// too (Tab) or not (Enter).
    Accept { replace: bool },
}

/// The popup's state in an editor (kept in `EditorViewState`).
#[derive(Default)]
pub struct CompletionState {
    pub popup: Option<Popup>,
    pub settings: CompletionSettings,
    /// Ctrl+Space was pressed this frame.
    invoke: bool,
    /// A word character was typed this frame.
    typed: bool,
    /// A row was clicked: accept it on the next frame (the document does
    /// not change while the frame is drawn).
    clicked: Option<usize>,
    /// Where the popup was drawn last frame: presses there are not the
    /// editor's.
    drawn: Option<Rect>,
    /// Nothing to show yet, but the source is still looking.
    waiting: Option<Waiting>,
}

/// A popup that had nothing to show while its source was still looking.
struct Waiting {
    start: ByteOffset,
    prefix: String,
    invoked: bool,
    generation: u64,
}

impl CompletionState {
    pub fn is_open(&self) -> bool {
        self.popup.is_some()
    }

    pub fn close(&mut self) {
        self.popup = None;
        self.waiting = None;
    }

    /// The popup was not drawn this frame (its caret is off screen).
    pub fn paint_nothing(&mut self) {
        self.drawn = None;
    }

    /// Whether the popup is open or about to open (typed, Ctrl+Space).
    pub fn active(&self) -> bool {
        self.popup.is_some() || self.typed || self.invoke || self.waiting.is_some()
    }

    /// Whether `p` is on the popup (as drawn last frame).
    pub fn covers(&self, p: Pos2) -> bool {
        self.popup.is_some() && self.drawn.is_some_and(|r| r.contains(p))
    }

    /// Ctrl+Space.
    pub fn invoke(&mut self) {
        self.invoke = true;
    }

    /// Text was typed: the popup may open or follow it.
    pub fn typed(&mut self, text: &str, word_chars: &[char]) {
        if text
            .chars()
            .last()
            .is_some_and(|c| complete::is_word_char(c, word_chars))
            && text.chars().count() <= 4
        {
            self.typed = true;
        }
    }

    /// The candidates shown, best first.
    pub fn shown(&self) -> Vec<&Candidate> {
        self.popup
            .as_ref()
            .map_or_else(Vec::new, |p| p.items.iter().map(|r| &r.candidate).collect())
    }

    /// The labels shown, best first (tests and accessibility).
    pub fn labels(&self) -> Vec<String> {
        self.popup.as_ref().map_or_else(Vec::new, |p| {
            p.items.iter().map(|r| r.candidate.label.clone()).collect()
        })
    }

    /// The popup's keys: Up, Down, PageUp, PageDown, Home and End move;
    /// Tab (and Enter, if it accepts) accept; Escape closes.
    pub fn key(&mut self, key: egui::Key, modifiers: egui::Modifiers) -> KeyResult {
        let enter_accepts = self.settings.enter_accepts;
        let Some(p) = &mut self.popup else {
            return KeyResult::Pass;
        };
        if modifiers.any() {
            return KeyResult::Pass;
        }
        let n = p.items.len();
        let page = ROWS - 1;
        match key {
            egui::Key::ArrowDown => p.selected = (p.selected + 1) % n.max(1),
            egui::Key::ArrowUp => p.selected = (p.selected + n.max(1) - 1) % n.max(1),
            egui::Key::PageDown => p.selected = (p.selected + page).min(n.saturating_sub(1)),
            egui::Key::PageUp => p.selected = p.selected.saturating_sub(page),
            egui::Key::Escape => {
                self.popup = None;
                return KeyResult::Taken;
            }
            egui::Key::Tab => return KeyResult::Accept { replace: true },
            egui::Key::Enter if enter_accepts => return KeyResult::Accept { replace: false },
            _ => return KeyResult::Pass,
        }
        p.reveal();
        KeyResult::Taken
    }

    /// The action that accepts the selected item (or row `clicked`), and
    /// closes the popup. `after` is how many word characters follow the
    /// caret (replaced when `replace`).
    pub fn accept(
        &mut self,
        replace: bool,
        after: usize,
        source: Option<&mut (dyn CompletionSource + '_)>,
        buffer: BufferId,
    ) -> Option<Action> {
        let p = self.popup.take()?;
        let index = self.clicked.take().unwrap_or(p.selected);
        let item = p.items.get(index)?;
        let text = item.candidate.text().to_owned();
        if let Some(s) = source {
            s.accepted(buffer, &text);
        }
        Some(Action::Complete {
            before: p.prefix.chars().count(),
            after: if replace { after } else { 0 },
            text,
        })
    }

    /// A row clicked last frame, to accept now.
    pub fn take_click(&mut self) -> bool {
        self.clicked.is_some() && self.popup.is_some()
    }

    /// After the frame's edits: opens, follows or closes the popup for the
    /// word at the primary caret. `caret` is `None` with a selection.
    pub fn update(
        &mut self,
        doc: &Document,
        buffer: BufferId,
        caret: Option<ByteOffset>,
        context: ochrepad_syntax::TextContext,
        source: Option<&mut (dyn CompletionSource + '_)>,
    ) {
        let (invoke, typed) = (
            std::mem::take(&mut self.invoke),
            std::mem::take(&mut self.typed),
        );
        let waiting = self.waiting.take();
        let Some(source) = source else {
            self.popup = None;
            return;
        };
        let Some(caret) = caret else {
            self.popup = None;
            return;
        };
        let start = complete::word_start(doc, caret, source.word_chars(buffer));
        let prefix = doc.slice(start..caret);
        // (Only looked at when the popup has something to do: this runs
        // every frame.)
        let busy = invoke || typed || self.popup.is_some() || waiting.is_some();
        let member = busy && source.members(buffer) && {
            // A short look back is enough.
            let line_start = doc.line_start(doc.line_of(start));
            let mut from = ByteOffset(start.0.saturating_sub(64).max(line_start.0));
            while !doc.is_char_boundary(from) {
                from.0 += 1;
            }
            complete::ends_with_member_separator(&doc.slice(from..start))
        };
        // After a separator one character is enough to open by itself.
        let min_chars = if member { 1 } else { self.settings.min_chars };
        let open = self.popup.is_some();
        // The caret left the word the popup was for.
        if self.popup.as_ref().is_some_and(|p| p.start != start) {
            self.popup = None;
        }
        let waiting = waiting.filter(|w| w.start == start);
        let invoked = invoke
            || self.popup.as_ref().is_some_and(|p| p.invoked)
            || waiting.as_ref().is_some_and(|w| w.invoked);
        // Not by itself in comments and strings (Ctrl+Space still works).
        if !invoked && context != ochrepad_syntax::TextContext::Code {
            self.popup = None;
            return;
        }
        let wanted = invoke
            || (open && self.popup.is_some())
            || waiting.is_some()
            || (typed && self.settings.auto && prefix.chars().count() >= min_chars);
        if !wanted || (!invoked && prefix.chars().count() < min_chars) {
            self.popup = None;
            return;
        }
        let generation = source.generation();
        // Still waiting, and nothing new to ask.
        if let Some(w) = waiting
            && !invoke
            && w.prefix == prefix
            && w.generation == generation
        {
            self.waiting = Some(w);
            return;
        }
        let stale = self
            .popup
            .as_ref()
            .is_none_or(|p| p.prefix != prefix || p.generation != generation || invoke);
        if !stale {
            return;
        }
        let req = CompletionRequest {
            buffer,
            doc,
            caret,
            start,
            prefix: &prefix,
            invoked,
            member,
        };
        let candidates = source.candidates(&req);
        let items = if prefix.is_empty() {
            // Ctrl+Space on no word: everything, best first.
            let mut all: Vec<Ranked> = candidates
                .into_iter()
                .map(|c| Ranked {
                    score: (c.boost * 100.0) as i64,
                    candidate: c,
                })
                .collect();
            all.sort_by(|a, b| {
                b.candidate
                    .tier
                    .cmp(&a.candidate.tier)
                    .then_with(|| b.score.cmp(&a.score))
                    .then_with(|| a.candidate.source.cmp(&b.candidate.source))
                    .then_with(|| a.candidate.label.cmp(&b.candidate.label))
            });
            all.dedup_by(|a, b| a.candidate.label == b.candidate.label);
            all.truncate(KEEP);
            all
        } else {
            complete::rank(&prefix, candidates, KEEP)
        };
        if items.is_empty() {
            self.popup = None;
            if source.waiting() {
                self.waiting = Some(Waiting {
                    start,
                    prefix,
                    invoked,
                    generation,
                });
            }
            return;
        }
        // Keep the selected item when the list is refreshed.
        let previous = self
            .popup
            .as_ref()
            .and_then(|p| p.items.get(p.selected))
            .map(|r| r.candidate.label.clone());
        let selected = previous
            .and_then(|l| items.iter().position(|r| r.candidate.label == l))
            .unwrap_or(0);
        let mut p = Popup {
            start,
            prefix,
            items,
            selected,
            scroll: 0,
            invoked,
            generation,
        };
        p.reveal();
        self.popup = Some(p);
    }

    /// Draws the popup with its top-left near `caret` (the caret's cell, on
    /// screen); `line_h` is a text row's height. Clicked rows are accepted
    /// on the next frame.
    pub fn paint(&mut self, ui: &Ui, id: Id, caret: Rect, font: &FontId) {
        let Some(p) = &mut self.popup else {
            self.drawn = None;
            return;
        };
        let visuals = ui.visuals().clone();
        let row_h = (caret.height() + 4.0).round();
        let shown = p.items.len().min(ROWS);
        let (label_w, detail_w) = ui.fonts_mut(|f| {
            let mut label_w: f32 = 0.0;
            let mut detail_w: f32 = 0.0;
            for r in &p.items {
                let w = f
                    .layout_no_wrap(r.candidate.label.clone(), font.clone(), Color32::WHITE)
                    .size()
                    .x;
                label_w = label_w.max(w);
                if let Some(d) = &r.candidate.detail {
                    let w = f
                        .layout_no_wrap(d.clone(), font.clone(), Color32::WHITE)
                        .size()
                        .x;
                    detail_w = detail_w.max(w);
                }
            }
            (label_w, detail_w)
        });
        let marker_w = row_h;
        let pad = 6.0;
        let detail_w = detail_w.min(260.0);
        let width = (marker_w
            + pad
            + label_w
            + if detail_w > 0.0 {
                3.0 * pad + detail_w
            } else {
                0.0
            }
            + pad)
            .clamp(160.0, 640.0);
        let height = shown as f32 * row_h + 2.0;
        // Below the caret's line, or above it when there is no room.
        let screen = ui.ctx().content_rect();
        let mut pos = Pos2::new(caret.min.x - marker_w - pad, caret.max.y + 2.0);
        if pos.y + height > screen.max.y && caret.min.y - height - 2.0 > screen.min.y {
            pos.y = caret.min.y - height - 2.0;
        }
        pos.x = pos.x.min(screen.max.x - width).max(screen.min.x);
        self.drawn = Some(Rect::from_min_size(pos, Vec2::new(width, height)));
        let mut clicked = None;
        let mut scroll_by = 0i64;
        egui::Area::new(id)
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ui.ctx(), |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Completions")
                });
                if response.hovered() {
                    let dy = ui.input(|i| i.smooth_scroll_delta.y);
                    if dy != 0.0 {
                        scroll_by = -(dy / row_h).round() as i64;
                        ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
                    }
                }
                let painter = ui.painter_at(rect.expand(8.0));
                painter.add(visuals.popup_shadow.as_shape(rect, 6.0));
                painter.rect(
                    rect,
                    6.0,
                    visuals.window_fill,
                    visuals.window_stroke,
                    egui::StrokeKind::Inside,
                );
                let accent = crate::theme::accent(&visuals);
                for (row, index) in (p.scroll..p.items.len()).take(ROWS).enumerate() {
                    let r = &p.items[index];
                    let top = rect.min.y + 1.0 + row as f32 * row_h;
                    let row_rect = Rect::from_min_size(
                        Pos2::new(rect.min.x + 1.0, top),
                        Vec2::new(rect.width() - 2.0, row_h),
                    );
                    // Clickable but never focused: the editor keeps the
                    // keyboard.
                    let row_resp = ui.interact(row_rect, id.with(("row", index)), Sense::CLICK);
                    let label = r.candidate.label.clone();
                    row_resp.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label)
                    });
                    if row_resp.clicked() {
                        clicked = Some(index);
                    }
                    if index == p.selected {
                        painter.rect_filled(row_rect.shrink(1.0), 4.0, visuals.selection.bg_fill);
                    } else if row_resp.hovered() {
                        painter.rect_filled(
                            row_rect.shrink(1.0),
                            4.0,
                            visuals.widgets.hovered.weak_bg_fill,
                        );
                    }
                    // The kind marker.
                    let (letter, color) = marker(r.candidate.kind, &visuals);
                    let m = Rect::from_center_size(
                        Pos2::new(
                            row_rect.min.x + pad + marker_w / 2.0 - 2.0,
                            row_rect.center().y,
                        ),
                        Vec2::splat(row_h - 8.0),
                    );
                    painter.rect_filled(m, 3.0, color.gamma_multiply(0.25));
                    painter.text(
                        m.center(),
                        Align2::CENTER_CENTER,
                        letter,
                        FontId::proportional(font.size * 0.75),
                        color,
                    );
                    // The label, with the matched characters in the accent.
                    let x = m.max.x + pad;
                    let matched = complete::match_with_positions(&p.prefix, &label)
                        .map(|(_, at)| at)
                        .unwrap_or_default();
                    let mut job = egui::text::LayoutJob::default();
                    for (i, c) in label.chars().enumerate() {
                        let color = if matched.contains(&i) {
                            accent
                        } else {
                            visuals.text_color()
                        };
                        job.append(
                            &c.to_string(),
                            0.0,
                            egui::TextFormat::simple(font.clone(), color),
                        );
                    }
                    let galley = ui.fonts_mut(|f| f.layout_job(job));
                    let ty = row_rect.center().y - galley.size().y / 2.0;
                    painter.galley(Pos2::new(x, ty), galley, visuals.text_color());
                    if let Some(detail) = &r.candidate.detail {
                        painter
                            .with_clip_rect(row_rect.shrink2(Vec2::new(pad, 0.0)))
                            .text(
                                Pos2::new(row_rect.max.x - pad, row_rect.center().y),
                                Align2::RIGHT_CENTER,
                                detail,
                                font.clone(),
                                visuals.weak_text_color(),
                            );
                    }
                }
                if p.items.len() > ROWS {
                    // A thin bar: where the rows shown are in the list.
                    let h = rect.height() * ROWS as f32 / p.items.len() as f32;
                    let t = p.scroll as f32 / (p.items.len() - ROWS) as f32;
                    let bar = Rect::from_min_size(
                        Pos2::new(rect.max.x - 4.0, rect.min.y + (rect.height() - h) * t),
                        Vec2::new(3.0, h),
                    );
                    painter.rect_filled(bar, 1.5, visuals.widgets.hovered.bg_fill);
                }
            });
        if scroll_by != 0 {
            let max = p.items.len().saturating_sub(ROWS) as i64;
            p.scroll = (p.scroll as i64 + scroll_by).clamp(0, max) as usize;
        }
        if let Some(i) = clicked {
            p.selected = i;
            self.clicked = Some(i);
        }
    }
}

impl Popup {
    /// Scrolls so the selected row is shown.
    fn reveal(&mut self) {
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + ROWS {
            self.scroll = self.selected + 1 - ROWS;
        }
    }
}

/// A kind's marker: a letter and its colour.
fn marker(kind: Kind, visuals: &egui::Visuals) -> (&'static str, Color32) {
    let dark = visuals.dark_mode;
    let pick = |d: (u8, u8, u8), l: (u8, u8, u8)| {
        let (r, g, b) = if dark { d } else { l };
        Color32::from_rgb(r, g, b)
    };
    match kind {
        Kind::Keyword => ("k", crate::theme::accent(visuals)),
        Kind::Function => ("f", pick((0x92, 0xBE, 0xEA), (0x2F, 0x64, 0xA8))),
        Kind::Type => ("T", pick((0x91, 0xD9, 0xCE), (0x1E, 0x7A, 0x70))),
        Kind::Module => ("m", pick((0xDC, 0xA8, 0xE0), (0x8E, 0x3E, 0x96))),
        Kind::Variable | Kind::Field => ("v", pick((0xBF, 0xD8, 0x8A), (0x4E, 0x7A, 0x1E))),
        Kind::Constant => ("c", pick((0xF2, 0xCC, 0x6E), (0x9A, 0x6A, 0x00))),
        Kind::Snippet => ("s", visuals.text_color()),
        Kind::Path => ("/", visuals.text_color()),
        Kind::Word | Kind::Other => ("w", visuals.weak_text_color()),
    }
}
