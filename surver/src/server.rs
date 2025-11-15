//! Handling of external communication in Surver.
use bincode::Options;
use chrono;
use eyre::{Context, Result, anyhow, bail};
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::collections::HashMap;
use std::fs;
use std::iter::repeat_with;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tracing::{error, info, warn};
use wellen::{
    CompressedSignal, CompressedTimeTable, FileFormat, Hierarchy, Signal, SignalRef, Time, viewers,
};

use crate::{
    BINCODE_OPTIONS, HTTP_SERVER_KEY, HTTP_SERVER_VALUE_SURFER, SURFER_VERSION, Status,
    WELLEN_SURFER_DEFAULT_OPTIONS, WELLEN_VERSION, X_SURFER_VERSION, X_WELLEN_VERSION,
};

// Constants
const ERROR_RELOAD_TOO_FREQUENT: &[u8] = b"{\"error\":\"Reload too frequent\"}";
const ERROR_FILE_NOT_FOUND: &[u8] = b"{\"error\":\"File not found\"}";
const INFO_FILE_UNCHANGED: &[u8] = b"{\"info\":\"File unchanged\"}";

struct ReadOnly {
    url: String,
    token: String,
    filename: String,
    hierarchy: Hierarchy,
    file_format: FileFormat,
    header_len: u64,
    body_len: u64,
    body_progress: Arc<AtomicU64>,
    reload_guard: Duration,
}

#[derive(Default)]
struct State {
    timetable: Vec<Time>,
    signals: HashMap<SignalRef, Signal>,
    reloading: bool,
    last_reload_ok: bool,
    last_reload_time: Option<Instant>,
    last_reload_request: Option<Instant>,
    last_file_mtime: Option<SystemTime>,
    notify: Arc<Notify>,
}

impl State {
    pub fn modification_time_string(&self) -> String {
        if let Some(mtime) = self.last_file_mtime {
            let dur = mtime
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            return chrono::DateTime::<chrono::Utc>::from_timestamp(
                dur.as_secs() as i64,
                dur.subsec_nanos(),
            )
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "Incorrect timestamp".to_string());
        }
        "unknown".to_string()
    }

    pub fn reload_time_string(&self) -> String {
        if let Some(time) = self.last_reload_time {
            return format!("{:?} ago", time.elapsed());
        }
        "never".to_string()
    }
}

enum LoaderMessage {
    SignalRequest(SignalRequest),
    Reload,
}

type SignalRequest = Vec<SignalRef>;

fn get_info_page(shared: Arc<ReadOnly>, state: Arc<RwLock<State>>) -> String {
    let bytes_loaded = shared.body_progress.load(Ordering::SeqCst);

    let progress = if bytes_loaded == shared.body_len {
        format!(
            "{} loaded",
            bytesize::ByteSize::b(shared.body_len + shared.header_len)
        )
    } else {
        format!(
            "{} / {}",
            bytesize::ByteSize::b(bytes_loaded + shared.header_len),
            bytesize::ByteSize::b(shared.body_len + shared.header_len)
        )
    };

    let state_guard = state.read().expect("State lock poisoned in get_info_page");
    let mtime_str = state_guard.modification_time_string();
    let reload_str = state_guard.reload_time_string();
    drop(state_guard);

    format!(
        r#"
    <!DOCTYPE html><html lang="en">
    <head><title>Surver - Surfer Remote Server</title></head><body>
    <h1>Surver - Surfer Remote Server</h1>
    <b>To connect, run:</b> <code>surfer {}</code><br>
    <b>Wellen version:</b> {WELLEN_VERSION}<br>
    <b>Surfer version:</b> {SURFER_VERSION}<br>
    <b>Filename:</b> {}<br>
    <b>Progress:</b> {progress}<br>
    <b>File last modified:</b> {}<br>
    <b>Last successful (re)load:</b> {}<br>
    </body></html>
    "#,
        shared.url, shared.filename, mtime_str, reload_str
    )
}

fn get_hierarchy(shared: Arc<ReadOnly>) -> Result<Vec<u8>> {
    let mut raw = BINCODE_OPTIONS.serialize(&shared.file_format)?;
    let mut raw2 = BINCODE_OPTIONS.serialize(&shared.hierarchy)?;
    raw.append(&mut raw2);
    let compressed = lz4_flex::compress_prepend_size(&raw);
    info!(
        "Sending hierarchy. {} raw, {} compressed.",
        bytesize::ByteSize::b(raw.len() as u64),
        bytesize::ByteSize::b(compressed.len() as u64)
    );
    Ok(compressed)
}

async fn get_timetable(state: Arc<RwLock<State>>) -> Result<Vec<u8>> {
    // poll to see when the time table is available
    #[allow(unused_assignments)]
    let mut table = vec![];
    loop {
        {
            let state = state.read().unwrap();
            if !state.timetable.is_empty() {
                table = state.timetable.clone();
                break;
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
    let raw_size = table.len() * std::mem::size_of::<Time>();
    let compressed = BINCODE_OPTIONS.serialize(&CompressedTimeTable::compress(&table))?;
    info!(
        "Sending timetable. {} raw, {} compressed.",
        bytesize::ByteSize::b(raw_size as u64),
        bytesize::ByteSize::b(compressed.len() as u64)
    );
    Ok(compressed)
}

fn get_status(shared: Arc<ReadOnly>, state: Arc<RwLock<State>>) -> Result<Vec<u8>> {
    let state = state.read().expect("State lock poisoned in get_status");
    let status = Status {
        bytes: shared.body_len + shared.header_len,
        bytes_loaded: shared.body_progress.load(Ordering::SeqCst) + shared.header_len,
        filename: shared.filename.clone(),
        wellen_version: WELLEN_VERSION.to_string(),
        surfer_version: SURFER_VERSION.to_string(),
        file_format: shared.file_format,
        reloading: state.reloading,
        last_reload_ok: state.last_reload_ok,
        last_reload_time: state.last_reload_time.map(|t| t.elapsed().as_secs()),
    };
    Ok(serde_json::to_vec(&status)?)
}

async fn get_signals(
    state: Arc<RwLock<State>>,
    tx: Sender<LoaderMessage>,
    id_strings: &[&str],
) -> Result<Vec<u8>> {
    let mut ids = Vec::with_capacity(id_strings.len());
    for id in id_strings.iter() {
        let index = id.parse::<u64>()? as usize;
        let signal_ref = SignalRef::from_index(index)
            .ok_or_else(|| anyhow!("Invalid signal index: {}", index))?;
        ids.push(signal_ref);
    }

    if ids.is_empty() {
        return Ok(vec![]);
    }
    let num_ids = ids.len();

    // send request to background thread
    tx.send(LoaderMessage::SignalRequest(ids.clone()))?;

    let notify = {
        let state = state.read().expect("State lock poisoned in get_signals");
        state.notify.clone()
    };

    // Wait for all signals to be loaded
    let mut data = vec![];
    leb128::write::unsigned(&mut data, num_ids as u64)?;
    let mut raw_size = 0;
    loop {
        {
            let state = state.read().expect("State lock poisoned in get_signals");
            if ids.iter().all(|id| state.signals.contains_key(id)) {
                for id in ids {
                    let signal = &state.signals[&id];
                    raw_size += BINCODE_OPTIONS.serialize(signal)?.len();
                    let comp = CompressedSignal::compress(signal);
                    data.append(&mut BINCODE_OPTIONS.serialize(&comp)?);
                }
                break;
            }
        };
        // Wait for notification that signals have been loaded
        notify.notified().await;
    }
    info!(
        "Sending {} signals. {} raw, {} compressed.",
        num_ids,
        bytesize::ByteSize::b(raw_size as u64),
        bytesize::ByteSize::b(data.len() as u64)
    );
    Ok(data)
}

const CONTENT_TYPE: &str = "Content-Type";
const JSON_MIME: &str = "application/json";
const OCTET_MIME: &str = "application/octet-stream";
const HTML_MIME: &str = "text/html; charset=utf-8";

trait DefaultHeader {
    fn default_header(self) -> Self;
}

impl DefaultHeader for hyper::http::response::Builder {
    fn default_header(self) -> Self {
        self.header(HTTP_SERVER_KEY, HTTP_SERVER_VALUE_SURFER)
            .header(X_WELLEN_VERSION, WELLEN_VERSION)
            .header(X_SURFER_VERSION, SURFER_VERSION)
            .header("Cache-Control", "no-cache")
    }
}

async fn handle_cmd(
    state: Arc<RwLock<State>>,
    shared: Arc<ReadOnly>,
    tx: Sender<LoaderMessage>,
    cmd: &str,
    args: &[&str],
) -> Result<Response<Full<Bytes>>> {
    let response = match (cmd, args) {
        ("get_status", []) => {
            let body = get_status(shared, state.clone())?;
            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, JSON_MIME)
                .default_header()
                .body(Full::from(body))
        }
        ("get_hierarchy", []) => {
            let body = get_hierarchy(shared)?;
            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, OCTET_MIME)
                .default_header()
                .body(Full::from(body))
        }
        ("get_time_table", []) => {
            let body = get_timetable(state).await?;
            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, OCTET_MIME)
                .default_header()
                .body(Full::from(body))
        }
        ("get_signals", id_strings) => {
            let body = get_signals(state, tx, id_strings).await?;
            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, OCTET_MIME)
                .default_header()
                .body(Full::from(body))
        }
        ("reload", []) => {
            let mut state_guard = state.write().expect("State lock poisoned in reload");
            let now = Instant::now();
            if let Some(last) = state_guard.last_reload_request {
                if now.duration_since(last) < shared.reload_guard {
                    drop(state_guard);
                    return Ok(Response::builder()
                        .status(StatusCode::TOO_MANY_REQUESTS)
                        .header(CONTENT_TYPE, JSON_MIME)
                        .default_header()
                        .body(Full::from(ERROR_RELOAD_TOO_FREQUENT.to_vec()))?);
                }
            }
            // Check file existence, size, and mtime
            let meta = match fs::metadata(&shared.filename) {
                Ok(m) => m,
                Err(_) => {
                    drop(state_guard);
                    return Ok(Response::builder()
                        .status(StatusCode::NOT_FOUND)
                        .header(CONTENT_TYPE, JSON_MIME)
                        .default_header()
                        .body(Full::from(ERROR_FILE_NOT_FOUND.to_vec()))?);
                }
            };
            let mtime = meta
                .modified()
                .unwrap_or_else(|_| std::time::SystemTime::UNIX_EPOCH);
            // Should probably look at file lengths as well for extra safety, but they are not updated correctly at the moment
            let unchanged =
                state_guard.last_file_mtime == Some(mtime) && state_guard.last_reload_ok;
            if unchanged {
                drop(state_guard);
                return Ok(Response::builder()
                    .status(StatusCode::NOT_MODIFIED)
                    .header(CONTENT_TYPE, JSON_MIME)
                    .default_header()
                    .body(Full::from(INFO_FILE_UNCHANGED.to_vec()))?);
            }
            state_guard.last_file_mtime = Some(mtime);
            info!(
                "File modification time updated to {}",
                state_guard.modification_time_string()
            );
            state_guard.last_reload_request = Some(now);
            state_guard.reloading = true;
            state_guard.last_reload_ok = false;
            drop(state_guard);
            info!("Reload requested");
            tx.send(LoaderMessage::Reload)?;
            let body = get_status(shared, state.clone())?;
            Response::builder()
                .status(StatusCode::ACCEPTED)
                .header(CONTENT_TYPE, JSON_MIME)
                .default_header()
                .body(Full::from(body))
        }
        _ => {
            // unknown command or unexpected number of arguments
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(CONTENT_TYPE, OCTET_MIME)
                .default_header()
                .body(Full::from(vec![]))
        }
    };
    Ok(response?)
}

async fn handle(
    state: Arc<RwLock<State>>,
    shared: Arc<ReadOnly>,
    tx: Sender<LoaderMessage>,
    req: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>> {
    // check to see if the correct token was received
    let path_parts = req.uri().path().split('/').skip(1).collect::<Vec<_>>();

    // check token
    if let Some(provided_token) = path_parts.first() {
        if *provided_token != shared.token {
            warn!(
                "Received request with invalid token: {provided_token} != {}\n{:?}",
                shared.token,
                req.uri()
            );
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(CONTENT_TYPE, OCTET_MIME)
                .default_header()
                .body(Full::from(vec![]))?);
        }
    } else {
        // no token
        warn!("Received request with no token: {:?}", req.uri());
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(CONTENT_TYPE, OCTET_MIME)
            .default_header()
            .body(Full::from(vec![]))?);
    }

    // check command
    let response = if let Some(cmd) = path_parts.get(1) {
        handle_cmd(state, shared, tx, cmd, &path_parts[2..]).await?
    } else {
        // valid token, but no command => return info
        let body = Full::from(get_info_page(shared, state));
        Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, HTML_MIME)
            .default_header()
            .body(body)?
    };

    Ok(response)
}

const MIN_TOKEN_LEN: usize = 8;
const RAND_TOKEN_LEN: usize = 24;

pub type ServerStartedFlag = Arc<std::sync::atomic::AtomicBool>;

pub async fn server_main(
    port: u16,
    bind_address: String,
    token: Option<String>,
    filename: String,
    started: Option<ServerStartedFlag>,
    reload_guard: u64,
) -> Result<()> {
    // if no token was provided, we generate one
    let token = token.unwrap_or_else(|| {
        // generate a random ASCII token
        repeat_with(fastrand::alphanumeric)
            .take(RAND_TOKEN_LEN)
            .collect()
    });

    if token.len() < MIN_TOKEN_LEN {
        bail!("Token `{token}` is too short. At least {MIN_TOKEN_LEN} characters are required!");
    }

    // load file
    let start_read_header = web_time::Instant::now();
    let header_result =
        wellen::viewers::read_header_from_file(filename.clone(), &WELLEN_SURFER_DEFAULT_OPTIONS)
            .map_err(|e| anyhow!("{e:?}"))
            .with_context(|| format!("Failed to parse wave file: {filename}"))?;
    info!(
        "Loaded header of {filename} in {:?}",
        start_read_header.elapsed()
    );
    let ip_addr: std::net::IpAddr = bind_address
        .parse()
        .with_context(|| format!("Invalid bind address: {bind_address}"))?;
    let addr = SocketAddr::new(ip_addr, port);

    // immutable read-only data
    let url = format!("http://{addr}/{token}");
    let url_copy = url.clone();
    let token_copy = token.clone();
    let shared = Arc::new(ReadOnly {
        url,
        token,
        filename,
        hierarchy: header_result.hierarchy,
        file_format: header_result.file_format,
        header_len: 0, // FIXME: get value from wellen
        body_len: header_result.body_len,
        body_progress: Arc::new(AtomicU64::new(0)),
        reload_guard: Duration::from_secs(reload_guard),
    });
    // state can be written by the loading thread
    let state = Arc::new(RwLock::new(State::default()));
    // channel to communicate with loader
    let (tx, rx) = std::sync::mpsc::channel::<LoaderMessage>();
    // start work thread
    let shared_2 = shared.clone();
    let state_2 = state.clone();
    std::thread::spawn(move || loader(shared_2, header_result.body, state_2, rx));

    if bind_address != "127.0.0.1" {
        warn!("Server is binding to {bind_address} instead of 127.0.0.1 (localhost)");
        warn!("This may make the server accessible from external networks");
        warn!("Surver traffic is unencrypted and unauthenticated - use with caution!");
    }

    // print out status
    info!("Starting server on {addr}. To use:");
    info!("1. Setup an ssh tunnel: -L {port}:localhost:{port}");
    let hostname = whoami::fallible::hostname();
    if let Ok(hostname) = hostname.as_ref() {
        let username = whoami::username();
        info!(
            "   The correct command may be: ssh -L {port}:localhost:{port} {username}@{hostname} "
        );
    }

    info!("2. Start Surfer: surfer {url_copy} ");
    if let Ok(hostname) = hostname {
        let hosturl = format!("http://{hostname}:{port}/{token_copy}");
        info!("or, if the host is directly accessible:");
        info!("1. Start Surfer: surfer {hosturl} ");
    }
    // create listener and serve it
    let listener = TcpListener::bind(&addr).await?;

    // we have started the server
    if let Some(started) = started {
        started.store(true, Ordering::SeqCst);
    }

    // main server loop
    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);

        let state = state.clone();
        let shared = shared.clone();
        let tx = tx.clone();
        tokio::task::spawn(async move {
            let service =
                service_fn(move |req| handle(state.clone(), shared.clone(), tx.clone(), req));
            if let Err(e) = http1::Builder::new().serve_connection(io, service).await {
                error!("server error: {e}");
            }
        });
    }
}

/// Thread that loads the body and signals.
fn loader(
    shared: Arc<ReadOnly>,
    mut body_cont: viewers::ReadBodyContinuation<std::io::BufReader<std::fs::File>>,
    state: Arc<RwLock<State>>,
    rx: std::sync::mpsc::Receiver<LoaderMessage>,
) -> Result<()> {
    loop {
        // load the body of the file
        let start_load_body = web_time::Instant::now();
        let body_result = viewers::read_body(
            body_cont,
            &shared.hierarchy,
            Some(shared.body_progress.clone()),
        )
        .map_err(|e| anyhow!("{e:?}"))
        .with_context(|| format!("Failed to parse body of wave file: {}", shared.filename))?;
        info!("Loaded body in {:?}", start_load_body.elapsed());

        // update state with body results
        {
            let mut state = state
                .write()
                .expect("State lock poisoned in loader after body load");
            state.timetable = body_result.time_table;
            state.signals.clear(); // Clear old signals on reload
            if let Ok(meta) = fs::metadata(&shared.filename) {
                state.last_file_mtime = Some(meta.modified()?);
                info!(
                    "File modification time set to {}",
                    state.modification_time_string()
                );
            }
            state.last_reload_time = Some(Instant::now());
            state.reloading = false;
            state.last_reload_ok = true;
            state.notify.notify_waiters();
        }
        // source is private, only owned by us
        let mut source = body_result.source;

        // process requests for signals to be loaded
        loop {
            let msg = rx.recv()?;

            match msg {
                LoaderMessage::SignalRequest(ids) => {
                    // make sure that we do not load signals that have already been loaded
                    let mut filtered_ids = {
                        let state_lock = state
                            .read()
                            .expect("State lock poisoned in loader signal request");
                        ids.iter()
                            .filter(|id| !state_lock.signals.contains_key(id))
                            .cloned()
                            .collect::<Vec<_>>()
                    };

                    // check if there is anything left to do
                    if filtered_ids.is_empty() {
                        continue;
                    }

                    // load signals without holding the lock
                    filtered_ids.sort();
                    filtered_ids.dedup();
                    let result = source.load_signals(&filtered_ids, &shared.hierarchy, true);

                    // store signals
                    {
                        let mut state = state
                            .write()
                            .expect("State lock poisoned in loader when storing signals");
                        for (id, signal) in result {
                            state.signals.insert(id, signal);
                        }
                        state.notify.notify_waiters();
                    }
                }
                LoaderMessage::Reload => {
                    info!("Reloading waveform file: {}", shared.filename);
                    // Reset progress counter
                    shared.body_progress.store(0, Ordering::SeqCst);

                    // Re-read header to get new body continuation
                    let header_result = wellen::viewers::read_header_from_file(
                        shared.filename.clone(),
                        &WELLEN_SURFER_DEFAULT_OPTIONS,
                    )
                    .map_err(|e| anyhow!("{e:?}"))
                    .with_context(|| format!("Failed to reload wave file: {}", shared.filename))?;

                    body_cont = header_result.body;
                    break; // Break inner loop to reload the body
                }
            }
        }
    }
}
