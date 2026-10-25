//! Signature help (PLAN_completion L2): while typing a call's arguments,
//! the function's signature above the caret, with the argument being typed
//! picked out and its documentation. Asked for when the server's trigger
//! characters are typed (`(`, `,`) or with Ctrl+Shift+Space, and asked
//! again as the caret moves while it shows; the server says when the caret
//! has left the call (no signatures).

use std::ops::Range;
use std::time::Duration;

use egui::text::LayoutJob;
use egui::{Align2, FontId, Id, Pos2, Rect, Response, TextFormat};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use ochrepad_core::position::ColumnUnits;
use ochrepad_core::{BufferId, ByteOffset, Document, ViewId};
use ochrepad_lsp::Tag;
use serde_json::{Value, json};

use crate::lsp::LanguageServers;

const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Default)]
pub struct SignatureHelp {
    /// The request in flight: its view, tag and id.
    pending: Option<(ViewId, Tag, i64)>,
    shown: Option<(ViewId, Help)>,
    /// The view's revision and caret last frame (a change asks again).
    last: Option<(ViewId, u64, ByteOffset)>,
    cache: CommonMarkCache,
}

#[derive(Clone, Debug, PartialEq)]
struct Help {
    signatures: Vec<Signature>,
    active: usize,
    /// As sent, for asking again (`activeSignatureHelp`).
    raw: Value,
}

#[derive(Clone, Debug, PartialEq)]
struct Signature {
    label: String,
    /// The parameters' byte ranges in `label`.
    params: Vec<Range<usize>>,
    active_param: Option<usize>,
    /// Markdown: the active parameter's documentation, then the
    /// signature's.
    docs: Vec<String>,
}

/// What the editor did this frame.
pub struct Frame<'a> {
    pub view: ViewId,
    pub buffer: BufferId,
    pub doc: &'a Document,
    pub revision: u64,
    /// The primary caret, unless there is a selection.
    pub caret: Option<ByteOffset>,
    /// Its cell on screen, when drawn.
    pub caret_rect: Option<Rect>,
    /// Ctrl+Shift+Space.
    pub invoked: bool,
    pub focused: bool,
}

impl SignatureHelp {
    /// After `f.view`'s editor is drawn.
    pub fn show(&mut self, response: &Response, f: &Frame<'_>, servers: &mut LanguageServers) {
        let view = f.view;
        if let Some((v, tag, _)) = self.pending
            && v == view
            && let Some(answer) = servers.take_answer(tag)
        {
            self.pending = None;
            let units = servers
                .session_of(f.buffer)
                .map_or(ColumnUnits::Utf16, |s| s.client.units);
            self.shown = answer
                .ok()
                .and_then(|v| parse(&v, units))
                .map(|h| (view, h));
        }
        let shown_here = self.shown.as_ref().is_some_and(|(v, _)| *v == view);
        let escape = f.focused && response.ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let Some(caret) = f.caret.filter(|_| f.focused && !escape) else {
            if shown_here || self.pending.is_some_and(|(v, _, _)| v == view) {
                self.close(f.buffer, servers);
            }
            return;
        };
        let last = self.last.replace((view, f.revision, caret));
        let typed = last.is_some_and(|(v, r, _)| v == view && r != f.revision);
        let moved = last.is_some_and(|(v, r, c)| v == view && (r != f.revision || c != caret));
        let trigger = typed
            .then(|| char_before(f.doc, caret))
            .flatten()
            .filter(|c| triggers(servers, f.buffer).contains(c));
        if f.invoked || trigger.is_some() || (shown_here && moved) {
            let context = match (&self.shown, trigger) {
                (Some((v, h)), _) if *v == view && !f.invoked => json!({
                    "triggerKind": if trigger.is_some() { 2 } else { 3 },
                    "triggerCharacter": trigger.map(String::from),
                    "isRetrigger": true,
                    "activeSignatureHelp": h.raw,
                }),
                (_, Some(c)) if !f.invoked => json!({
                    "triggerKind": 2, "triggerCharacter": c.to_string(), "isRetrigger": false,
                }),
                _ => json!({ "triggerKind": 1, "isRetrigger": shown_here }),
            };
            if let Some((_, tag, id)) = self.pending.take() {
                servers.cancel(f.buffer, id);
                let _ = servers.take_answer(tag);
            }
            self.pending = servers
                .request(
                    f.buffer,
                    f.doc,
                    "signatureHelpProvider",
                    "textDocument/signatureHelp",
                    |c, uri| {
                        json!({
                            "textDocument": { "uri": uri },
                            "position": c.position(f.doc, caret),
                            "context": context,
                        })
                    },
                    TIMEOUT,
                )
                .map(|(tag, id)| (view, tag, id));
        }
        if let (Some((v, help)), Some(rect)) = (&self.shown, f.caret_rect)
            && *v == view
        {
            paint(&response.ctx, view, help, rect, &mut self.cache);
        }
    }

    fn close(&mut self, buffer: BufferId, servers: &mut LanguageServers) {
        self.shown = None;
        if let Some((_, tag, id)) = self.pending.take() {
            servers.cancel(buffer, id);
            let _ = servers.take_answer(tag);
        }
    }

    /// Whether it shows for `view` (tests).
    pub fn label(&self, view: ViewId) -> Option<String> {
        let (v, h) = self.shown.as_ref()?;
        (*v == view).then(|| h.signatures.get(h.active).map(|s| s.label.clone()))?
    }
}

fn char_before(doc: &Document, at: ByteOffset) -> Option<char> {
    let line_start = doc.line_start(doc.line_of(at));
    doc.slice(line_start..at).chars().next_back()
}

/// The characters after which the server's signature help is asked for.
fn triggers(servers: &LanguageServers, buffer: BufferId) -> Vec<char> {
    let Some(s) = servers.session_of(buffer) else {
        return Vec::new();
    };
    let caps = &s.client.capabilities;
    ["triggerCharacters", "retriggerCharacters"]
        .iter()
        .filter_map(|k| {
            caps.pointer(&format!("/signatureHelpProvider/{k}"))?
                .as_array()
        })
        .flatten()
        .filter_map(|c| c.as_str()?.chars().next())
        .collect()
}

/// A `SignatureHelp` answer (`None` when it has no signatures: the caret is
/// not in a call).
fn parse(v: &Value, units: ColumnUnits) -> Option<Help> {
    let sigs = v.get("signatures")?.as_array()?;
    let index = |k: &str, of: &Value| {
        of.get(k)
            .and_then(Value::as_u64)
            .map(|n| usize::try_from(n).unwrap_or(usize::MAX))
    };
    let help_param = index("activeParameter", v);
    let signatures: Vec<Signature> = sigs
        .iter()
        .filter_map(|s| {
            let label = s.get("label")?.as_str()?.to_owned();
            if label.len() > 4096 {
                return None;
            }
            let params: Vec<Range<usize>> = s
                .get("parameters")
                .and_then(Value::as_array)
                .map(|ps| {
                    ps.iter()
                        .map(|p| param_range(&label, p.get("label"), units).unwrap_or(0..0))
                        .collect()
                })
                .unwrap_or_default();
            let active_param = index("activeParameter", s)
                .or(help_param)
                .filter(|i| *i < params.len());
            let mut docs = Vec::new();
            if let Some(d) = active_param
                .and_then(|i| s.get("parameters")?.get(i)?.get("documentation"))
                .and_then(documentation)
            {
                docs.push(d);
            }
            if let Some(d) = s.get("documentation").and_then(documentation) {
                docs.push(d);
            }
            Some(Signature {
                label,
                params,
                active_param,
                docs,
            })
        })
        .collect();
    if signatures.is_empty() {
        return None;
    }
    let active = index("activeSignature", v)
        .filter(|i| *i < signatures.len())
        .unwrap_or(0);
    Some(Help {
        signatures,
        active,
        raw: v.clone(),
    })
}

/// A parameter's place in its signature's label: given as text (its first
/// occurrence after the opening bracket) or as offsets in the server's
/// units.
fn param_range(label: &str, p: Option<&Value>, units: ColumnUnits) -> Option<Range<usize>> {
    match p? {
        Value::String(s) if !s.is_empty() => {
            let from = label.find('(').map_or(0, |i| i + 1);
            let at = label[from..].find(s.as_str())? + from;
            Some(at..at + s.len())
        }
        Value::Array(a) if a.len() == 2 => {
            let n = |i: usize| a[i].as_u64().map(|n| n as usize);
            let (start, end) = (n(0)?, n(1)?);
            let byte = |n: usize| match units {
                ColumnUnits::Utf8 => (n <= label.len() && label.is_char_boundary(n)).then_some(n),
                ColumnUnits::Utf16 => {
                    let mut count = 0;
                    for (i, c) in label.char_indices() {
                        if count >= n {
                            return (count == n).then_some(i);
                        }
                        count += c.len_utf16();
                    }
                    (count == n).then_some(label.len())
                }
                ColumnUnits::Utf32 => match label.char_indices().nth(n) {
                    Some((i, _)) => Some(i),
                    None => (label.chars().count() == n).then_some(label.len()),
                },
            };
            let (a, b) = (byte(start)?, byte(end)?);
            (a <= b).then_some(a..b)
        }
        _ => None,
    }
}

fn documentation(v: &Value) -> Option<String> {
    let text = match v {
        Value::String(s) => s.clone(),
        Value::Object(o) => {
            let value = o.get("value")?.as_str()?;
            match o.get("kind").and_then(Value::as_str) {
                Some("markdown") => value.to_owned(),
                _ => format!("```text\n{value}\n```"),
            }
        }
        _ => return None,
    };
    (!text.trim().is_empty()).then_some(text)
}

/// Draws `help` above the caret's line (below it when there is no room).
fn paint(ctx: &egui::Context, view: ViewId, help: &Help, caret: Rect, cache: &mut CommonMarkCache) {
    let Some(sig) = help.signatures.get(help.active) else {
        return;
    };
    let style = ctx.global_style();
    let visuals = &style.visuals;
    let font = FontId::monospace(caret.height().max(8.0) * 0.8);
    let accent = crate::theme::accent(visuals);
    let mut job = LayoutJob::default();
    if help.signatures.len() > 1 {
        job.append(
            &format!("{}/{}  ", help.active + 1, help.signatures.len()),
            0.0,
            TextFormat::simple(font.clone(), visuals.weak_text_color()),
        );
    }
    let active = sig
        .active_param
        .and_then(|i| sig.params.get(i))
        .filter(|r| !r.is_empty())
        .cloned();
    let mut at = 0;
    let mut add = |text: &str, strong: bool| {
        let mut format = TextFormat::simple(
            font.clone(),
            if strong { accent } else { visuals.text_color() },
        );
        if strong {
            format.underline = egui::Stroke::new(1.0, accent);
        }
        job.append(text, 0.0, format);
    };
    if let Some(r) = &active {
        add(&sig.label[at..r.start], false);
        add(&sig.label[r.clone()], true);
        at = r.end;
    }
    add(&sig.label[at..], false);
    job.wrap.max_width = 640.0;
    let screen = ctx.content_rect();
    let id = Id::new(("ochrepad-signature", view.0));
    // How tall it was last time, to place it above the caret.
    let height: f32 = ctx
        .data(|d| d.get_temp(id))
        .unwrap_or(caret.height() + 16.0);
    let mut pos = Pos2::new(caret.min.x - 8.0, caret.min.y - height - 4.0);
    if pos.y < screen.min.y {
        pos.y = caret.max.y + 4.0;
    }
    pos.x = pos.x.min(screen.max.x - 660.0).max(screen.min.x);
    let shown = egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .pivot(Align2::LEFT_TOP)
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(640.0);
                ui.add(egui::Label::new(job).selectable(false));
                if !sig.docs.is_empty() {
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(160.0)
                        .show(ui, |ui| {
                            crate::safe_images::install(ui);
                            for d in &sig.docs {
                                CommonMarkViewer::new().show(ui, cache, d);
                            }
                        });
                }
            });
        });
    let h = shown.response.rect.height();
    if (h - height).abs() > 0.5 {
        ctx.data_mut(|d| d.insert_temp(id, h));
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_from_json() {
        let h = parse(
            &json!({
                "signatures": [{
                    "label": "fn push(&mut self, value: T)",
                    "parameters": [{"label": [8, 17]}, {"label": "value: T"}],
                    "documentation": {"kind": "markdown", "value": "Appends."},
                }],
                "activeSignature": 0,
                "activeParameter": 1,
            }),
            ColumnUnits::Utf16,
        )
        .unwrap();
        let s = &h.signatures[0];
        assert_eq!(&s.label[s.params[0].clone()], "&mut self");
        assert_eq!(&s.label[s.params[1].clone()], "value: T");
        assert_eq!(s.active_param, Some(1));
        assert_eq!(s.docs, ["Appends."]);
        assert!(parse(&json!({"signatures": []}), ColumnUnits::Utf16).is_none());
        assert!(parse(&Value::Null, ColumnUnits::Utf16).is_none());
    }

    #[test]
    fn parameter_offsets_count_in_the_servers_units() {
        let label = "f(é: u8, x)";
        // `x` starts at UTF-16 unit 9, byte 10.
        assert_eq!(
            param_range(label, Some(&json!([9, 10])), ColumnUnits::Utf16),
            Some(10..11)
        );
        assert_eq!(
            param_range(label, Some(&json!([10, 11])), ColumnUnits::Utf8),
            Some(10..11)
        );
        // Off a character boundary, or past the end: none.
        assert_eq!(
            param_range(label, Some(&json!([3, 4])), ColumnUnits::Utf8),
            None
        );
        assert_eq!(
            param_range(label, Some(&json!([2, 99])), ColumnUnits::Utf16),
            None
        );
    }
}
