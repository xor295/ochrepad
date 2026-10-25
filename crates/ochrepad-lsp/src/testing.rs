//! A scripted language server on a thread, for tests (the `testing`
//! feature): it answers requests with functions given to it, sends
//! notifications in reply to what it is sent, and records everything.

use std::collections::HashMap;
use std::io::{BufReader, Write};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::framing;
use crate::message::{Id, Message, ResponseError, codes};
use crate::transport::Launch;

type Answer = Arc<dyn Fn(&Value) -> Value + Send + Sync>;
type Reaction = Arc<dyn Fn(&Value) -> Vec<(String, Value)> + Send + Sync>;

#[derive(Clone)]
pub struct FakeServer {
    /// Everything it was sent, in order.
    pub received: Arc<Mutex<Vec<Message>>>,
    capabilities: Value,
    answers: HashMap<String, Answer>,
    reactions: HashMap<String, Reaction>,
    /// Requests made of the client once it says `initialized`.
    requests: Vec<(String, Value)>,
    /// Requests made of the client before a request is answered.
    requests_on: HashMap<String, Reaction>,
}

impl Default for FakeServer {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeServer {
    /// A server with incremental sync, UTF-16 positions, completion and
    /// hover.
    pub fn new() -> Self {
        Self {
            received: Arc::default(),
            capabilities: json!({
                "textDocumentSync": { "openClose": true, "change": 2, "save": { "includeText": false } },
                "completionProvider": { "triggerCharacters": ["."] },
                "hoverProvider": true,
            }),
            answers: HashMap::new(),
            reactions: HashMap::new(),
            requests: Vec::new(),
            requests_on: HashMap::new(),
        }
    }

    pub fn capabilities(mut self, caps: Value) -> Self {
        self.capabilities = caps;
        self
    }

    /// Answers requests for `method` with `f(params)`.
    pub fn answer(
        mut self,
        method: &str,
        f: impl Fn(&Value) -> Value + Send + Sync + 'static,
    ) -> Self {
        self.answers.insert(method.into(), Arc::new(f));
        self
    }

    /// After a message for `method` arrives, sends the notifications
    /// `f(params)` gives.
    pub fn react(
        mut self,
        method: &str,
        f: impl Fn(&Value) -> Vec<(String, Value)> + Send + Sync + 'static,
    ) -> Self {
        self.reactions.insert(method.into(), Arc::new(f));
        self
    }

    /// Before answering a request for `method`, makes the requests of the
    /// client `f(params)` gives, as a server asks the client to apply an
    /// edit while it runs a command.
    pub fn request_on(
        mut self,
        method: &str,
        f: impl Fn(&Value) -> Vec<(String, Value)> + Send + Sync + 'static,
    ) -> Self {
        self.requests_on.insert(method.into(), Arc::new(f));
        self
    }

    /// Makes a request of the client once it is initialized.
    pub fn request_after_start(mut self, method: &str, params: Value) -> Self {
        self.requests.push((method.into(), params));
        self
    }

    /// The methods it was sent, in order.
    pub fn methods(&self) -> Vec<String> {
        self.received
            .lock()
            .map(|r| {
                r.iter()
                    .filter_map(|m| match m {
                        Message::Request { method, .. } | Message::Notification { method, .. } => {
                            Some(method.clone())
                        }
                        Message::Response { .. } => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The params of the messages for `method` it was sent.
    pub fn params(&self, method: &str) -> Vec<Value> {
        self.received
            .lock()
            .map(|r| {
                r.iter()
                    .filter_map(|m| match m {
                        Message::Request {
                            method: m, params, ..
                        }
                        | Message::Notification { method: m, params }
                            if m == method =>
                        {
                            Some(params.clone())
                        }
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The answers the client gave to its requests.
    pub fn responses(&self) -> Vec<Result<Value, ResponseError>> {
        self.received
            .lock()
            .map(|r| {
                r.iter()
                    .filter_map(|m| match m {
                        Message::Response { result, .. } => Some(result.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn launch(&self) -> Launch {
        let server = self.clone();
        Launch::InProcess(Arc::new(move |input, output| server.run(input, output)))
    }

    fn run(&self, input: std::io::PipeReader, mut output: std::io::PipeWriter) {
        let mut r = BufReader::new(input);
        let mut next_id = 1000;
        let send = |out: &mut std::io::PipeWriter, m: Message| {
            let _ = framing::write_message(out, &m.to_bytes());
            let _ = out.flush();
        };
        while let Ok(Some(body)) = framing::read_message(&mut r) {
            let Ok(m) = Message::parse(&body) else {
                continue;
            };
            if let Ok(mut rec) = self.received.lock() {
                rec.push(m.clone());
            }
            let (method, params) = match &m {
                Message::Request { method, params, .. }
                | Message::Notification { method, params } => (method.clone(), params.clone()),
                Message::Response { .. } => continue,
            };
            if let Message::Request { id, .. } = &m {
                if let Some(f) = self.requests_on.get(&method) {
                    for (method, params) in f(&params) {
                        next_id += 1;
                        send(
                            &mut output,
                            Message::Request {
                                id: Id::Number(next_id),
                                method,
                                params,
                            },
                        );
                    }
                }
                let result = match method.as_str() {
                    "initialize" => Ok(json!({ "capabilities": self.capabilities })),
                    "shutdown" => Ok(Value::Null),
                    other => match self.answers.get(other) {
                        Some(f) => Ok(f(&params)),
                        None => Err(ResponseError {
                            code: codes::METHOD_NOT_FOUND,
                            message: other.into(),
                        }),
                    },
                };
                send(
                    &mut output,
                    Message::Response {
                        id: id.clone(),
                        result,
                    },
                );
            }
            if method == "exit" {
                return;
            }
            if method == "initialized" {
                for (method, params) in &self.requests {
                    next_id += 1;
                    send(
                        &mut output,
                        Message::Request {
                            id: Id::Number(next_id),
                            method: method.clone(),
                            params: params.clone(),
                        },
                    );
                }
            }
            if let Some(f) = self.reactions.get(&method) {
                for (method, params) in f(&params) {
                    send(&mut output, Message::Notification { method, params });
                }
            }
        }
    }
}
