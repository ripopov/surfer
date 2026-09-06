//! Message transports for the language server client.
//!
//! [`ProcessTransport`] talks to a real `slang-server` child over stdio. [`ReplayTransport`]
//! answers requests from a recorded script so the rest of the client, the tile and the
//! snapshot tests run without a C++ toolchain. Both deliver incoming messages through the
//! same callback, so the client cannot tell them apart.

use super::protocol::{self, Incoming};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

/// Delivers messages that arrive from the server.
pub type OnMessage = Arc<dyn Fn(Incoming) + Send + Sync>;

pub trait Transport: Send + Sync {
    /// Sends one JSON-RPC message.
    fn send(&self, message: &Value);
    /// A human-readable description for status displays and logs.
    fn describe(&self) -> String;
}

/// A `slang-server` child process speaking LSP over stdio.
pub struct ProcessTransport {
    child: Mutex<Child>,
    stdin: Mutex<std::process::ChildStdin>,
    command: String,
}

impl ProcessTransport {
    /// Spawns `binary` with `workspace` as its working directory. Server stderr is copied
    /// to the log at debug level; stdout frames are decoded on a reader thread and
    /// delivered to `on_message`.
    pub fn spawn(binary: &str, workspace: &str, on_message: OnMessage) -> std::io::Result<Self> {
        let mut child = Command::new(binary)
            .current_dir(workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        std::thread::Builder::new()
            .name("slang-server stdout".into())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    match protocol::read_frame(&mut reader) {
                        Ok(Some(message)) => {
                            if let Some(incoming) = protocol::classify(message) {
                                on_message(incoming);
                            }
                        }
                        Ok(None) => break,
                        Err(error) => {
                            tracing::warn!(%error, "slang-server stream ended");
                            break;
                        }
                    }
                }
                on_message(Incoming::Notification {
                    method: "surfer/serverExited".into(),
                    params: Value::Null,
                });
            })?;
        std::thread::Builder::new()
            .name("slang-server stderr".into())
            .spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    tracing::debug!(target: "slang_server", "{line}");
                }
            })?;
        Ok(Self {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            command: binary.to_owned(),
        })
    }
}

impl Transport for ProcessTransport {
    fn send(&self, message: &Value) {
        let mut stdin = self.stdin.lock().unwrap();
        if let Err(error) = protocol::write_frame(&mut *stdin, message) {
            tracing::warn!(%error, "Failed to write to slang-server");
        }
    }

    fn describe(&self) -> String {
        self.command.clone()
    }
}

impl Drop for ProcessTransport {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// One recorded request/response pair. Paths inside `params` and `result` are
/// normalized with [`ReplayScript::placeholders`] so recordings stay portable.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Exchange {
    pub method: String,
    pub params: Value,
    #[serde(default)]
    pub result: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

/// A recording of a session with the real server.
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct ReplayScript {
    /// Server-initiated notifications replayed after `initialized`, in order.
    #[serde(default)]
    pub notifications: Vec<Exchange>,
    pub exchanges: Vec<Exchange>,
}

impl ReplayScript {
    /// Removes fields that legitimately differ between sessions before matching.
    pub fn stable_params(method: &str, mut params: Value) -> Value {
        if method == "initialize"
            && let Some(fields) = params.as_object_mut()
        {
            fields.remove("processId");
        }
        params
    }

    /// Replaces every occurrence of a concrete path with its placeholder (or back).
    pub fn substitute(value: &mut Value, pairs: &[(String, String)]) {
        match value {
            Value::String(text) => {
                for (from, to) in pairs {
                    if text.contains(from.as_str()) {
                        *text = text.replace(from.as_str(), to);
                    }
                }
            }
            Value::Array(items) => items
                .iter_mut()
                .for_each(|item| Self::substitute(item, pairs)),
            Value::Object(fields) => fields
                .values_mut()
                .for_each(|item| Self::substitute(item, pairs)),
            _ => {}
        }
    }
}

/// Answers requests from a [`ReplayScript`] synchronously.
pub struct ReplayTransport {
    script: ReplayScript,
    /// `(concrete, placeholder)` pairs applied to outgoing params before matching and to
    /// recorded results before delivery (reversed).
    placeholders: Vec<(String, String)>,
    on_message: OnMessage,
    unmatched: Mutex<Vec<String>>,
}

impl ReplayTransport {
    pub fn new(
        script: ReplayScript,
        placeholders: Vec<(String, String)>,
        on_message: OnMessage,
    ) -> Self {
        Self {
            script,
            placeholders,
            on_message,
            unmatched: Mutex::new(Vec::new()),
        }
    }

    /// Requests that no recorded exchange answered, for test diagnostics.
    pub fn unmatched(&self) -> Vec<String> {
        self.unmatched.lock().unwrap().clone()
    }

    fn normalize(&self, mut value: Value) -> Value {
        ReplayScript::substitute(&mut value, &self.placeholders);
        value
    }

    fn concrete(&self, mut value: Value) -> Value {
        let reversed: Vec<_> = self
            .placeholders
            .iter()
            .map(|(concrete, placeholder)| (placeholder.clone(), concrete.clone()))
            .collect();
        ReplayScript::substitute(&mut value, &reversed);
        value
    }
}

impl Transport for ReplayTransport {
    fn send(&self, message: &Value) {
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return;
        };
        let params = self.normalize(ReplayScript::stable_params(
            method,
            message.get("params").cloned().unwrap_or(Value::Null),
        ));
        let Some(id) = message.get("id").and_then(Value::as_u64) else {
            if method == "initialized" {
                for note in &self.script.notifications {
                    (self.on_message)(Incoming::Notification {
                        method: note.method.clone(),
                        params: self.concrete(note.params.clone()),
                    });
                }
            }
            return;
        };
        let found = self
            .script
            .exchanges
            .iter()
            .find(|exchange| exchange.method == method && exchange.params == params);
        match found {
            Some(exchange) => (self.on_message)(Incoming::Response {
                id,
                result: exchange
                    .error
                    .is_none()
                    .then(|| self.concrete(exchange.result.clone())),
                error: exchange.error.clone(),
            }),
            None => {
                self.unmatched
                    .lock()
                    .unwrap()
                    .push(format!("{method} {params}"));
                (self.on_message)(Incoming::Response {
                    id,
                    result: None,
                    error: Some(serde_json::json!({
                        "code": -32601,
                        "message": format!("no recorded response for {method}"),
                    })),
                });
            }
        }
    }

    fn describe(&self) -> String {
        "recorded session".to_owned()
    }
}

/// Wraps a transport and records every request together with its response, so a
/// session with the real server can be replayed later.
pub struct RecordingTransport {
    inner: Arc<dyn Transport>,
    pending: Arc<Mutex<std::collections::HashMap<u64, (String, Value)>>>,
    script: Arc<Mutex<ReplayScript>>,
    placeholders: Vec<(String, String)>,
}

impl RecordingTransport {
    /// `on_message` receives the messages after they have been recorded.
    pub fn new(
        placeholders: Vec<(String, String)>,
        on_message: OnMessage,
        make_inner: impl FnOnce(OnMessage) -> Arc<dyn Transport>,
    ) -> Self {
        let pending: Arc<Mutex<std::collections::HashMap<u64, (String, Value)>>> = Arc::default();
        let script: Arc<Mutex<ReplayScript>> = Arc::default();
        let recorder = {
            let pending = pending.clone();
            let script = script.clone();
            let placeholders = placeholders.clone();
            Arc::new(move |incoming: Incoming| {
                match &incoming {
                    Incoming::Response { id, result, error } => {
                        if let Some((method, params)) = pending.lock().unwrap().remove(id) {
                            let mut result = result.clone().unwrap_or(Value::Null);
                            ReplayScript::substitute(&mut result, &placeholders);
                            script.lock().unwrap().exchanges.push(Exchange {
                                method,
                                params,
                                result,
                                error: error.clone(),
                            });
                        }
                    }
                    Incoming::Notification { method, params } => {
                        if method != "surfer/serverExited" {
                            let mut params = params.clone();
                            ReplayScript::substitute(&mut params, &placeholders);
                            script.lock().unwrap().notifications.push(Exchange {
                                method: method.clone(),
                                params,
                                result: Value::Null,
                                error: None,
                            });
                        }
                    }
                    Incoming::Request { .. } => {}
                }
                on_message(incoming);
            }) as OnMessage
        };
        Self {
            inner: make_inner(recorder),
            pending,
            script,
            placeholders,
        }
    }

    pub fn script(&self) -> ReplayScript {
        self.script.lock().unwrap().clone()
    }
}

impl Transport for RecordingTransport {
    fn send(&self, message: &Value) {
        if let (Some(id), Some(method)) = (
            message.get("id").and_then(Value::as_u64),
            message.get("method").and_then(Value::as_str),
        ) {
            let mut params = ReplayScript::stable_params(
                method,
                message.get("params").cloned().unwrap_or(Value::Null),
            );
            ReplayScript::substitute(&mut params, &self.placeholders);
            self.pending
                .lock()
                .unwrap()
                .insert(id, (method.to_owned(), params));
        }
        self.inner.send(message);
    }

    fn describe(&self) -> String {
        format!("recording {}", self.inner.describe())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn collector() -> (OnMessage, Arc<Mutex<Vec<Incoming>>>) {
        let seen: Arc<Mutex<Vec<Incoming>>> = Arc::default();
        let sink = seen.clone();
        (
            Arc::new(move |incoming| sink.lock().unwrap().push(incoming)),
            seen,
        )
    }

    #[test]
    fn replay_matches_normalized_params_and_restores_paths() {
        let script = ReplayScript {
            notifications: vec![],
            exchanges: vec![Exchange {
                method: "textDocument/hover".into(),
                params: json!({"textDocument": {"uri": "file://${ROOT}/a.sv"}}),
                result: json!({"uri": "file://${ROOT}/a.sv"}),
                error: None,
            }],
        };
        let (on_message, seen) = collector();
        let transport = ReplayTransport::new(
            script,
            vec![("/tmp/design".into(), "${ROOT}".into())],
            on_message,
        );
        transport.send(&protocol::request(
            7,
            "textDocument/hover",
            json!({"textDocument": {"uri": "file:///tmp/design/a.sv"}}),
        ));
        transport.send(&protocol::request(
            8,
            "textDocument/hover",
            json!({"other": 1}),
        ));
        let seen = seen.lock().unwrap();
        assert_eq!(
            seen[0],
            Incoming::Response {
                id: 7,
                result: Some(json!({"uri": "file:///tmp/design/a.sv"})),
                error: None
            }
        );
        assert!(matches!(
            &seen[1],
            Incoming::Response {
                id: 8,
                result: None,
                error: Some(_)
            }
        ));
        assert_eq!(transport.unmatched().len(), 1);
    }

    #[test]
    fn recording_captures_exchanges_with_placeholders() {
        let (on_message, _seen) = collector();
        let recording = RecordingTransport::new(
            vec![("/tmp/design".into(), "${ROOT}".into())],
            on_message,
            |deliver| {
                Arc::new(ReplayTransport::new(
                    ReplayScript {
                        notifications: vec![],
                        exchanges: vec![Exchange {
                            method: "initialize".into(),
                            params: json!({"rootUri": "file://${ROOT}"}),
                            result: json!({"capabilities": {}}),
                            error: None,
                        }],
                    },
                    vec![("/tmp/design".into(), "${ROOT}".into())],
                    deliver,
                ))
            },
        );
        recording.send(&protocol::request(
            1,
            "initialize",
            json!({"rootUri": "file:///tmp/design"}),
        ));
        let script = recording.script();
        assert_eq!(script.exchanges.len(), 1);
        assert_eq!(
            script.exchanges[0].params,
            json!({"rootUri": "file://${ROOT}"})
        );
    }
}
