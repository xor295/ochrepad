//! A language server's completions in the popup (PLAN_completion L2): the
//! request for the word being typed, its answer kept for that word while
//! typing narrows it (unless the server said the list was incomplete), what
//! accepting an item changes (the server's range, its extra edits such as
//! an import), and the selected item's documentation
//! (`completionItem/resolve`).

use std::time::Duration;

use ochrepad_core::complete::{Candidate, Kind, Source};
use ochrepad_core::{Action, BufferId, ByteOffset, Document, TextEdit};
use ochrepad_lsp::Tag;
use serde_json::{Value, json};

use crate::completion_popup::{Accepting, CompletionRequest};
use crate::lsp::LanguageServers;

/// How long a completion request may take.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Items kept from one answer.
const MAX_ITEMS: usize = 5000;
/// Server items come before every built-in candidate (a member separator's
/// words are tier 1).
const TIER: u8 = 2;

#[derive(Default)]
pub struct ServerCompletion {
    pending: Option<Pending>,
    list: Option<List>,
    /// Answers taken: the popup asks again when this changes.
    pub answered: u64,
    /// The resolve in flight: the list it is for, the item, tag and id.
    resolving: Option<(u64, usize, Tag, i64)>,
}

/// A request in flight, for the word starting at `start`.
struct Pending {
    buffer: BufferId,
    start: ByteOffset,
    prefix: String,
    tag: Tag,
    id: i64,
}

/// An answer, for the word starting at `start` as typed up to `prefix`.
struct List {
    /// Tells lists apart (a resolve's answer is for one).
    number: u64,
    buffer: BufferId,
    start: ByteOffset,
    prefix: String,
    /// More items may match a longer prefix: ask again as typing goes on.
    incomplete: bool,
    items: Vec<Item>,
}

struct Item {
    label: String,
    filter: Option<String>,
    kind: Kind,
    detail: Option<String>,
    /// Markdown.
    documentation: Option<String>,
    /// What accepting inserts.
    text: String,
    /// The range it replaces, as the server sent it.
    range: Option<Value>,
    /// Edits elsewhere (an import).
    extra: Vec<Value>,
    /// The server's order.
    sort: String,
    /// As sent, for `completionItem/resolve`.
    raw: Value,
    resolved: bool,
}

impl ServerCompletion {
    /// Takes the answers that came (once a frame, before the popup asks).
    pub fn poll(&mut self, servers: &mut LanguageServers) {
        if let Some(p) = &self.pending
            && let Some(answer) = servers.take_answer(p.tag)
        {
            let p = self.pending.take().expect("pending");
            let number = self.list.as_ref().map_or(1, |l| l.number + 1);
            let (incomplete, items) = match answer {
                Ok(v) => parse_list(&v),
                // Asked again as typing goes on.
                Err(_) => (true, Vec::new()),
            };
            self.list = Some(List {
                number,
                buffer: p.buffer,
                start: p.start,
                prefix: p.prefix,
                incomplete,
                items,
            });
            self.answered += 1;
        }
        if let Some((number, index, tag, _)) = self.resolving
            && let Some(answer) = servers.take_answer(tag)
        {
            self.resolving = None;
            if let Some(item) = self
                .list
                .as_mut()
                .filter(|l| l.number == number)
                .and_then(|l| l.items.get_mut(index))
            {
                item.resolved = true;
                if let Ok(v) = answer {
                    if let Some(d) = documentation(&v) {
                        item.documentation = Some(d);
                    }
                    if let Some(d) = v.get("detail").and_then(Value::as_str) {
                        item.detail = Some(d.to_owned());
                    }
                    if let Some(Value::Array(e)) = v.get("additionalTextEdits")
                        && item.extra.is_empty()
                    {
                        item.extra = e.clone();
                    }
                }
            }
        }
    }

    /// Whether a request is in flight.
    pub fn waiting(&self) -> bool {
        self.pending.is_some()
    }

    /// The server's candidates for `req` (asking it when its last answer
    /// does not cover them); empty when no server completes there.
    pub fn candidates(
        &mut self,
        req: &CompletionRequest<'_>,
        servers: &mut LanguageServers,
    ) -> Vec<Candidate> {
        let same_word = |buffer: BufferId, start: ByteOffset, prefix: &str| {
            buffer == req.buffer && start == req.start && req.prefix.starts_with(prefix)
        };
        let list = self
            .list
            .as_ref()
            .filter(|l| same_word(l.buffer, l.start, &l.prefix));
        let covered = list.is_some_and(|l| !l.incomplete || l.prefix == req.prefix);
        let asked = self
            .pending
            .as_ref()
            .is_some_and(|p| same_word(p.buffer, p.start, &p.prefix));
        if !covered && !asked {
            let incomplete = list.is_some();
            self.ask(req, incomplete, servers);
        }
        let Some(list) = self
            .list
            .as_ref()
            .filter(|l| same_word(l.buffer, l.start, &l.prefix))
        else {
            return Vec::new();
        };
        let n = list.items.len().max(1) as f32;
        let mut order: Vec<usize> = (0..list.items.len()).collect();
        order.sort_by(|&a, &b| list.items[a].sort.cmp(&list.items[b].sort));
        let mut out = Vec::with_capacity(list.items.len());
        for (place, &i) in order.iter().enumerate() {
            let item = &list.items[i];
            let mut c = Candidate::new(item.label.clone(), item.kind, Source::Server);
            c.detail = item.detail.as_ref().map(|d| first_line(d));
            c.filter = item.filter.clone();
            if item.text != item.label {
                c.insert = Some(item.text.clone());
            }
            // The server's order counts for something.
            c.boost = 0.5 * (1.0 - place as f32 / n);
            c.tier = TIER;
            c.key = Some(i as u32);
            out.push(c);
        }
        out
    }

    fn ask(
        &mut self,
        req: &CompletionRequest<'_>,
        incomplete: bool,
        servers: &mut LanguageServers,
    ) {
        if let Some(p) = self.pending.take() {
            servers.cancel(p.buffer, p.id);
            let _ = servers.take_answer(p.tag);
        }
        // The character before the word (a trigger character, `.`).
        let line_start = req.doc.line_start(req.doc.line_of(req.start));
        let before: String = req
            .doc
            .slice(line_start..req.start)
            .chars()
            .next_back()
            .map(String::from)
            .unwrap_or_default();
        let sent = servers.request(
            req.buffer,
            req.doc,
            "completionProvider",
            "textDocument/completion",
            |c, uri| {
                let triggers = c
                    .capabilities
                    .pointer("/completionProvider/triggerCharacters")
                    .and_then(Value::as_array);
                let trigger = req.prefix.is_empty()
                    && triggers.is_some_and(|t| t.iter().any(|t| t.as_str() == Some(&before)));
                let context = if incomplete {
                    json!({ "triggerKind": 3 })
                } else if trigger {
                    json!({ "triggerKind": 2, "triggerCharacter": before })
                } else {
                    json!({ "triggerKind": 1 })
                };
                json!({
                    "textDocument": { "uri": uri },
                    "position": c.position(req.doc, req.caret),
                    "context": context,
                })
            },
            TIMEOUT,
        );
        if let Some((tag, id)) = sent {
            self.pending = Some(Pending {
                buffer: req.buffer,
                start: req.start,
                prefix: req.prefix.to_owned(),
                tag,
                id,
            });
        }
    }

    fn item(&self, buffer: BufferId, candidate: &Candidate) -> Option<(u64, usize, &Item)> {
        if candidate.source != Source::Server {
            return None;
        }
        let list = self.list.as_ref().filter(|l| l.buffer == buffer)?;
        let i = candidate.key? as usize;
        let item = list.items.get(i).filter(|it| it.label == candidate.label)?;
        Some((list.number, i, item))
    }

    /// Markdown about `candidate` (its signature and documentation), asking
    /// the server for it when the item came without.
    pub fn documentation(
        &mut self,
        buffer: BufferId,
        doc: &Document,
        candidate: &Candidate,
        servers: &mut LanguageServers,
    ) -> Option<String> {
        let (number, i, item) = self.item(buffer, candidate)?;
        let shown = describe(item);
        let wanted = !item.resolved
            && (item.documentation.is_none() || item.detail.is_none())
            && self
                .resolving
                .is_none_or(|(n, r, _, _)| n != number || r != i);
        if wanted {
            let raw = item.raw.clone();
            if let Some((_, _, tag, id)) = self.resolving.take() {
                servers.cancel(buffer, id);
                let _ = servers.take_answer(tag);
            }
            let resolvable = servers.session_of(buffer).is_some_and(|s| {
                s.client
                    .capabilities
                    .pointer("/completionProvider/resolveProvider")
                    .and_then(Value::as_bool)
                    == Some(true)
            });
            let sent = resolvable
                .then(|| {
                    servers.request(
                        buffer,
                        doc,
                        "completionProvider",
                        "completionItem/resolve",
                        |_, _| raw,
                        TIMEOUT,
                    )
                })
                .flatten();
            match sent {
                Some((tag, id)) => self.resolving = Some((number, i, tag, id)),
                None => {
                    if let Some(item) = self.list.as_mut().and_then(|l| l.items.get_mut(i)) {
                        item.resolved = true;
                    }
                }
            }
        }
        shown
    }

    /// The action accepting a server's item: from the start of the range
    /// it names (when that is on the caret's line, before it), and with its
    /// extra edits when there is one caret.
    pub fn accept_action(
        &self,
        a: &Accepting<'_>,
        servers: &LanguageServers,
        complete: Action,
    ) -> Action {
        let Some((_, _, item)) = self.item(a.buffer, a.candidate) else {
            return complete;
        };
        let Action::Complete { before, after, .. } = complete else {
            return complete;
        };
        let doc = a.doc;
        let line_start = doc.line_start(doc.line_of(a.caret));
        let start = item
            .range
            .as_ref()
            .and_then(|r| servers.range_in(a.buffer, doc, r))
            .map(|r| r.start)
            .filter(|s| *s >= line_start && *s <= a.caret);
        let before = match start {
            Some(s) => doc.slice(s..a.caret).chars().count(),
            None => before,
        };
        let complete = Action::Complete {
            before,
            after,
            text: item.text.clone(),
        };
        if item.extra.is_empty() || a.carets != 1 {
            return complete;
        }
        // One step: the completion and its extra edits.
        let start = char_back(doc, a.caret, before);
        let end = char_forward(doc, a.caret, after);
        let mut edits = vec![TextEdit {
            start,
            end,
            text: item.text.clone(),
        }];
        for e in &item.extra {
            let (Some(r), Some(text)) = (
                e.get("range")
                    .and_then(|r| servers.range_in(a.buffer, doc, r)),
                e.get("newText").and_then(Value::as_str),
            ) else {
                return complete;
            };
            // Never over the completion itself.
            if r.start < end && start < r.end {
                return complete;
            }
            edits.push(TextEdit {
                start: r.start,
                end: r.end,
                text: text.to_owned(),
            });
        }
        Action::Edits(edits)
    }
}

/// `n` characters before `at` (not before its line).
fn char_back(doc: &Document, at: ByteOffset, n: usize) -> ByteOffset {
    let line_start = doc.line_start(doc.line_of(at));
    let text = doc.slice(line_start..at);
    let back: usize = text.chars().rev().take(n).map(char::len_utf8).sum();
    ByteOffset(at.0 - back)
}

/// `n` characters after `at` (not past its line).
fn char_forward(doc: &Document, at: ByteOffset, n: usize) -> ByteOffset {
    let line_end = doc.line_end(doc.line_of(at));
    let text = doc.slice(at..line_end);
    let ahead: usize = text.chars().take(n).map(char::len_utf8).sum();
    ByteOffset(at.0 + ahead)
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_owned()
}

/// What the side panel shows for an item: its detail as code, then its
/// documentation.
fn describe(item: &Item) -> Option<String> {
    let detail = item
        .detail
        .as_deref()
        .filter(|d| !d.trim().is_empty())
        .map(|d| format!("```\n{d}\n```"));
    let doc = item
        .documentation
        .as_deref()
        .filter(|d| !d.trim().is_empty());
    match (detail, doc) {
        (Some(a), Some(b)) => Some(format!("{a}\n\n{b}")),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b.to_owned()),
        (None, None) => None,
    }
}

/// An item's documentation as Markdown (a string, or `{kind, value}`).
fn documentation(v: &Value) -> Option<String> {
    match v.get("documentation")? {
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => {
            let value = o.get("value")?.as_str()?;
            Some(match o.get("kind").and_then(Value::as_str) {
                Some("markdown") => value.to_owned(),
                _ => format!("```text\n{value}\n```"),
            })
        }
        _ => None,
    }
}

/// A completion answer: a list of items, or `{isIncomplete, items}`.
fn parse_list(v: &Value) -> (bool, Vec<Item>) {
    let (incomplete, items) = match v {
        Value::Array(items) => (false, items.as_slice()),
        Value::Object(o) => (
            o.get("isIncomplete").and_then(Value::as_bool) == Some(true),
            o.get("items")
                .and_then(Value::as_array)
                .map_or(&[][..], Vec::as_slice),
        ),
        _ => (false, &[][..]),
    };
    (
        incomplete || items.len() > MAX_ITEMS,
        items
            .iter()
            .take(MAX_ITEMS)
            .filter_map(parse_item)
            .collect(),
    )
}

fn parse_item(v: &Value) -> Option<Item> {
    let label = v.get("label")?.as_str()?.to_owned();
    if label.is_empty() || label.contains('\n') {
        return None;
    }
    let str_of = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    let edit = v.get("textEdit");
    // A text edit, or an insert/replace edit (its insert range).
    let range = edit.and_then(|e| e.get("range").or_else(|| e.get("insert")).cloned());
    let raw_text = edit
        .and_then(|e| e.get("newText"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| str_of("insertText"))
        .unwrap_or_else(|| label.clone());
    let text = if v.get("insertTextFormat").and_then(Value::as_i64) == Some(2) {
        plain_snippet(&raw_text)
    } else {
        raw_text
    };
    // Shown after the label: its details (`(as Clone)`), or what it is.
    let label_details = v.get("labelDetails");
    let detail = str_of("detail").or_else(|| {
        let d = label_details?;
        let parts: Vec<&str> = ["detail", "description"]
            .iter()
            .filter_map(|k| d.get(*k)?.as_str())
            .filter(|s| !s.is_empty())
            .collect();
        (!parts.is_empty()).then(|| parts.join(" "))
    });
    let kind = match v.get("kind").and_then(Value::as_i64) {
        Some(2..=4) => Kind::Function,
        Some(5 | 10) => Kind::Field,
        Some(6) => Kind::Variable,
        Some(7 | 8 | 13 | 22 | 25) => Kind::Type,
        Some(9) => Kind::Module,
        Some(14) => Kind::Keyword,
        Some(15) => Kind::Snippet,
        Some(17 | 19) => Kind::Path,
        Some(20 | 21) => Kind::Constant,
        _ => Kind::Other,
    };
    let filter = str_of("filterText").filter(|f| *f != label);
    Some(Item {
        sort: str_of("sortText").unwrap_or_else(|| label.clone()),
        documentation: documentation(v),
        extra: v
            .get("additionalTextEdits")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        resolved: false,
        raw: v.clone(),
        label,
        filter,
        kind,
        detail,
        text,
        range,
    })
}

/// Snippet syntax as plain text: placeholders keep their text (a choice
/// its first option), tab stops and variables go, `\` escapes.
fn plain_snippet(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    // Placeholders open, and whether each is a choice still in its first
    // option (`Some(true)`), past it (`Some(false)`), or not one.
    let mut open: Vec<Option<bool>> = Vec::new();
    let keep = |open: &[Option<bool>]| open.iter().all(|o| *o != Some(false));
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next()
                    && keep(&open)
                {
                    out.push(n);
                }
            }
            '$' if chars.peek() == Some(&'{') => {
                chars.next();
                while chars
                    .peek()
                    .is_some_and(|c| c.is_alphanumeric() || *c == '_')
                {
                    chars.next();
                }
                match chars.next() {
                    Some(':') => open.push(None),
                    Some('|') => open.push(Some(true)),
                    // `${1}`
                    _ => {}
                }
            }
            '$' if chars
                .peek()
                .is_some_and(|c| c.is_alphanumeric() || *c == '_') =>
            {
                while chars
                    .peek()
                    .is_some_and(|c| c.is_alphanumeric() || *c == '_')
                {
                    chars.next();
                }
            }
            ',' if open.last() == Some(&Some(true)) => {
                if let Some(o) = open.last_mut() {
                    *o = Some(false);
                }
            }
            '|' if matches!(open.last(), Some(Some(_))) && chars.peek() == Some(&'}') => {
                chars.next();
                open.pop();
            }
            '}' if !open.is_empty() => {
                open.pop();
            }
            c => {
                if keep(&open) {
                    out.push(c);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippets_as_plain_text() {
        assert_eq!(plain_snippet("push(${1:value})$0"), "push(value)");
        assert_eq!(plain_snippet("f($1, ${2:b})"), "f(, b)");
        assert_eq!(plain_snippet("${1|one,two|} x"), "one x");
        assert_eq!(plain_snippet("a \\$1 \\} b"), "a $1 } b");
        assert_eq!(plain_snippet("${1:outer ${2:inner}}"), "outer inner");
        assert_eq!(plain_snippet("$TM_FILENAME!"), "!");
    }

    #[test]
    fn items_from_json() {
        let (incomplete, items) = parse_list(&json!({
            "isIncomplete": true,
            "items": [
                {"label": "push(…)", "kind": 2, "filterText": "push", "insertText": "push",
                 "labelDetails": {"description": "fn(&mut self, T)"}, "sortText": "b"},
                {"label": "HashMap", "kind": 22,
                 "textEdit": {"range": {"start": {"line": 0, "character": 0},
                                        "end": {"line": 0, "character": 2}},
                              "newText": "HashMap"},
                 "additionalTextEdits": [{"range": {"start": {"line": 0, "character": 0},
                                                    "end": {"line": 0, "character": 0}},
                                          "newText": "use std::collections::HashMap;\n"}],
                 "documentation": {"kind": "markdown", "value": "A **map**."}},
                {"label": "bad\nlabel"},
                {"nolabel": 1}
            ]
        }));
        assert!(incomplete);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].filter.as_deref(), Some("push"));
        assert_eq!(items[0].kind, Kind::Function);
        assert_eq!(items[0].detail.as_deref(), Some("fn(&mut self, T)"));
        assert_eq!(items[1].kind, Kind::Type);
        assert_eq!(items[1].extra.len(), 1);
        assert!(items[1].range.is_some());
        assert_eq!(items[1].documentation.as_deref(), Some("A **map**."));
        let (incomplete, items) = parse_list(&json!([{"label": "x"}]));
        assert!(!incomplete);
        assert_eq!(items[0].text, "x");
    }
}
