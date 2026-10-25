//! One server's session: starting it, the documents it was given, the
//! app's requests and their answers, and what the server asks of us. The
//! app calls [`Client::poll`] each frame; nothing here waits.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ochrepad_core::position::{ColumnUnits, from_lsp_position, to_lsp_position};
use ochrepad_core::{ByteOffset, Document};
use serde_json::{Value, json};

use crate::config::ServerConfig;
use crate::message::{Id, Message, ResponseError, codes};
use crate::transport::{Incoming, Launch, Notify, SharedLog, Transport, log};
use crate::uri;

/// How long a server has to answer `initialize`.
const START_TIMEOUT: Duration = Duration::from_secs(60);
/// How long `shutdown` may take before the server is told to exit anyway.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// Started; `initialize` not answered yet.
    Starting,
    Running,
    /// Asked to shut down.
    Stopping,
    /// Ended: why (it stopped as asked, crashed, or never started).
    Stopped {
        reason: String,
        asked: bool,
    },
}

/// What a request was for: the app's own number, handed back with the
/// answer.
pub type Tag = u64;

#[derive(Debug)]
pub enum Event {
    /// The server is ready (its capabilities are known).
    Started,
    /// A request's answer, or an error (`codes::TIMED_OUT` when it took too
    /// long).
    Answer {
        tag: Tag,
        result: Result<Value, ResponseError>,
    },
    /// `textDocument/publishDiagnostics`, as sent.
    Diagnostics {
        uri: String,
        version: Option<i64>,
        diagnostics: Vec<Value>,
    },
    /// `window/showMessage`, or a `showMessageRequest` (its buttons do
    /// nothing: the request is answered with none chosen).
    Message { kind: i64, text: String },
    /// `window/showDocument`: the app opens it if it is a local file.
    ShowDocument {
        uri: String,
        selection: Option<Value>,
    },
    /// `workspace/applyEdit`: answer with [`Client::answer_apply_edit`].
    ApplyEdit { id: Id, edit: Value },
    /// `$/progress`: what the server is busy with (`None` when done).
    Progress { token: String, text: Option<String> },
    /// The session ended.
    Stopped { reason: String, asked: bool },
}

struct Pending {
    tag: Tag,
    sent: Instant,
    timeout: Duration,
}

/// How the server wants documents' changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sync {
    None,
    Full,
    Incremental,
}

struct OpenDoc {
    version: i64,
    text: Document,
    language_id: String,
    /// `didOpen` was sent (it waits until the server is ready).
    sent: bool,
}

pub struct Client {
    pub name: String,
    pub root: PathBuf,
    root_uri: String,
    transport: Option<Transport>,
    pub log: SharedLog,
    pub state: State,
    started: Instant,
    next_id: i64,
    pending: HashMap<i64, Pending>,
    init_id: Option<i64>,
    shutdown: Option<(i64, Instant)>,
    /// The server's capabilities, as it sent them.
    pub capabilities: Value,
    pub units: ColumnUnits,
    sync: Sync,
    save_text: bool,
    docs: HashMap<String, OpenDoc>,
    settings: Value,
    events: Vec<Event>,
    /// `$/progress` titles by token, from each `begin`.
    progress_titles: HashMap<String, String>,
}

impl Client {
    /// Starts the server and sends `initialize`. A server that cannot be
    /// started gives a client that is already stopped.
    pub fn start(
        config: &ServerConfig,
        launch: &Launch,
        root: &Path,
        log: SharedLog,
        notify: Notify,
    ) -> Self {
        let root_uri = uri::file_uri(root).unwrap_or_default();
        let mut c = Self {
            name: config.name.clone(),
            root: root.to_path_buf(),
            root_uri,
            transport: None,
            log,
            state: State::Starting,
            started: Instant::now(),
            next_id: 1,
            pending: HashMap::new(),
            init_id: None,
            shutdown: None,
            capabilities: Value::Null,
            units: ColumnUnits::Utf16,
            sync: Sync::Full,
            save_text: false,
            docs: HashMap::new(),
            settings: config.settings.clone(),
            events: Vec::new(),
            progress_titles: HashMap::new(),
        };
        crate::transport::log(
            &c.log,
            &format!("[ochrepad] starting {launch:?} in {}", root.display()),
        );
        match Transport::start(launch, c.log.clone(), notify) {
            Ok(t) => c.transport = Some(t),
            Err(e) => {
                c.stop(format!("could not start: {e}"), false);
                return c;
            }
        }
        let id = c.take_id();
        c.init_id = Some(id);
        let params = initialize_params(&c.root, &c.root_uri, config);
        c.send(&Message::Request {
            id: Id::Number(id),
            method: "initialize".into(),
            params,
        });
        c
    }

    fn take_id(&mut self) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn send(&self, m: &Message) {
        if let Some(t) = &self.transport {
            t.send(m);
        }
    }

    pub fn is_running(&self) -> bool {
        self.state == State::Running
    }

    /// Whether the server says it can do `capability` (a key of its
    /// `ServerCapabilities`, such as `"hoverProvider"`).
    pub fn can(&self, capability: &str) -> bool {
        self.capabilities
            .get(capability)
            .is_some_and(|v| !v.is_null() && *v != Value::Bool(false))
    }

    /// Takes what the server sent since last time, and gives up on
    /// requests that took too long.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut incoming = Vec::new();
        if let Some(t) = &self.transport {
            while let Ok(i) = t.incoming.try_recv() {
                incoming.push(i);
            }
        }
        for i in incoming {
            match i {
                Incoming::Message(m) => self.handle(m),
                Incoming::Closed(reason) => {
                    let asked = matches!(self.state, State::Stopping);
                    self.stop(if asked { "stopped".into() } else { reason }, asked);
                }
            }
        }
        let now = Instant::now();
        if self.state == State::Starting && now - self.started > START_TIMEOUT {
            self.stop("did not answer initialize in time".into(), false);
        }
        if let Some((_, at)) = self.shutdown
            && now - at > SHUTDOWN_TIMEOUT
        {
            self.exit();
        }
        let late: Vec<i64> = self
            .pending
            .iter()
            .filter(|(_, p)| now - p.sent > p.timeout)
            .map(|(id, _)| *id)
            .collect();
        for id in late {
            if let Some(p) = self.pending.remove(&id) {
                self.notify("$/cancelRequest", json!({ "id": id }));
                self.events.push(Event::Answer {
                    tag: p.tag,
                    result: Err(ResponseError {
                        code: codes::TIMED_OUT,
                        message: "the server took too long".into(),
                    }),
                });
            }
        }
        std::mem::take(&mut self.events)
    }

    fn stop(&mut self, reason: String, asked: bool) {
        if matches!(self.state, State::Stopped { .. }) {
            return;
        }
        log(&self.log, &format!("[ochrepad] stopped: {reason}"));
        if let Some(mut t) = self.transport.take() {
            t.close();
        }
        for (_, p) in self.pending.drain() {
            self.events.push(Event::Answer {
                tag: p.tag,
                result: Err(ResponseError {
                    code: codes::REQUEST_CANCELLED,
                    message: "the server stopped".into(),
                }),
            });
        }
        self.state = State::Stopped {
            reason: reason.clone(),
            asked,
        };
        self.events.push(Event::Stopped { reason, asked });
    }

    fn handle(&mut self, m: Message) {
        match m {
            Message::Response {
                id: Id::Number(id),
                result,
            } if Some(id) == self.init_id => {
                self.init_id = None;
                match result {
                    Ok(v) => self.initialized(v),
                    Err(e) => self.stop(format!("initialize failed: {}", e.message), false),
                }
            }
            Message::Response {
                id: Id::Number(id), ..
            } if self.shutdown.is_some_and(|(s, _)| s == id) => {
                self.exit();
            }
            Message::Response {
                id: Id::Number(id),
                result,
            } => {
                if let Some(p) = self.pending.remove(&id) {
                    self.events.push(Event::Answer { tag: p.tag, result });
                }
            }
            Message::Response { .. } => {}
            Message::Notification { method, params } => self.notification(&method, params),
            Message::Request { id, method, params } => self.server_request(id, &method, params),
        }
    }

    fn initialized(&mut self, result: Value) {
        self.capabilities = result.get("capabilities").cloned().unwrap_or(Value::Null);
        self.units = match self
            .capabilities
            .get("positionEncoding")
            .and_then(Value::as_str)
        {
            Some("utf-8") => ColumnUnits::Utf8,
            Some("utf-32") => ColumnUnits::Utf32,
            _ => ColumnUnits::Utf16,
        };
        let sync = self.capabilities.get("textDocumentSync");
        let kind = match sync {
            Some(Value::Number(n)) => n.as_i64(),
            Some(o) => o.get("change").and_then(Value::as_i64),
            None => None,
        };
        self.sync = match kind {
            Some(2) => Sync::Incremental,
            Some(1) => Sync::Full,
            _ => Sync::None,
        };
        self.save_text = sync
            .and_then(|s| s.get("save"))
            .and_then(|s| s.get("includeText"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.notify("initialized", json!({}));
        if !self.settings.is_null() {
            let settings = self.settings.clone();
            self.notify(
                "workspace/didChangeConfiguration",
                json!({ "settings": settings }),
            );
        }
        self.state = State::Running;
        log(&self.log, "[ochrepad] running");
        // The documents opened while it started.
        let uris: Vec<String> = self.docs.keys().cloned().collect();
        for u in uris {
            self.send_open(&u);
        }
        self.events.push(Event::Started);
    }

    fn notification(&mut self, method: &str, params: Value) {
        match method {
            "textDocument/publishDiagnostics" => {
                let Some(uri) = params.get("uri").and_then(Value::as_str) else {
                    return;
                };
                let diagnostics = match params.get("diagnostics") {
                    Some(Value::Array(a)) => a.clone(),
                    _ => Vec::new(),
                };
                self.events.push(Event::Diagnostics {
                    uri: uri::normalize(uri),
                    version: params.get("version").and_then(Value::as_i64),
                    diagnostics,
                });
            }
            "window/logMessage" => {
                if let Some(text) = params.get("message").and_then(Value::as_str) {
                    for line in text.lines() {
                        log(&self.log, line);
                    }
                }
            }
            "window/showMessage" => self.message(&params),
            "$/progress" => {
                let token = match params.get("token") {
                    Some(Value::String(s)) => s.clone(),
                    Some(v) => v.to_string(),
                    None => return,
                };
                let v = params.get("value").cloned().unwrap_or(Value::Null);
                let text = match v.get("kind").and_then(Value::as_str) {
                    Some("end") => {
                        self.progress_titles.remove(&token);
                        None
                    }
                    _ => {
                        let part = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
                        // Reports leave out the title `begin` gave.
                        let title = match part("title") {
                            Some(t) => {
                                self.progress_titles.insert(token.clone(), t.clone());
                                Some(t)
                            }
                            None => self.progress_titles.get(&token).cloned(),
                        };
                        let mut t = [title, part("message")]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join(": ");
                        if let Some(p) = v.get("percentage").and_then(Value::as_u64) {
                            t.push_str(&format!(" ({p}%)"));
                        }
                        Some(t)
                    }
                };
                self.events.push(Event::Progress { token, text });
            }
            _ => {}
        }
    }

    fn message(&mut self, params: &Value) {
        if let Some(text) = params.get("message").and_then(Value::as_str) {
            let kind = params.get("type").and_then(Value::as_i64).unwrap_or(3);
            log(&self.log, text);
            self.events.push(Event::Message {
                kind,
                text: text.to_owned(),
            });
        }
    }

    fn server_request(&mut self, id: Id, method: &str, params: Value) {
        let answer = match method {
            "workspace/configuration" => {
                let items = params
                    .get("items")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                Ok(Value::Array(
                    items
                        .iter()
                        .map(|item| {
                            let section = item.get("section").and_then(Value::as_str);
                            setting(&self.settings, section)
                        })
                        .collect(),
                ))
            }
            "workspace/workspaceFolders" => {
                let name = self
                    .root
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                Ok(json!([{ "uri": self.root_uri, "name": name }]))
            }
            "window/workDoneProgress/create"
            | "client/registerCapability"
            | "client/unregisterCapability"
            | "workspace/semanticTokens/refresh"
            | "workspace/inlayHint/refresh"
            | "workspace/inlineValue/refresh"
            | "workspace/codeLens/refresh"
            | "workspace/diagnostic/refresh"
            | "workspace/foldingRange/refresh" => Ok(Value::Null),
            "window/showMessageRequest" => {
                self.message(&params);
                Ok(Value::Null)
            }
            "window/showDocument" => {
                let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
                let local = uri::file_path(uri).is_some();
                if local {
                    self.events.push(Event::ShowDocument {
                        uri: uri::normalize(uri),
                        selection: params.get("selection").cloned(),
                    });
                }
                Ok(json!({ "success": local }))
            }
            "workspace/applyEdit" => {
                // The app answers.
                self.events.push(Event::ApplyEdit {
                    id,
                    edit: params.get("edit").cloned().unwrap_or(Value::Null),
                });
                return;
            }
            _ => Err(ResponseError {
                code: codes::METHOD_NOT_FOUND,
                message: format!("ochrepad does not handle {method}"),
            }),
        };
        self.send(&Message::Response { id, result: answer });
    }

    /// Answers a `workspace/applyEdit` request.
    pub fn answer_apply_edit(&mut self, id: Id, applied: bool, reason: Option<&str>) {
        let mut result = json!({ "applied": applied });
        if let Some(r) = reason {
            result["failureReason"] = json!(r);
        }
        self.send(&Message::Response {
            id,
            result: Ok(result),
        });
    }

    /// Sends a request; its answer comes back from [`Self::poll`] with
    /// `tag`. `None` while the server is not running.
    pub fn request(
        &mut self,
        method: &str,
        params: Value,
        tag: Tag,
        timeout: Duration,
    ) -> Option<i64> {
        if self.state != State::Running {
            return None;
        }
        let id = self.take_id();
        self.send(&Message::Request {
            id: Id::Number(id),
            method: method.into(),
            params,
        });
        self.pending.insert(
            id,
            Pending {
                tag,
                sent: Instant::now(),
                timeout,
            },
        );
        Some(id)
    }

    /// Tells the server a request's answer is no longer wanted; it will
    /// not come back from [`Self::poll`].
    pub fn cancel(&mut self, id: i64) {
        if self.pending.remove(&id).is_some() {
            self.notify("$/cancelRequest", json!({ "id": id }));
        }
    }

    pub fn notify(&mut self, method: &str, params: Value) {
        self.send(&Message::Notification {
            method: method.into(),
            params,
        });
    }

    /// Asks the server to shut down (then exit).
    pub fn shutdown(&mut self) {
        match self.state {
            State::Running => {
                let id = self.take_id();
                self.send(&Message::Request {
                    id: Id::Number(id),
                    method: "shutdown".into(),
                    params: Value::Null,
                });
                self.shutdown = Some((id, Instant::now()));
                self.state = State::Stopping;
            }
            State::Starting => self.exit(),
            _ => {}
        }
    }

    fn exit(&mut self) {
        self.shutdown = None;
        self.notify("exit", Value::Null);
        self.state = State::Stopping;
        if let Some(t) = &mut self.transport {
            // The server sees its input end; its output ending stops us.
            t.close();
        }
    }

    // --- Documents ------------------------------------------------------

    /// Gives the server a document (sent once it is running).
    pub fn open(&mut self, uri: &str, language_id: &str, doc: &Document) {
        if self.docs.contains_key(uri) {
            self.change(uri, doc);
            return;
        }
        self.docs.insert(
            uri.to_owned(),
            OpenDoc {
                version: 1,
                text: doc.clone(),
                language_id: language_id.to_owned(),
                sent: false,
            },
        );
        if self.state == State::Running {
            self.send_open(uri);
        }
    }

    fn send_open(&mut self, uri: &str) {
        let Some(d) = self.docs.get_mut(uri) else {
            return;
        };
        d.sent = true;
        let params = json!({
            "textDocument": {
                "uri": uri,
                "languageId": d.language_id,
                "version": d.version,
                "text": d.text.to_string(),
            }
        });
        self.notify("textDocument/didOpen", params);
    }

    pub fn is_open(&self, uri: &str) -> bool {
        self.docs.contains_key(uri)
    }

    /// The version the server has of `uri`.
    pub fn version(&self, uri: &str) -> Option<i64> {
        self.docs.get(uri).map(|d| d.version)
    }

    /// The text the server has of `uri` (positions in its answers refer to
    /// it).
    pub fn text(&self, uri: &str) -> Option<&Document> {
        self.docs.get(uri).map(|d| &d.text)
    }

    /// The document's text is now `doc`: sends what changed. Returns the
    /// range of the old text that changed and the new text's range in its
    /// place (`None` when nothing did).
    pub fn change(
        &mut self,
        uri: &str,
        doc: &Document,
    ) -> Option<(Range<ByteOffset>, Range<ByteOffset>)> {
        let (sync, units) = (self.sync, self.units);
        let d = self.docs.get_mut(uri)?;
        let (old, new) = d.text.changed_range(doc)?;
        if !d.sent {
            d.text = doc.clone();
            return Some((old, new));
        }
        d.version += 1;
        let change = match sync {
            Sync::None => None,
            Sync::Full => Some(json!({ "text": doc.to_string() })),
            Sync::Incremental => Some(json!({
                "range": range_json(&d.text, &old, units),
                "text": doc.slice(new.clone()),
            })),
        };
        d.text = doc.clone();
        let version = d.version;
        if let Some(change) = change {
            self.notify(
                "textDocument/didChange",
                json!({
                    "textDocument": { "uri": uri, "version": version },
                    "contentChanges": [change],
                }),
            );
        }
        Some((old, new))
    }

    pub fn save(&mut self, uri: &str) {
        let Some(d) = self.docs.get(uri).filter(|d| d.sent) else {
            return;
        };
        let mut params = json!({ "textDocument": { "uri": uri } });
        if self.save_text {
            params["text"] = json!(d.text.to_string());
        }
        self.notify("textDocument/didSave", params);
    }

    pub fn close(&mut self, uri: &str) {
        if let Some(d) = self.docs.remove(uri)
            && d.sent
        {
            self.notify(
                "textDocument/didClose",
                json!({ "textDocument": { "uri": uri } }),
            );
        }
    }

    /// The documents given to the server.
    pub fn documents(&self) -> impl Iterator<Item = &str> {
        self.docs.keys().map(String::as_str)
    }

    // --- Positions ------------------------------------------------------

    /// `pos` in `doc` as the server counts it.
    pub fn position(&self, doc: &Document, pos: ByteOffset) -> Value {
        position_json(doc, pos, self.units)
    }

    /// The offset a server's position names in `doc` (clamped).
    pub fn offset(&self, doc: &Document, position: &Value) -> Option<ByteOffset> {
        offset_of(doc, position, self.units)
    }

    /// The offsets a server's range names in `doc` (clamped, ordered).
    pub fn range(&self, doc: &Document, range: &Value) -> Option<Range<ByteOffset>> {
        let start = self.offset(doc, range.get("start")?)?;
        let end = self.offset(doc, range.get("end")?)?;
        Some(start.min(end)..start.max(end))
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if matches!(self.state, State::Running | State::Starting) {
            self.notify("exit", Value::Null);
        }
    }
}

fn position_json(doc: &Document, pos: ByteOffset, units: ColumnUnits) -> Value {
    let (line, character) = to_lsp_position(doc, pos, units);
    json!({ "line": line, "character": character })
}

fn range_json(doc: &Document, range: &Range<ByteOffset>, units: ColumnUnits) -> Value {
    json!({
        "start": position_json(doc, range.start, units),
        "end": position_json(doc, range.end, units),
    })
}

fn offset_of(doc: &Document, position: &Value, units: ColumnUnits) -> Option<ByteOffset> {
    let n = |k: &str| {
        position
            .get(k)
            .and_then(Value::as_u64)
            .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
    };
    Some(from_lsp_position(doc, n("line")?, n("character")?, units))
}

/// The settings for `section` (a dotted path into the configured
/// settings; all of them for none).
fn setting(settings: &Value, section: Option<&str>) -> Value {
    let Some(section) = section.filter(|s| !s.is_empty()) else {
        return settings.clone();
    };
    let mut v = settings;
    for part in section.split('.') {
        match v.get(part) {
            Some(next) => v = next,
            None => return Value::Null,
        }
    }
    v.clone()
}

fn initialize_params(root: &Path, root_uri: &str, config: &ServerConfig) -> Value {
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut params = json!({
        "processId": std::process::id(),
        "clientInfo": { "name": "ochrepad", "version": env!("CARGO_PKG_VERSION") },
        "rootUri": root_uri,
        "rootPath": root.to_string_lossy(),
        "workspaceFolders": [{ "uri": root_uri, "name": name }],
        "capabilities": {
            "general": { "positionEncodings": ["utf-8", "utf-16"] },
            "workspace": {
                "configuration": true,
                "workspaceFolders": true,
                "applyEdit": true,
                "workspaceEdit": { "documentChanges": true },
                "didChangeConfiguration": { "dynamicRegistration": false },
                "symbol": { "dynamicRegistration": false },
            },
            "window": {
                "workDoneProgress": true,
                "showDocument": { "support": true },
                "showMessage": {},
            },
            "textDocument": {
                "synchronization": { "didSave": true, "willSave": false, "dynamicRegistration": false },
                "completion": {
                    "contextSupport": true,
                    "completionItem": {
                        "snippetSupport": false,
                        "documentationFormat": ["markdown", "plaintext"],
                        "labelDetailsSupport": true,
                        "resolveSupport": { "properties": ["documentation", "detail"] },
                    },
                },
                "hover": { "contentFormat": ["markdown", "plaintext"] },
                "signatureHelp": {
                    "signatureInformation": {
                        "documentationFormat": ["markdown", "plaintext"],
                        "parameterInformation": { "labelOffsetSupport": true },
                    },
                },
                "definition": { "linkSupport": false },
                "references": {},
                "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
                "formatting": {},
                "rename": { "prepareSupport": true },
                "codeAction": {
                    "codeActionLiteralSupport": {
                        "codeActionKind": {
                            "valueSet": [
                                "quickfix", "refactor", "refactor.extract", "refactor.inline",
                                "refactor.rewrite", "source", "source.organizeImports",
                            ],
                        },
                    },
                    "resolveSupport": { "properties": ["edit"] },
                },
                "publishDiagnostics": { "relatedInformation": false, "versionSupport": true },
            },
        },
    });
    if !config.initialization_options.is_null() {
        params["initializationOptions"] = config.initialization_options.clone();
    }
    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_by_section() {
        let s = json!({"rust-analyzer": {"check": {"command": "clippy"}}});
        assert_eq!(
            setting(&s, Some("rust-analyzer.check.command")),
            json!("clippy")
        );
        assert_eq!(setting(&s, Some("rust-analyzer.nope")), Value::Null);
        assert_eq!(setting(&s, None), s);
    }
}
