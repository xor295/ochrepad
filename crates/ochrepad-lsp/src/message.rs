//! JSON-RPC 2.0 messages, kept as JSON values: a message is turned into
//! the protocol's types only where it is used, so one a server got wrong
//! is dropped on its own instead of ending the session.

use serde_json::{Value, json};

/// A request's id.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Id {
    Number(i64),
    Text(String),
}

impl Id {
    fn from_value(v: &Value) -> Option<Self> {
        match v {
            Value::Number(n) => n.as_i64().map(Id::Number),
            Value::String(s) => Some(Id::Text(s.clone())),
            _ => None,
        }
    }

    fn to_value(&self) -> Value {
        match self {
            Id::Number(n) => json!(n),
            Id::Text(s) => json!(s),
        }
    }
}

/// An error a request was answered with.
#[derive(Clone, Debug, PartialEq)]
pub struct ResponseError {
    pub code: i64,
    pub message: String,
}

/// The protocol's error codes used here.
pub mod codes {
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const REQUEST_CANCELLED: i64 = -32800;
    /// Not the protocol's: a request this client gave up on.
    pub const TIMED_OUT: i64 = -32099;
}

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Request {
        id: Id,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    Response {
        id: Id,
        result: Result<Value, ResponseError>,
    },
}

impl Message {
    /// Parses a message's body.
    pub fn parse(body: &[u8]) -> Result<Message, String> {
        let v: Value = serde_json::from_slice(body).map_err(|e| format!("not JSON: {e}"))?;
        let Value::Object(mut o) = v else {
            return Err("not a JSON object".into());
        };
        let params = o.remove("params").unwrap_or(Value::Null);
        let id = o.get("id").filter(|v| !v.is_null()).map(Id::from_value);
        match (o.get("method").and_then(Value::as_str), id) {
            (Some(method), None) => Ok(Message::Notification {
                method: method.to_owned(),
                params,
            }),
            (Some(method), Some(Some(id))) => Ok(Message::Request {
                id,
                method: method.to_owned(),
                params,
            }),
            (None, Some(Some(id))) => {
                let result = match o.remove("error") {
                    Some(e) if !e.is_null() => Err(ResponseError {
                        code: e.get("code").and_then(Value::as_i64).unwrap_or(0),
                        message: e
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                    }),
                    _ => Ok(o.remove("result").unwrap_or(Value::Null)),
                };
                Ok(Message::Response { id, result })
            }
            _ => Err("neither a request, a notification nor a response".into()),
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let v = match self {
            Message::Request { id, method, params } => {
                json!({"jsonrpc": "2.0", "id": id.to_value(), "method": method, "params": params})
            }
            Message::Notification { method, params } => {
                json!({"jsonrpc": "2.0", "method": method, "params": params})
            }
            Message::Response { id, result } => match result {
                Ok(result) => json!({"jsonrpc": "2.0", "id": id.to_value(), "result": result}),
                Err(e) => json!({
                    "jsonrpc": "2.0",
                    "id": id.to_value(),
                    "error": {"code": e.code, "message": e.message},
                }),
            },
        };
        serde_json::to_vec(&v).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing_and_writing() {
        let all = [
            Message::Request {
                id: Id::Number(3),
                method: "textDocument/hover".into(),
                params: json!({"x": 1}),
            },
            Message::Request {
                id: Id::Text("a".into()),
                method: "workspace/configuration".into(),
                params: json!([]),
            },
            Message::Notification {
                method: "initialized".into(),
                params: json!({}),
            },
            Message::Response {
                id: Id::Number(1),
                result: Ok(json!(null)),
            },
            Message::Response {
                id: Id::Number(2),
                result: Err(ResponseError {
                    code: -32601,
                    message: "no".into(),
                }),
            },
        ];
        for m in all {
            assert_eq!(Message::parse(&m.to_bytes()).unwrap(), m);
        }
    }

    #[test]
    fn what_is_not_a_message() {
        for bad in [
            &b"[1]"[..],
            b"nope",
            b"{\"jsonrpc\":\"2.0\"}",
            b"{\"id\":{\"x\":1},\"result\":1}",
        ] {
            assert!(
                Message::parse(bad).is_err(),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
    }
}
