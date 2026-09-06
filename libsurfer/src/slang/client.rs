//! The language-server client that backs the source tile.
//!
//! One client serves one loaded design. The tile only reads caches; every server round
//! trip is asynchronous: a request is issued when a cache entry is missing, the reply
//! arrives on the message bus as [`Message::Slang`], and [`SlangClient::handle`] fills the
//! cache or emits follow-up messages such as opening a file or adding a signal.

use super::launch::LaunchPlan;
use super::protocol::{self, Incoming, file_uri, uri_path};
use super::tokens::{Legend, LineTokens, Range, TokenClass};
use super::transport::{OnMessage, Transport};
use crate::message::Message;
use camino::{Utf8Path, Utf8PathBuf};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, Weak};

/// What the user meant by clicking a token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Intent {
    /// Ctrl-click: open the declaration, or the module for an instance.
    Navigate,
    /// Alt-click: add the recorded signal(s) to the waveform.
    AddToWaveform,
}

/// A source position in a file, zero-based, byte columns.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Location {
    pub file: Utf8PathBuf,
    pub line: u32,
    pub character: u32,
}

/// Bookkeeping for a request in flight.
#[derive(Clone, Debug, PartialEq)]
pub enum Pending {
    Initialize,
    SetBuildFile,
    SemanticTokens(Utf8PathBuf),
    InactiveRanges {
        file: Utf8PathBuf,
        instance: String,
    },
    Hover(Location),
    HoverInstances(Location),
    Definition {
        at: Location,
        intent: Intent,
    },
    Instances {
        at: Location,
        intent: Intent,
    },
    ModuleSymbol {
        definition: String,
        instance: Option<String>,
    },
    /// `textDocument/documentSymbol` of the file that declares `definition`.
    ModuleInFile {
        file: Utf8PathBuf,
        definition: String,
        instance: Option<String>,
    },
}

/// Delivered through [`Message::Slang`].
#[derive(Clone, Debug)]
pub enum Event {
    Response {
        pending: Pending,
        result: Option<Value>,
        error: Option<Value>,
    },
    Notification {
        method: String,
        params: Value,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Starting,
    Initialized,
    Ready,
    Failed(String),
}

/// Hover information gathered from the server for one position.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HoverInfo {
    /// Markdown from `textDocument/hover`, if the server had anything to say.
    pub markdown: Option<String>,
    /// Elaborated hierarchical paths the token denotes, one per instance.
    pub paths: Vec<String>,
    pending: u8,
}

impl HoverInfo {
    pub fn is_ready(&self) -> bool {
        self.pending == 0
    }
}

#[derive(Debug, Default)]
struct Document {
    text: Arc<str>,
    opened: bool,
    tokens: Option<Arc<LineTokens>>,
    tokens_requested: bool,
    inactive: HashMap<String, Option<Arc<Vec<Range>>>>,
    hovers: HashMap<(u32, u32), HoverInfo>,
    diagnostics: Vec<String>,
}

#[derive(Debug)]
struct Activation {
    intent: Intent,
    token: String,
    class: TokenClass,
    definition: Option<Vec<(Utf8PathBuf, u32, u32)>>,
    instances: Option<Vec<String>>,
}

#[derive(Debug)]
struct ClientState {
    phase: Phase,
    legend: Legend,
    documents: HashMap<Utf8PathBuf, Document>,
    activations: HashMap<Location, Activation>,
    messages: Vec<String>,
}

struct Shared {
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, Pending>>,
    sender: Sender<Message>,
    transport: Mutex<Option<Weak<dyn Transport>>>,
}

impl Shared {
    fn deliver(&self, event: Event) {
        if self.sender.send(Message::Slang(event)).is_ok()
            && let Some(context) = crate::EGUI_CONTEXT.read().unwrap().as_ref()
        {
            context.request_repaint();
        }
    }

    fn reply(&self, id: Value, result: Value) {
        let transport = self
            .transport
            .lock()
            .unwrap()
            .as_ref()
            .and_then(Weak::upgrade);
        if let Some(transport) = transport {
            transport.send(&protocol::response(id, result));
        }
    }

    fn on_incoming(&self, incoming: Incoming) {
        match incoming {
            Incoming::Response { id, result, error } => {
                let pending = self.pending.lock().unwrap().remove(&id);
                match pending {
                    Some(pending) => self.deliver(Event::Response {
                        pending,
                        result,
                        error,
                    }),
                    None => tracing::debug!(id, "Unexpected response from slang-server"),
                }
            }
            Incoming::Request { id, method, params } => match method.as_str() {
                "workspace/configuration" => {
                    let count = params
                        .get("items")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len);
                    self.reply(id, Value::Array(vec![Value::Null; count]));
                }
                "window/showDocument" => self.reply(id, json!({"success": true})),
                _ => self.reply(id, Value::Null),
            },
            Incoming::Notification { method, params } => {
                self.deliver(Event::Notification { method, params });
            }
        }
    }
}

/// A running language server (or a replayed session) for the loaded design.
pub struct SlangClient {
    transport: Arc<dyn Transport>,
    shared: Arc<Shared>,
    plan: LaunchPlan,
    build_file: Utf8PathBuf,
    state: Mutex<ClientState>,
}

impl std::fmt::Debug for SlangClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlangClient")
            .field("transport", &self.transport.describe())
            .field("phase", &self.state.lock().unwrap().phase)
            .finish()
    }
}

impl SlangClient {
    /// Starts a session over a transport built by `make_transport`, which receives the
    /// callback that delivers server messages. The `initialize` request is sent at once.
    pub fn start(
        plan: LaunchPlan,
        build_file: Utf8PathBuf,
        sender: Sender<Message>,
        make_transport: impl FnOnce(OnMessage) -> std::io::Result<Arc<dyn Transport>>,
    ) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            sender,
            transport: Mutex::new(None),
        });
        let on_message: OnMessage = {
            let shared = shared.clone();
            Arc::new(move |incoming| shared.on_incoming(incoming))
        };
        let transport = make_transport(on_message)?;
        *shared.transport.lock().unwrap() = Some(Arc::downgrade(&transport));
        let client = Self {
            transport,
            shared,
            plan,
            build_file,
            state: Mutex::new(ClientState {
                phase: Phase::Starting,
                legend: Legend::default(),
                documents: HashMap::new(),
                activations: HashMap::new(),
                messages: Vec::new(),
            }),
        };
        client.request(
            Pending::Initialize,
            "initialize",
            json!({
                "processId": std::process::id(),
                "rootUri": file_uri(client.plan.workspace.as_str()),
                "workspaceFolders": [{"uri": file_uri(client.plan.workspace.as_str()), "name": "design"}],
                "capabilities": {
                    "textDocument": {
                        "definition": {"linkSupport": false},
                        "hover": {"contentFormat": ["markdown", "plaintext"]},
                        "semanticTokens": {"requests": {"full": true}, "tokenTypes": [], "tokenModifiers": [], "formats": ["relative"]},
                    },
                    "experimental": {"inactiveRegions": {"inactiveRegions": true}},
                },
            }),
        );
        Ok(client)
    }

    pub fn plan(&self) -> &LaunchPlan {
        &self.plan
    }

    pub fn phase(&self) -> Phase {
        self.state.lock().unwrap().phase.clone()
    }

    /// One line for the tile header.
    pub fn status(&self) -> String {
        match self.phase() {
            Phase::Starting => format!("starting {}", self.transport.describe()),
            Phase::Initialized => "elaborating design".to_owned(),
            Phase::Ready => format!("slang: {}", self.plan.top),
            Phase::Failed(error) => format!("slang unavailable: {error}"),
        }
    }

    /// Diagnostics the server published for `file`.
    pub fn diagnostics(&self, file: &Utf8Path) -> Vec<String> {
        self.state
            .lock()
            .unwrap()
            .documents
            .get(file)
            .map(|d| d.diagnostics.clone())
            .unwrap_or_default()
    }

    fn request(&self, pending: Pending, method: &str, params: Value) {
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        self.shared.pending.lock().unwrap().insert(id, pending);
        self.transport.send(&protocol::request(id, method, params));
    }

    fn notify(&self, method: &str, params: Value) {
        self.transport.send(&protocol::notification(method, params));
    }

    fn command(&self, pending: Pending, command: &str, argument: Value) {
        self.request(
            pending,
            "workspace/executeCommand",
            json!({"command": command, "arguments": [argument]}),
        );
    }

    /// Registers the text of `file` with the server. Safe to call every frame.
    pub fn open_document(&self, file: &Utf8Path, text: Arc<str>) {
        let mut state = self.state.lock().unwrap();
        let ready = state.phase != Phase::Starting;
        let document = state.documents.entry(file.to_owned()).or_default();
        if document.opened || !ready {
            document.text = text;
            return;
        }
        document.text = text.clone();
        document.opened = true;
        drop(state);
        self.send_did_open(file, &text);
    }

    fn send_did_open(&self, file: &Utf8Path, text: &str) {
        self.notify(
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": file_uri(file.as_str()),
                "languageId": "systemverilog",
                "version": 1,
                "text": text,
            }}),
        );
    }

    /// Semantic tokens of an opened file, requesting them on first use.
    pub fn tokens(&self, file: &Utf8Path) -> Option<Arc<LineTokens>> {
        let mut state = self.state.lock().unwrap();
        if state.phase != Phase::Ready {
            return None;
        }
        let document = state.documents.get_mut(file)?;
        if let Some(tokens) = &document.tokens {
            return Some(tokens.clone());
        }
        if !document.opened || document.tokens_requested {
            return None;
        }
        document.tokens_requested = true;
        drop(state);
        self.request(
            Pending::SemanticTokens(file.to_owned()),
            "textDocument/semanticTokens/full",
            json!({"textDocument": {"uri": file_uri(file.as_str())}}),
        );
        None
    }

    /// Generate blocks of `file` that `instance` does not instantiate.
    pub fn inactive_ranges(&self, file: &Utf8Path, instance: &str) -> Option<Arc<Vec<Range>>> {
        let mut state = self.state.lock().unwrap();
        if state.phase != Phase::Ready {
            return None;
        }
        let document = state.documents.get_mut(file)?;
        if !document.opened {
            return None;
        }
        match document.inactive.get(instance) {
            Some(ranges) => ranges.clone(),
            None => {
                document.inactive.insert(instance.to_owned(), None);
                drop(state);
                self.command(
                    Pending::InactiveRanges {
                        file: file.to_owned(),
                        instance: instance.to_owned(),
                    },
                    "slang.getInactiveGenerateRanges",
                    json!({"textDocument": {"uri": file_uri(file.as_str())}, "instance": instance}),
                );
                None
            }
        }
    }

    /// Hover data for a position, requesting it on first use.
    pub fn hover(&self, at: &Location) -> Option<HoverInfo> {
        let mut state = self.state.lock().unwrap();
        if state.phase != Phase::Ready {
            return None;
        }
        let document = state.documents.get_mut(&at.file)?;
        if !document.opened {
            return None;
        }
        if let Some(info) = document.hovers.get(&(at.line, at.character)) {
            return Some(info.clone());
        }
        document.hovers.insert(
            (at.line, at.character),
            HoverInfo {
                pending: 2,
                ..HoverInfo::default()
            },
        );
        drop(state);
        let position = json!({"textDocument": {"uri": file_uri(at.file.as_str())}, "position": {"line": at.line, "character": at.character}});
        self.request(
            Pending::Hover(at.clone()),
            "textDocument/hover",
            position.clone(),
        );
        self.command(
            Pending::HoverInstances(at.clone()),
            "slang.getInstances",
            position,
        );
        None
    }

    /// Resolves the token at `at` and performs `intent` once the server answers.
    pub fn activate(&self, at: Location, token: String, class: TokenClass, intent: Intent) {
        let mut state = self.state.lock().unwrap();
        if state.phase != Phase::Ready {
            state
                .messages
                .push("slang-server is not ready yet".to_owned());
            return;
        }
        state.activations.insert(
            at.clone(),
            Activation {
                intent,
                token,
                class,
                definition: None,
                instances: None,
            },
        );
        drop(state);
        let position = json!({"textDocument": {"uri": file_uri(at.file.as_str())}, "position": {"line": at.line, "character": at.character}});
        self.request(
            Pending::Definition {
                at: at.clone(),
                intent,
            },
            "textDocument/definition",
            position.clone(),
        );
        self.command(
            Pending::Instances { at, intent },
            "slang.getInstances",
            position,
        );
    }

    /// Requests still waiting for an answer.
    pub fn pending_count(&self) -> usize {
        self.shared.pending.lock().unwrap().len()
    }

    /// Recent one-line notices for the tile, newest last.
    pub fn take_messages(&self) -> Vec<String> {
        std::mem::take(&mut self.state.lock().unwrap().messages)
    }

    /// Applies a server event. `resolve` maps elaborated paths to recorded signals and
    /// design instances; follow-up UI actions are appended to `out`.
    pub fn handle(&self, event: Event, resolve: &dyn Resolver, out: &mut Vec<Message>) {
        match event {
            Event::Notification { method, params } => self.handle_notification(&method, params),
            Event::Response {
                pending,
                result,
                error,
            } => {
                if let Some(error) = &error {
                    let text = error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error");
                    tracing::warn!(?pending, text, "slang-server request failed");
                }
                self.handle_response(
                    pending,
                    result.unwrap_or(Value::Null),
                    error.is_some(),
                    resolve,
                    out,
                );
            }
        }
    }

    fn handle_notification(&self, method: &str, params: Value) {
        match method {
            "surfer/serverExited" => {
                self.state.lock().unwrap().phase = Phase::Failed("server exited".to_owned());
            }
            "textDocument/publishDiagnostics" => {
                let Some(file) = params.get("uri").and_then(Value::as_str).and_then(uri_path)
                else {
                    return;
                };
                let diagnostics: Vec<String> = params
                    .get("diagnostics")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| {
                                let line =
                                    item.get("range")?.get("start")?.get("line")?.as_u64()?;
                                let message = item.get("message")?.as_str()?;
                                Some(format!("{}: {message}", line + 1))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let mut state = self.state.lock().unwrap();
                state
                    .documents
                    .entry(Utf8PathBuf::from(file))
                    .or_default()
                    .diagnostics = diagnostics;
            }
            "window/showMessage" => {
                if let Some(message) = params.get("message").and_then(Value::as_str) {
                    self.state.lock().unwrap().messages.push(message.to_owned());
                }
            }
            _ => {}
        }
    }

    fn handle_response(
        &self,
        pending: Pending,
        result: Value,
        failed: bool,
        resolve: &dyn Resolver,
        out: &mut Vec<Message>,
    ) {
        match pending {
            Pending::Initialize => {
                if failed {
                    self.state.lock().unwrap().phase =
                        Phase::Failed("initialize rejected".to_owned());
                    return;
                }
                let legend = result
                    .get("capabilities")
                    .and_then(Legend::from_capabilities)
                    .unwrap_or_default();
                let pending_docs: Vec<_> = {
                    let mut state = self.state.lock().unwrap();
                    if legend.is_empty() {
                        state
                            .messages
                            .push("server has no semantic tokens; update slang-server".to_owned());
                    }
                    state.legend = legend;
                    state.phase = Phase::Initialized;
                    state
                        .documents
                        .iter_mut()
                        .filter(|(_, d)| !d.opened)
                        .map(|(file, d)| {
                            d.opened = true;
                            (file.clone(), d.text.clone())
                        })
                        .collect()
                };
                self.notify("initialized", json!({}));
                for (file, text) in pending_docs {
                    self.send_did_open(&file, &text);
                }
                self.command(
                    Pending::SetBuildFile,
                    "slang.setBuildFile",
                    Value::String(self.build_file.to_string()),
                );
            }
            Pending::SetBuildFile => {
                let mut state = self.state.lock().unwrap();
                state.phase = if failed {
                    Phase::Failed("build file rejected".to_owned())
                } else {
                    tracing::info!(top = %self.plan.top, "slang-server elaborated the design");
                    Phase::Ready
                };
            }
            Pending::SemanticTokens(file) => {
                let data: Vec<u32> = result
                    .get("data")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_u64)
                            .map(|v| v as u32)
                            .collect()
                    })
                    .unwrap_or_default();
                let mut state = self.state.lock().unwrap();
                let tokens = LineTokens::decode(&data, &state.legend);
                if let Some(document) = state.documents.get_mut(&file) {
                    document.tokens = Some(Arc::new(tokens));
                }
            }
            Pending::InactiveRanges { file, instance } => {
                let ranges: Vec<Range> = serde_json::from_value(result).unwrap_or_default();
                let mut state = self.state.lock().unwrap();
                if let Some(document) = state.documents.get_mut(&file) {
                    document.inactive.insert(instance, Some(Arc::new(ranges)));
                }
            }
            Pending::Hover(at) => {
                let markdown = result
                    .get("contents")
                    .and_then(|c| c.get("value").or(Some(c)))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let mut state = self.state.lock().unwrap();
                if let Some(info) = state
                    .documents
                    .get_mut(&at.file)
                    .and_then(|d| d.hovers.get_mut(&(at.line, at.character)))
                {
                    info.markdown = markdown;
                    info.pending = info.pending.saturating_sub(1);
                }
            }
            Pending::HoverInstances(at) => {
                let paths = string_list(&result);
                let mut state = self.state.lock().unwrap();
                if let Some(info) = state
                    .documents
                    .get_mut(&at.file)
                    .and_then(|d| d.hovers.get_mut(&(at.line, at.character)))
                {
                    info.paths = paths;
                    info.pending = info.pending.saturating_sub(1);
                }
            }
            Pending::Definition { at, .. } => {
                let locations = location_list(&result);
                self.advance_activation(&at, Some(locations), None, resolve, out);
            }
            Pending::Instances { at, .. } => {
                let paths = string_list(&result);
                self.advance_activation(&at, None, Some(paths), resolve, out);
            }
            Pending::ModuleSymbol {
                definition,
                instance,
            } => {
                let symbol = result
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|item| item.get("name").and_then(Value::as_str) == Some(&definition))
                    .cloned();
                let Some(symbol) = symbol else {
                    self.state
                        .lock()
                        .unwrap()
                        .messages
                        .push(format!("module {definition} not found in the design"));
                    return;
                };
                if let Some((file, line, column)) =
                    symbol.get("location").and_then(location_from_value)
                {
                    out.push(Message::OpenSource {
                        file,
                        line: line + 1,
                        column: column + 1,
                        instance,
                    });
                    return;
                }
                // Workspace symbols name the file only; the declaration line comes from
                // the document's own symbol tree.
                let Some(file) = symbol
                    .get("location")
                    .and_then(|l| l.get("uri"))
                    .and_then(Value::as_str)
                    .and_then(uri_path)
                    .map(Utf8PathBuf::from)
                else {
                    return;
                };
                self.request(
                    Pending::ModuleInFile {
                        file: file.clone(),
                        definition,
                        instance,
                    },
                    "textDocument/documentSymbol",
                    json!({"textDocument": {"uri": file_uri(file.as_str())}}),
                );
            }
            Pending::ModuleInFile {
                file,
                definition,
                instance,
            } => {
                let start = result
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|item| item.get("name").and_then(Value::as_str) == Some(&definition))
                    .and_then(|item| item.get("selectionRange").or_else(|| item.get("range")))
                    .and_then(|range| range.get("start"))
                    .and_then(|start| {
                        Some((
                            start.get("line")?.as_u64()? as u32,
                            start.get("character")?.as_u64()? as u32,
                        ))
                    });
                match start {
                    Some((line, column)) => out.push(Message::OpenSource {
                        file,
                        line: line + 1,
                        column: column + 1,
                        instance,
                    }),
                    None => self
                        .state
                        .lock()
                        .unwrap()
                        .messages
                        .push(format!("module {definition} not declared in {file}")),
                }
            }
        }
    }

    fn advance_activation(
        &self,
        at: &Location,
        definition: Option<Vec<(Utf8PathBuf, u32, u32)>>,
        instances: Option<Vec<String>>,
        resolve: &dyn Resolver,
        out: &mut Vec<Message>,
    ) {
        let mut state = self.state.lock().unwrap();
        let Some(activation) = state.activations.get_mut(at) else {
            return;
        };
        if definition.is_some() {
            activation.definition = definition;
        }
        if instances.is_some() {
            activation.instances = instances;
        }
        if activation.definition.is_none() || activation.instances.is_none() {
            return;
        }
        let activation = state.activations.remove(at).unwrap();
        drop(state);
        let paths = activation.instances.unwrap_or_default();
        let owner = resolve.owner_instance(&paths);
        match activation.intent {
            Intent::AddToWaveform => {
                let signals = resolve.recorded_signals(&paths);
                if signals.is_empty() {
                    self.state
                        .lock()
                        .unwrap()
                        .messages
                        .push(if paths.is_empty() {
                            "no design symbol at this token".to_owned()
                        } else {
                            format!("{} is not recorded in the trace", paths.join(", "))
                        });
                } else {
                    out.push(Message::AddVariables(
                        signals
                            .iter()
                            .map(|path| {
                                crate::wave_container::VariableRefExt::from_hierarchy_string(path)
                            })
                            .collect(),
                    ));
                }
            }
            Intent::Navigate => {
                let definition = activation.definition.unwrap_or_default();
                // The declaration itself was clicked when the only definition starts on
                // the clicked line at or before the clicked column.
                let is_self = definition.len() == 1
                    && definition[0].0 == at.file
                    && definition[0].1 == at.line
                    && definition[0].2 <= at.character;
                if matches!(activation.class, TokenClass::Instance) && is_self {
                    // Ctrl-click on an instance name opens its module definition.
                    match resolve.instance_definition(&paths) {
                        Some((instance, definition)) => self.request(
                            Pending::ModuleSymbol {
                                definition: definition.clone(),
                                instance: Some(instance),
                            },
                            "workspace/symbol",
                            json!({"query": definition}),
                        ),
                        None => self
                            .state
                            .lock()
                            .unwrap()
                            .messages
                            .push("instance is not part of the elaborated design".to_owned()),
                    }
                    return;
                }
                // A module or interface name has no owner; view it in its first instance.
                let instance = owner.or_else(|| {
                    matches!(activation.class, TokenClass::Module | TokenClass::Interface)
                        .then(|| {
                            resolve
                                .instances_of_module(&activation.token)
                                .into_iter()
                                .next()
                        })
                        .flatten()
                });
                match definition.into_iter().next() {
                    Some((file, line, column)) => out.push(Message::OpenSource {
                        file,
                        line: line + 1,
                        column: column + 1,
                        instance,
                    }),
                    None => self
                        .state
                        .lock()
                        .unwrap()
                        .messages
                        .push("no declaration found for this token".to_owned()),
                }
            }
        }
    }
}

/// Design knowledge the client needs to turn elaborated paths into UI actions.
pub trait Resolver {
    /// Recorded waveform paths for elaborated symbol paths (empty when unrecorded).
    fn recorded_signals(&self, paths: &[String]) -> Vec<String>;
    /// The innermost design instance owning the first resolvable path.
    fn owner_instance(&self, paths: &[String]) -> Option<String>;
    /// `(instance path, module name)` for the first path naming an instance.
    fn instance_definition(&self, paths: &[String]) -> Option<(String, String)>;
    /// Elaborated instances of the module or interface called `name`, in design order.
    fn instances_of_module(&self, name: &str) -> Vec<String>;
}

/// A resolver that knows nothing, for sessions without a design database.
pub struct NoDesign;

impl Resolver for NoDesign {
    fn recorded_signals(&self, _: &[String]) -> Vec<String> {
        Vec::new()
    }
    fn owner_instance(&self, _: &[String]) -> Option<String> {
        None
    }
    fn instance_definition(&self, _: &[String]) -> Option<(String, String)> {
        None
    }
    fn instances_of_module(&self, _: &str) -> Vec<String> {
        Vec::new()
    }
}

fn string_list(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn location_from_value(value: &Value) -> Option<(Utf8PathBuf, u32, u32)> {
    let uri = value
        .get("uri")
        .or_else(|| value.get("targetUri"))?
        .as_str()?;
    let range = value
        .get("range")
        .or_else(|| value.get("targetSelectionRange"))?;
    let start = range.get("start")?;
    Some((
        Utf8PathBuf::from(uri_path(uri)?),
        start.get("line")?.as_u64()? as u32,
        start.get("character")?.as_u64()? as u32,
    ))
}

fn location_list(value: &Value) -> Vec<(Utf8PathBuf, u32, u32)> {
    match value {
        Value::Array(items) => items.iter().filter_map(location_from_value).collect(),
        other => location_from_value(other).into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slang::transport::{Exchange, ReplayScript, ReplayTransport};
    use std::sync::mpsc;

    fn plan() -> LaunchPlan {
        LaunchPlan {
            workspace: Utf8PathBuf::from("/design"),
            build_file: "--top top\n/design/top.sv\n".into(),
            files: vec![Utf8PathBuf::from("/design/top.sv")],
            top: "top".into(),
        }
    }

    fn init_exchange() -> Exchange {
        Exchange {
            method: "initialize".into(),
            params: Value::Null,
            result: json!({"capabilities": {"semanticTokensProvider": {"legend": {
                "tokenTypes": ["keyword", "variable", "instance"],
                "tokenModifiers": ["declaration", "input"]}}}}),
            error: None,
        }
    }

    /// Builds a client over a replay script; `initialize` params are matched loosely by
    /// rewriting the recorded exchange to whatever the client sent.
    fn client(mut script: ReplayScript) -> (SlangClient, mpsc::Receiver<Message>) {
        let (sender, receiver) = mpsc::channel();
        let init = init_exchange();
        script.exchanges.push(Exchange {
            params: Value::Null,
            ..init.clone()
        });
        let client = SlangClient::start(plan(), Utf8PathBuf::from("/tmp/x.f"), sender, |deliver| {
            // Replace the initialize params with a wildcard by intercepting the send.
            struct Loose(ReplayTransport);
            impl Transport for Loose {
                fn send(&self, message: &Value) {
                    let mut message = message.clone();
                    if message.get("method").and_then(Value::as_str) == Some("initialize") {
                        message["params"] = Value::Null;
                    }
                    self.0.send(&message);
                }
                fn describe(&self) -> String {
                    "loose".into()
                }
            }
            Ok(Arc::new(Loose(ReplayTransport::new(
                script,
                vec![],
                deliver,
            ))))
        })
        .unwrap();
        (client, receiver)
    }

    fn pump(client: &SlangClient, receiver: &mpsc::Receiver<Message>, out: &mut Vec<Message>) {
        while let Ok(message) = receiver.try_recv() {
            if let Message::Slang(event) = message {
                client.handle(event, &NoDesign, out);
            }
        }
    }

    #[test]
    fn startup_sets_the_build_file_and_becomes_ready() {
        let script = ReplayScript {
            notifications: vec![],
            exchanges: vec![Exchange {
                method: "workspace/executeCommand".into(),
                params: json!({"command": "slang.setBuildFile", "arguments": ["/tmp/x.f"]}),
                result: Value::Null,
                error: None,
            }],
        };
        let (client, receiver) = client(script);
        assert_eq!(client.phase(), Phase::Starting);
        let mut out = Vec::new();
        pump(&client, &receiver, &mut out);
        assert_eq!(client.phase(), Phase::Ready);
        assert!(client.status().contains("top"));
    }

    #[test]
    fn tokens_are_requested_once_and_decoded_with_the_legend() {
        let uri = "file:///design/top.sv";
        let script = ReplayScript {
            notifications: vec![],
            exchanges: vec![
                Exchange {
                    method: "workspace/executeCommand".into(),
                    params: json!({"command": "slang.setBuildFile", "arguments": ["/tmp/x.f"]}),
                    result: Value::Null,
                    error: None,
                },
                Exchange {
                    method: "textDocument/semanticTokens/full".into(),
                    params: json!({"textDocument": {"uri": uri}}),
                    result: json!({"data": [0, 0, 6, 0, 0, 0, 7, 3, 1, 3]}),
                    error: None,
                },
            ],
        };
        let (client, receiver) = client(script);
        let file = Utf8Path::new("/design/top.sv");
        client.open_document(file, Arc::from("module top;"));
        let mut out = Vec::new();
        pump(&client, &receiver, &mut out);
        assert!(
            client.tokens(file).is_none(),
            "first call issues the request"
        );
        pump(&client, &receiver, &mut out);
        let tokens = client.tokens(file).expect("tokens cached");
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens.at(0, 8).unwrap().class, TokenClass::Variable);
        assert!(
            tokens
                .at(0, 8)
                .unwrap()
                .modifiers
                .contains(super::super::tokens::Modifiers::INPUT)
        );
    }

    #[test]
    fn alt_click_adds_recorded_signals_through_the_resolver() {
        let uri = "file:///design/top.sv";
        let position =
            json!({"textDocument": {"uri": uri}, "position": {"line": 3, "character": 9}});
        let script = ReplayScript {
            notifications: vec![],
            exchanges: vec![
                Exchange {
                    method: "workspace/executeCommand".into(),
                    params: json!({"command": "slang.setBuildFile", "arguments": ["/tmp/x.f"]}),
                    result: Value::Null,
                    error: None,
                },
                Exchange {
                    method: "textDocument/definition".into(),
                    params: position.clone(),
                    result: json!([{"uri": uri, "range": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 5}}}]),
                    error: None,
                },
                Exchange {
                    method: "workspace/executeCommand".into(),
                    params: json!({"command": "slang.getInstances", "arguments": [position]}),
                    result: json!(["top.u0.q", "top.u1.q"]),
                    error: None,
                },
            ],
        };
        struct Design;
        impl Resolver for Design {
            fn recorded_signals(&self, paths: &[String]) -> Vec<String> {
                paths.iter().map(|p| format!("TOP.{p}")).collect()
            }
            fn owner_instance(&self, paths: &[String]) -> Option<String> {
                paths
                    .first()
                    .map(|p| p.rsplit_once('.').unwrap().0.to_owned())
            }
            fn instance_definition(&self, _: &[String]) -> Option<(String, String)> {
                None
            }
            fn instances_of_module(&self, _: &str) -> Vec<String> {
                Vec::new()
            }
        }
        let (client, receiver) = client(script);
        let file = Utf8Path::new("/design/top.sv");
        client.open_document(file, Arc::from("module top;"));
        let mut out = Vec::new();
        pump(&client, &receiver, &mut out);
        let at = Location {
            file: file.to_owned(),
            line: 3,
            character: 9,
        };
        client.activate(
            at.clone(),
            "q".into(),
            TokenClass::Variable,
            Intent::AddToWaveform,
        );
        while let Ok(message) = receiver.try_recv() {
            if let Message::Slang(event) = message {
                client.handle(event, &Design, &mut out);
            }
        }
        assert_eq!(out.len(), 1);
        let Message::AddVariables(vars) = &out[0] else {
            panic!("expected AddVariables, got {:?}", out[0]);
        };
        assert_eq!(vars.len(), 2);

        // Navigation to the declaration keeps the owner instance from the resolver.
        client.activate(at, "q".into(), TokenClass::Variable, Intent::Navigate);
        out.clear();
        while let Ok(message) = receiver.try_recv() {
            if let Message::Slang(event) = message {
                client.handle(event, &Design, &mut out);
            }
        }
        assert!(matches!(
            &out[0],
            Message::OpenSource { file, line: 2, column: 5, instance: Some(owner) }
                if file.as_str() == "/design/top.sv" && owner == "top.u0"
        ));
    }
}
