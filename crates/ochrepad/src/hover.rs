//! What the pointer rests on in an editor (PLAN_completion L2): the
//! diagnostics there and the language server's hover text, in a tooltip.
//! The server is asked once the pointer has rested a moment; its Markdown
//! is drawn with the preview's renderer, behind the same image guards.

use std::ops::Range;
use std::time::Duration;

use egui::{Response, RichText};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use ochrepad_core::{BufferId, ByteOffset, Document, ViewId};
use serde_json::{Value, json};

use crate::lsp::{LanguageServers, Severity};

/// How long the pointer rests before the server is asked.
const REST_SECS: f64 = 0.4;

#[derive(Default)]
pub struct Hover {
    /// The request in flight: its view and offset, tag and id.
    pending: Option<(ViewId, ByteOffset, ochrepad_lsp::Tag, i64)>,
    /// The answer: its view, the range it is about, and its Markdown
    /// (empty when the server had nothing).
    shown: Option<(ViewId, Range<ByteOffset>, String)>,
    cache: CommonMarkCache,
}

impl Hover {
    /// After `view`'s editor is drawn: asks the server about the text the
    /// pointer rests on, and shows what there is in a tooltip.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        response: Response,
        view: ViewId,
        buffer: BufferId,
        doc: &Document,
        rest: Option<(ByteOffset, f64)>,
        now: f64,
        servers: &mut LanguageServers,
    ) -> Response {
        let Some((at, since)) = rest else {
            self.forget(view, buffer, servers);
            return response;
        };
        // Still about the same name.
        if self
            .shown
            .as_ref()
            .is_some_and(|(v, r, _)| *v == view && !r.contains(&at) && r.end != at)
        {
            self.shown = None;
        }
        if let Some((v, a, tag, _)) = self.pending
            && v == view
        {
            if a != at {
                self.forget(view, buffer, servers);
            } else if let Some(answer) = servers.take_answer(tag) {
                self.pending = None;
                let value = answer.unwrap_or(Value::Null);
                let range = value
                    .get("range")
                    .and_then(|r| servers.range_in(buffer, doc, r))
                    .unwrap_or(at..at);
                self.shown = Some((view, range, markdown(value.get("contents"))));
            }
        }
        let rested = now - since >= REST_SECS;
        let have = self.shown.as_ref().is_some_and(|(v, _, _)| *v == view);
        if rested && !have && self.pending.is_none() {
            self.pending = servers
                .request(
                    buffer,
                    doc,
                    "hoverProvider",
                    "textDocument/hover",
                    |c, uri| {
                        json!({
                            "textDocument": { "uri": uri },
                            "position": c.position(doc, at),
                        })
                    },
                    Duration::from_secs(10),
                )
                .map(|(tag, id)| (view, at, tag, id));
        }
        let problems: Vec<(Severity, String)> = servers
            .diagnostics_of(buffer)
            .iter()
            .filter(|d| {
                d.range.as_ref().is_some_and(|r| {
                    r.start <= at && (at < r.end || (r.start == r.end && at == r.start))
                })
            })
            .map(|d| {
                let source = d
                    .source
                    .as_deref()
                    .map(|s| format!(" ({s})"))
                    .unwrap_or_default();
                (d.severity, format!("{}{source}", d.message))
            })
            .collect();
        let text = self
            .shown
            .as_ref()
            .filter(|(v, _, t)| *v == view && !t.trim().is_empty())
            .map(|(_, _, t)| t.clone());
        if !rested || (problems.is_empty() && text.is_none()) {
            return response;
        }
        let cache = &mut self.cache;
        response.on_hover_ui_at_pointer(|ui| {
            ui.set_max_width(560.0);
            for (severity, message) in &problems {
                let color = crate::theme::diagnostic_color(ui.visuals(), *severity as u8);
                ui.label(RichText::new(message).color(color));
            }
            if let Some(text) = &text {
                if !problems.is_empty() {
                    ui.separator();
                }
                crate::safe_images::install(ui);
                CommonMarkViewer::new().show(ui, cache, text);
            }
        })
    }

    /// The pointer left: a pending request is no longer wanted.
    fn forget(&mut self, view: ViewId, buffer: BufferId, servers: &mut LanguageServers) {
        if let Some((v, _, tag, id)) = self.pending
            && v == view
        {
            servers.cancel(buffer, id);
            let _ = servers.take_answer(tag);
            self.pending = None;
        }
        if self.shown.as_ref().is_some_and(|(v, _, _)| *v == view) {
            self.shown = None;
        }
    }
}

/// A hover's contents as Markdown: a string, `{language, value}`, a list
/// of those, or `{kind, value}`.
fn markdown(contents: Option<&Value>) -> String {
    match contents {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|i| markdown(Some(i)))
            .filter(|s| !s.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n---\n\n"),
        Some(Value::Object(o)) => {
            let value = o.get("value").and_then(Value::as_str).unwrap_or("");
            match (
                o.get("language").and_then(Value::as_str),
                o.get("kind").and_then(Value::as_str),
            ) {
                (Some(lang), _) => format!("```{lang}\n{value}\n```"),
                (None, Some("plaintext")) => format!("```text\n{value}\n```"),
                _ => value.to_owned(),
            }
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_contents_as_markdown() {
        assert_eq!(markdown(Some(&json!("**x**"))), "**x**");
        assert_eq!(
            markdown(Some(&json!({"language": "rust", "value": "fn f()"}))),
            "```rust\nfn f()\n```"
        );
        assert_eq!(
            markdown(Some(&json!({"kind": "markdown", "value": "doc"}))),
            "doc"
        );
        assert_eq!(
            markdown(Some(&json!(["a", {"language": "c", "value": "int x"}]))),
            "a\n\n---\n\n```c\nint x\n```"
        );
        assert_eq!(markdown(None), "");
    }
}
