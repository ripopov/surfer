//! Handling of external communication in Surver.
use bincode::Options;
use eyre::{Result, WrapErr as _, anyhow, bail};
use ftr_parser::types::{BlockStatus, FTR, StreamId};
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
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Instant, SystemTime};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tracing::{error, info, warn};
use wellen::{
    CompressedSignal, CompressedTimeTable, FileFormat, Hierarchy, Signal, SignalRef, Time, viewers,
};

use crate::{
    BINCODE_OPTIONS, HTTP_SERVER_KEY, HTTP_SERVER_VALUE_SURFER, SURFER_VERSION, SurverFileInfo,
    SurverFileKind, SurverStatus, TRANSACTION_DICTIONARY_PAGE_RECORDS,
    TRANSACTION_PAGE_PROTOCOL_VERSION, TRANSACTION_RELATION_PAGE_RECORDS,
    TransactionDictionaryPage, TransactionManifest, TransactionPageCapability,
    TransactionRecordPage, TransactionRelationPage, WELLEN_SURFER_DEFAULT_OPTIONS, WELLEN_VERSION,
    X_SURFER_VERSION, X_WELLEN_VERSION, modification_time_string,
};

struct ReadOnly {
    url: String,
    token: String,
}

struct FileInfo {
    filename: String,
    hierarchy: Option<Arc<Hierarchy>>,
    file_format: Option<FileFormat>,
    transaction: Option<Arc<Mutex<FTR>>>,
    source_revision: u64,
    header_len: u64,
    body_len: u64,
    body_progress: Arc<AtomicU64>,
    notify: Arc<Notify>,
    timetable: Vec<Time>,
    signals: HashMap<SignalRef, Signal>,
    reloading: bool,
    requested_in_session: bool,
    last_reload_ok: bool,
    last_reload_time: Option<Instant>,
    last_modification_time: Option<SystemTime>,
}

#[derive(Default)]
struct SurverState {
    file_infos: Vec<FileInfo>,
}

impl FileInfo {
    fn modification_time_string(&self) -> String {
        modification_time_string(self.last_modification_time)
    }

    fn reload_time_string(&self) -> String {
        if let Some(time) = self.last_reload_time {
            return format!("{:?} ago", time.elapsed());
        }
        "never".to_string()
    }

    pub fn html_table_line(&self) -> String {
        let bytes_loaded = self.body_progress.load(Ordering::SeqCst);

        let progress = if bytes_loaded == self.body_len {
            format!(
                "{} loaded",
                bytesize::ByteSize::b(self.body_len + self.header_len)
            )
        } else {
            format!(
                "{} / {}",
                bytesize::ByteSize::b(bytes_loaded + self.header_len),
                bytesize::ByteSize::b(self.body_len + self.header_len)
            )
        };

        format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            self.filename,
            progress,
            self.modification_time_string(),
            self.reload_time_string()
        )
    }
}

impl From<&FileInfo> for SurverFileInfo {
    fn from(file_info: &FileInfo) -> Self {
        Self {
            bytes: file_info.body_len + file_info.header_len,
            bytes_loaded: file_info.body_progress.load(Ordering::SeqCst) + file_info.header_len,
            filename: file_info.filename.clone(),
            kind: if file_info.transaction.is_some() {
                SurverFileKind::Transaction
            } else {
                SurverFileKind::Waveform
            },
            format: file_info.file_format,
            reloading: file_info.reloading,
            last_load_ok: file_info.last_reload_ok,
            last_modification_time: file_info.last_modification_time,
        }
    }
}
enum LoaderMessage {
    SignalRequest(SignalRequest),
    Reload,
}

type SignalRequest = Vec<SignalRef>;

fn get_info_page(shared: &Arc<ReadOnly>, state: &Arc<RwLock<SurverState>>) -> String {
    let state_guard = state.read().expect("State lock poisoned in get_info_page");
    let html_table_content = state_guard
        .file_infos
        .iter()
        .map(FileInfo::html_table_line)
        .collect::<Vec<_>>()
        .join("\n");
    drop(state_guard);

    format!(
        r#"
    <!DOCTYPE html><html lang="en">
    <head>
    <link rel="icon" href="favicon.ico" sizes="any">
    <title>Surver - Surfer Remote Server</title>
    </head>
    <body>
    <h1>Surver - Surfer Remote Server</h1>
    <b>To connect, run:</b> <code>surfer {}</code><br>
    <b>Wellen version:</b> {WELLEN_VERSION}<br>
    <b>Surfer version:</b> {SURFER_VERSION}<br>
    <table border="1" cellpadding="5" cellspacing="0">
    <tr><th>Filename</th><th>Load progress</th><th>File modification time</th><th>(Re)load time</th></tr>
    {}
    </table>
    </body></html>
    "#,
        shared.url, html_table_content
    )
}

fn get_hierarchy(state: &Arc<RwLock<SurverState>>, file_index: usize) -> Result<Vec<u8>> {
    let state_guard = state.read().expect("State lock poisoned in get_hierarchy");
    let file_info = &state_guard.file_infos[file_index];
    let file_format = file_info
        .file_format
        .ok_or_else(|| anyhow!("Selected file is not a waveform"))?;
    let hierarchy = file_info
        .hierarchy
        .as_ref()
        .ok_or_else(|| anyhow!("Selected file has no waveform hierarchy"))?;
    let mut raw = BINCODE_OPTIONS.serialize(&file_format)?;
    let mut raw2 = BINCODE_OPTIONS.serialize(hierarchy.as_ref())?;
    drop(state_guard);
    raw.append(&mut raw2);
    let compressed = lz4_flex::compress_prepend_size(&raw);
    info!(
        "Sending hierarchy. {} raw, {} compressed.",
        bytesize::ByteSize::b(raw.len() as u64),
        bytesize::ByteSize::b(compressed.len() as u64)
    );
    Ok(compressed)
}

async fn get_timetable(state: &Arc<RwLock<SurverState>>, file_index: usize) -> Result<Vec<u8>> {
    let notify = {
        let state_guard = state.read().expect("State lock poisoned in get_timetable");
        if state_guard.file_infos[file_index].transaction.is_some() {
            bail!("Selected file is not a waveform");
        }
        state_guard.file_infos[file_index].notify.clone()
    };

    // Wait until the time table is available
    let table = loop {
        {
            let state_guard = state.read().expect("State lock poisoned in get_timetable");
            let timetable = &state_guard.file_infos[file_index].timetable;
            if !timetable.is_empty() {
                break timetable.clone();
            }
        }

        notify.notified().await;
    };

    let raw_size = table.len() * std::mem::size_of::<Time>();
    let compressed = BINCODE_OPTIONS.serialize(&CompressedTimeTable::compress(&table))?;
    info!(
        "Sending timetable. {} raw, {} compressed.",
        bytesize::ByteSize::b(raw_size as u64),
        bytesize::ByteSize::b(compressed.len() as u64)
    );
    Ok(compressed)
}

fn get_status(state: &Arc<RwLock<SurverState>>) -> Result<Vec<u8>> {
    let state_guard = state.read().expect("State lock poisoned in get_status");
    let file_infos = state_guard
        .file_infos
        .iter()
        .map(SurverFileInfo::from)
        .collect::<Vec<_>>();
    drop(state_guard);
    let status = SurverStatus {
        wellen_version: WELLEN_VERSION.to_string(),
        surfer_version: SURFER_VERSION.to_string(),
        capabilities: crate::SurverCapabilities {
            discovery_version: 1,
            waveform_signals: true,
            transaction_pages: Some(TransactionPageCapability {
                protocol_version: TRANSACTION_PAGE_PROTOCOL_VERSION,
                formats: vec!["ftr".to_string()],
                revisioned: true,
                byte_ranges: false,
            }),
        },
        file_infos,
    };
    Ok(serde_json::to_vec(&status)?)
}

fn transaction_file(
    state: &Arc<RwLock<SurverState>>,
    file_index: usize,
) -> Result<(Arc<Mutex<FTR>>, u64)> {
    let state_guard = state
        .read()
        .expect("State lock poisoned in transaction request");
    let file = &state_guard.file_infos[file_index];
    Ok((
        file.transaction
            .clone()
            .ok_or_else(|| anyhow!("Selected file is not an FTR transaction trace"))?,
        file.source_revision,
    ))
}

fn check_transaction_revision(expected: u64, actual: u64) -> Result<()> {
    if expected != actual {
        bail!("Stale transaction source revision {expected}; current revision is {actual}");
    }
    Ok(())
}

fn encode_transaction_payload(value: &impl serde::Serialize) -> Result<Vec<u8>> {
    Ok(lz4_flex::compress_prepend_size(
        &BINCODE_OPTIONS.serialize(value)?,
    ))
}

fn get_transaction_manifest(
    state: &Arc<RwLock<SurverState>>,
    file_index: usize,
) -> Result<Vec<u8>> {
    let (transaction, source_revision) = transaction_file(state, file_index)?;
    let mut ftr = transaction
        .lock()
        .map_err(|_| anyhow!("Transaction trace lock poisoned"))?;
    if ftr
        .relation_blocks
        .iter()
        .any(|block| block.record_count.is_none())
    {
        ftr.visit_relation_blocks(|_, _| Ok(()))
            .map_err(eyre::Report::msg)?;
    }
    let relation_count = ftr
        .relation_blocks
        .iter()
        .map(|block| block.record_count.unwrap_or_default())
        .sum::<u64>();
    let mut streams = ftr.tx_streams.values().cloned().collect::<Vec<_>>();
    streams.sort_by_key(|stream| stream.id);
    for stream in &mut streams {
        stream.transactions_loaded = false;
        for block in &mut stream.tx_blocks {
            block.status = BlockStatus::Indexed;
        }
    }
    let mut generators = ftr.tx_generators.values().cloned().collect::<Vec<_>>();
    generators.sort_by_key(|generator| generator.id);
    for generator in &mut generators {
        generator.transactions = Arc::new(Vec::new());
    }
    let manifest = TransactionManifest {
        protocol_version: TRANSACTION_PAGE_PROTOCOL_VERSION,
        source_revision,
        time_scale: ftr.time_scale,
        max_timestamp: ftr.max_timestamp,
        streams,
        generators,
        dictionary_pages: ftr
            .str_dict
            .len()
            .div_ceil(TRANSACTION_DICTIONARY_PAGE_RECORDS) as u64,
        relation_pages: relation_count.div_ceil(TRANSACTION_RELATION_PAGE_RECORDS as u64),
        relation_count,
    };
    encode_transaction_payload(&manifest)
}

fn get_transaction_dictionary_page(
    state: &Arc<RwLock<SurverState>>,
    file_index: usize,
    source_revision: u64,
    page_id: u64,
) -> Result<Vec<u8>> {
    let (transaction, actual_revision) = transaction_file(state, file_index)?;
    check_transaction_revision(source_revision, actual_revision)?;
    let ftr = transaction
        .lock()
        .map_err(|_| anyhow!("Transaction trace lock poisoned"))?;
    let mut entries = ftr
        .str_dict
        .iter()
        .map(|(id, value)| (*id, value.clone()))
        .collect::<Vec<_>>();
    entries.sort_by_key(|(id, _)| *id);
    let start = usize::try_from(page_id)
        .ok()
        .and_then(|page| page.checked_mul(TRANSACTION_DICTIONARY_PAGE_RECORDS))
        .ok_or_else(|| anyhow!("Invalid dictionary page id {page_id}"))?;
    if start > entries.len() {
        bail!("Dictionary page {page_id} is unavailable");
    }
    let end = (start + TRANSACTION_DICTIONARY_PAGE_RECORDS).min(entries.len());
    encode_transaction_payload(&TransactionDictionaryPage {
        source_revision,
        page_id,
        entries: entries[start..end].to_vec(),
    })
}

fn get_transaction_relation_page(
    state: &Arc<RwLock<SurverState>>,
    file_index: usize,
    source_revision: u64,
    page_id: u64,
) -> Result<Vec<u8>> {
    let (transaction, actual_revision) = transaction_file(state, file_index)?;
    check_transaction_revision(source_revision, actual_revision)?;
    let mut ftr = transaction
        .lock()
        .map_err(|_| anyhow!("Transaction trace lock poisoned"))?;
    if ftr
        .relation_blocks
        .iter()
        .any(|block| block.record_count.is_none())
    {
        ftr.visit_relation_blocks(|_, _| Ok(()))
            .map_err(eyre::Report::msg)?;
    }
    let start = page_id
        .checked_mul(TRANSACTION_RELATION_PAGE_RECORDS as u64)
        .ok_or_else(|| anyhow!("Invalid relation page id {page_id}"))?;
    let relation_count = ftr
        .relation_blocks
        .iter()
        .map(|block| block.record_count.unwrap_or_default())
        .sum::<u64>();
    if start > relation_count {
        bail!("Relation page {page_id} is unavailable");
    }
    let end = start
        .saturating_add(TRANSACTION_RELATION_PAGE_RECORDS as u64)
        .min(relation_count);
    let blocks = ftr.relation_blocks.clone();
    let mut block_start = 0u64;
    let mut relations = Vec::with_capacity((end - start) as usize);
    for block in blocks {
        let block_end = block_start.saturating_add(block.record_count.unwrap_or_default());
        if block_end > start && block_start < end {
            let decoded = ftr
                .read_relation_block(block.ordinal)
                .map_err(eyre::Report::msg)?;
            let local_start = start.saturating_sub(block_start) as usize;
            let local_end = (end.min(block_end) - block_start) as usize;
            relations.extend_from_slice(&decoded[local_start..local_end]);
        }
        block_start = block_end;
        if block_start >= end {
            break;
        }
    }
    let page = TransactionRelationPage {
        source_revision,
        page_id,
        relations,
    };
    encode_transaction_payload(&page)
}

fn get_transaction_record_page(
    state: &Arc<RwLock<SurverState>>,
    file_index: usize,
    source_revision: u64,
    stream_id: StreamId,
    page_id: u64,
) -> Result<Vec<u8>> {
    let (transaction, actual_revision) = transaction_file(state, file_index)?;
    check_transaction_revision(source_revision, actual_revision)?;
    let mut ftr = transaction
        .lock()
        .map_err(|_| anyhow!("Transaction trace lock poisoned"))?;
    let block = ftr
        .get_stream(stream_id)
        .and_then(|stream| stream.tx_blocks.get(page_id as usize))
        .cloned()
        .ok_or_else(|| {
            anyhow!("Transaction page {page_id} for stream {stream_id} is unavailable")
        })?;
    let transactions = ftr
        .read_stream_block_unlinked(stream_id, page_id)
        .map_err(eyre::Report::msg)?;
    let page = TransactionRecordPage {
        source_revision,
        stream_id,
        page_id,
        block,
        transactions,
    };
    encode_transaction_payload(&page)
}

async fn get_signals(
    state: &Arc<RwLock<SurverState>>,
    file_index: usize,
    txs: &[Option<Sender<LoaderMessage>>],
    id_strings: &[&str],
) -> Result<Vec<u8>> {
    let ids = id_strings
        .iter()
        .map(|id_str| {
            id_str
                .parse::<u64>()
                .map_err(|e| anyhow!("Failed to parse signal id `{id_str}`: {e:#}"))
                .and_then(|index| {
                    SignalRef::from_index(index as usize)
                        .ok_or_else(|| anyhow!("Invalid signal index: {}", index))
                })
        })
        .collect::<Result<Vec<SignalRef>>>()?;

    if ids.is_empty() {
        return Ok(vec![]);
    }
    let num_ids = ids.len();

    // send request to background thread
    txs[file_index]
        .as_ref()
        .ok_or_else(|| anyhow!("Selected file is not a waveform"))?
        .send(LoaderMessage::SignalRequest(ids.clone()))?;

    let notify = {
        let state_guard = state.read().expect("State lock poisoned in get_signals");
        state_guard.file_infos[file_index].notify.clone()
    };

    // Wait for all signals to be loaded
    let mut data = vec![];
    leb128::write::unsigned(&mut data, num_ids as u64)?;
    let mut raw_size = 0;
    loop {
        {
            let state_guard = state.read().expect("State lock poisoned in get_signals");
            if ids
                .iter()
                .all(|id| state_guard.file_infos[file_index].signals.contains_key(id))
            {
                for id in ids {
                    let signal = &state_guard.file_infos[file_index].signals[&id];
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

fn build_response(
    status: StatusCode,
    content_type: &str,
    body: Vec<u8>,
) -> Result<Response<Full<Bytes>>> {
    Ok(Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .default_header()
        .body(Full::from(body))?)
}

fn not_found_response(message: &[u8]) -> Result<Response<Full<Bytes>>> {
    build_response(StatusCode::NOT_FOUND, OCTET_MIME, message.to_vec())
}

fn transaction_response(result: Result<Vec<u8>>, immutable: bool) -> Result<Response<Full<Bytes>>> {
    match result {
        Ok(body) => {
            let mut response = build_response(StatusCode::OK, OCTET_MIME, body)?;
            if immutable {
                response.headers_mut().insert(
                    "Cache-Control",
                    hyper::header::HeaderValue::from_static("private, max-age=31536000, immutable"),
                );
            }
            Ok(response)
        }
        Err(error) => {
            let message = format!("{error:#}");
            let status = if message.contains("Stale transaction source revision") {
                StatusCode::CONFLICT
            } else {
                StatusCode::BAD_REQUEST
            };
            build_response(status, OCTET_MIME, message.into_bytes())
        }
    }
}

fn parse_path_u64(value: &str, label: &str) -> Result<u64> {
    value
        .parse()
        .with_context(|| format!("Invalid {label} `{value}`"))
}

fn mark_file_requested(state: &Arc<RwLock<SurverState>>, file_index: usize) {
    let mut state_guard = state
        .write()
        .expect("State lock poisoned in request tracking");
    state_guard.file_infos[file_index].requested_in_session = true;
}

fn source_revision(metadata: &fs::Metadata) -> u64 {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |duration| {
            duration.as_secs() ^ u64::from(duration.subsec_nanos()).rotate_left(17)
        });
    metadata.len() ^ modified.rotate_left(29)
}

fn handle_reload_cmd(
    state: &Arc<RwLock<SurverState>>,
    txs: &[Option<Sender<LoaderMessage>>],
    file_index: usize,
) -> Result<Response<Full<Bytes>>> {
    let mtime = {
        let state_guard = state
            .read()
            .expect("State lock poisoned in reload before metadata");
        // Read metadata before taking the write lock to minimize lock contention.
        let Ok(meta) = fs::metadata(&state_guard.file_infos[file_index].filename) else {
            return not_found_response(b"error: file not found");
        };
        meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    };

    let mut state_guard = state.write().expect("State lock poisoned in reload");
    let file_info = &mut state_guard.file_infos[file_index];

    // Should probably look at file lengths as well for extra safety, but they are not updated correctly at the moment
    let unchanged = file_info.last_modification_time == Some(mtime) && file_info.last_reload_ok;
    if unchanged {
        if file_info.requested_in_session {
            drop(state_guard);
            return build_response(
                StatusCode::NOT_MODIFIED,
                JSON_MIME,
                b"info: file unchanged".to_vec(),
            );
        }
        // If file is unchanged but not yet requested in this session.
        // Probably a new Surver session is started, so return file.
        file_info.requested_in_session = true;
        drop(state_guard);
        let body = get_status(state)?;
        return build_response(StatusCode::ACCEPTED, JSON_MIME, body);
    }
    file_info.requested_in_session = true;
    file_info.last_modification_time = Some(mtime);
    info!(
        "File modification time updated to {}",
        file_info.modification_time_string()
    );
    file_info.reloading = true;
    file_info.last_reload_ok = false;
    if file_info.transaction.is_some() {
        let filename = file_info.filename.clone();
        drop(state_guard);
        let parsed =
            ftr_parser::parse::parse_ftr(filename.clone().into()).map_err(eyre::Report::msg)?;
        let metadata = fs::metadata(&filename)?;
        let mut state_guard = state.write().expect("State lock poisoned after FTR reload");
        let file_info = &mut state_guard.file_infos[file_index];
        file_info.transaction = Some(Arc::new(Mutex::new(parsed)));
        file_info.source_revision = source_revision(&metadata);
        file_info.header_len = 0;
        file_info.body_len = metadata.len();
        file_info
            .body_progress
            .store(metadata.len(), Ordering::SeqCst);
        file_info.reloading = false;
        file_info.last_reload_ok = true;
        file_info.last_reload_time = Some(Instant::now());
        drop(state_guard);
        let body = get_status(state)?;
        return build_response(StatusCode::ACCEPTED, JSON_MIME, body);
    }
    drop(state_guard);
    info!("Reload requested");
    txs[file_index]
        .as_ref()
        .ok_or_else(|| anyhow!("Selected file cannot be reloaded"))?
        .send(LoaderMessage::Reload)?;
    let body = get_status(state)?;
    build_response(StatusCode::ACCEPTED, JSON_MIME, body)
}

async fn handle_cmd(
    state: &Arc<RwLock<SurverState>>,
    txs: &[Option<Sender<LoaderMessage>>],
    cmd: &str,
    file_index: Option<usize>,
    args: &[&str],
) -> Result<Response<Full<Bytes>>> {
    // Check file index is valid if provided
    if let Some(file_index) = file_index {
        let state_guard = state.read().expect("State lock poisoned in handle_cmd");
        if file_index >= state_guard.file_infos.len() {
            drop(state_guard);
            return not_found_response(b"Invalid file index");
        }
    }
    match (file_index, cmd, args) {
        (_, "get_status", []) => {
            let body = get_status(state)?;
            build_response(StatusCode::OK, JSON_MIME, body)
        }
        (Some(file_index), "get_hierarchy", []) => {
            mark_file_requested(state, file_index);
            let body = get_hierarchy(state, file_index)?;
            build_response(StatusCode::OK, OCTET_MIME, body)
        }
        (Some(file_index), "get_time_table", []) => {
            mark_file_requested(state, file_index);
            let body = get_timetable(state, file_index).await?;
            build_response(StatusCode::OK, OCTET_MIME, body)
        }
        (Some(file_index), "get_signals", id_strings) => {
            mark_file_requested(state, file_index);
            let body = get_signals(state, file_index, txs, id_strings).await?;
            build_response(StatusCode::OK, OCTET_MIME, body)
        }
        (Some(file_index), "get_transaction_manifest", []) => {
            mark_file_requested(state, file_index);
            transaction_response(get_transaction_manifest(state, file_index), false)
        }
        (Some(file_index), "get_transaction_dictionary_page", [revision, page]) => {
            mark_file_requested(state, file_index);
            let result = (|| {
                get_transaction_dictionary_page(
                    state,
                    file_index,
                    parse_path_u64(revision, "source revision")?,
                    parse_path_u64(page, "dictionary page")?,
                )
            })();
            transaction_response(result, true)
        }
        (Some(file_index), "get_transaction_relation_page", [revision, page]) => {
            mark_file_requested(state, file_index);
            let result = (|| {
                get_transaction_relation_page(
                    state,
                    file_index,
                    parse_path_u64(revision, "source revision")?,
                    parse_path_u64(page, "relation page")?,
                )
            })();
            transaction_response(result, true)
        }
        (Some(file_index), "get_transaction_page", [revision, stream_id, page_id]) => {
            mark_file_requested(state, file_index);
            let result = (|| {
                get_transaction_record_page(
                    state,
                    file_index,
                    parse_path_u64(revision, "source revision")?,
                    StreamId(parse_path_u64(stream_id, "stream id")?),
                    parse_path_u64(page_id, "transaction page")?,
                )
            })();
            transaction_response(result, true)
        }
        (Some(file_index), "reload", []) => handle_reload_cmd(state, txs, file_index),
        _ => {
            // unknown command or unexpected number of arguments
            not_found_response(&[])
        }
    }
}

async fn handle(
    state: Arc<RwLock<SurverState>>,
    shared: Arc<ReadOnly>,
    txs: Vec<Option<Sender<LoaderMessage>>>,
    req: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>> {
    // Check if favicon is requested
    if req.uri().path() == "/favicon.ico" {
        let favicon_data = include_bytes!("../assets/favicon.ico");
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "image/x-icon")
            .header("Cache-Control", "public, max-age=604800")
            .body(Full::from(&favicon_data[..]))?);
    }
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
            return not_found_response(&[]);
        }
    } else {
        // no token
        warn!("Received request with no token: {:?}", req.uri());
        return not_found_response(&[]);
    }

    // Try to parse file index from path_parts[1]
    let (file_index, cmd_idx) = path_parts
        .get(1)
        .and_then(|s| s.parse::<usize>().ok())
        .map_or((None, 1), |idx| (Some(idx), 2));
    // check command
    let response = if let Some(cmd) = path_parts.get(cmd_idx) {
        handle_cmd(&state, &txs, cmd, file_index, &path_parts[cmd_idx + 1..]).await?
    } else {
        // valid token, but no command => return info
        let body = Full::from(get_info_page(&shared, &state));
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

pub async fn surver_main(
    port: u16,
    bind_address: String,
    token: Option<String>,
    filenames: &[String],
    started: Option<ServerStartedFlag>,
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

    let state = Arc::new(RwLock::new(SurverState { file_infos: vec![] }));

    let mut txs: Vec<Option<Sender<LoaderMessage>>> = Vec::new();
    // load files
    for (file_index, filename) in filenames.iter().enumerate() {
        if std::path::Path::new(filename)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("ftr"))
        {
            let start = web_time::Instant::now();
            let ftr =
                ftr_parser::parse::parse_ftr(filename.clone().into()).map_err(eyre::Report::msg)?;
            let metadata = fs::metadata(filename)?;
            let bytes = metadata.len();
            info!(
                "Loaded FTR directory of {filename} in {:?}",
                start.elapsed()
            );
            let file_info = FileInfo {
                filename: filename.clone(),
                hierarchy: None,
                file_format: None,
                transaction: Some(Arc::new(Mutex::new(ftr))),
                source_revision: source_revision(&metadata),
                header_len: 0,
                body_len: bytes,
                body_progress: Arc::new(AtomicU64::new(bytes)),
                notify: Arc::new(Notify::new()),
                timetable: Vec::new(),
                signals: HashMap::new(),
                reloading: false,
                requested_in_session: false,
                last_reload_ok: true,
                last_reload_time: Some(Instant::now()),
                last_modification_time: metadata.modified().ok(),
            };
            state
                .write()
                .expect("State lock poisoned when adding transaction file")
                .file_infos
                .push(file_info);
            txs.push(None);
            continue;
        }
        let start_read_header = web_time::Instant::now();
        let header_result = wellen::viewers::read_header_from_file(
            filename.clone(),
            &WELLEN_SURFER_DEFAULT_OPTIONS,
        )
        .map_err(|e| anyhow!("{e:?}"))
        .with_context(|| format!("Failed to parse wave file: {filename}"))?;
        info!(
            "Loaded header of {filename} in {:?}",
            start_read_header.elapsed()
        );

        let file_info = FileInfo {
            filename: filename.clone(),
            hierarchy: Some(Arc::new(header_result.hierarchy)),
            file_format: Some(header_result.file_format),
            transaction: None,
            source_revision: 0,
            header_len: 0, // FIXME: get value from wellen
            body_len: header_result.body_len,
            body_progress: Arc::new(AtomicU64::new(0)),
            notify: Arc::new(Notify::new()),
            timetable: vec![],
            signals: HashMap::new(),
            reloading: false,
            requested_in_session: false,
            last_reload_ok: true,
            last_reload_time: None,
            last_modification_time: None,
        };
        {
            let mut state_guard = state.write().expect("State lock poisoned when adding file");
            state_guard.file_infos.push(file_info);
        }
        // channel to communicate with loader
        let (tx, rx) = std::sync::mpsc::channel::<LoaderMessage>();
        txs.push(Some(tx.clone()));
        // start work thread
        let state_2 = state.clone();
        std::thread::spawn(move || loader(&state_2, header_result.body, file_index, &rx));
    }
    let ip_addr: std::net::IpAddr = bind_address
        .parse()
        .with_context(|| format!("Invalid bind address: {bind_address}"))?;
    let use_localhost = ip_addr.is_loopback();
    if !use_localhost {
        warn!(
            "Server is binding to {bind_address} instead of 127.0.0.1/0:0:0:0:0:0:0:1 (localhost)"
        );
        warn!("This may make the server accessible from external networks");
        warn!("Surver traffic is unencrypted and unauthenticated - use with caution!");
    }

    // immutable read-only data
    let addr = SocketAddr::new(ip_addr, port);
    let url = format!("http://{addr}/{token}");
    let url_copy = url.clone();
    let token_copy = token.clone();
    let shared = Arc::new(ReadOnly { url, token });

    // print out status
    info!("Starting server on {addr}. To use:");
    info!("1. Setup an ssh tunnel: -L {port}:localhost:{port}");
    let hostname = whoami::hostname();
    if let Ok(hostname) = hostname.as_ref()
        && hostname != "localhost"
        && let Ok(username) = whoami::username()
    {
        info!(
            "   The correct command may be: ssh -L {port}:localhost:{port} {username}@{hostname} "
        );
    }

    info!("2. Start Surfer: surfer {url_copy} ");
    if !use_localhost && let Ok(hostname) = hostname {
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
        let txs = txs.clone();
        tokio::task::spawn(async move {
            let service =
                service_fn(move |req| handle(state.clone(), shared.clone(), txs.clone(), req));
            if let Err(e) = http1::Builder::new().serve_connection(io, service).await {
                error!("server error: {e}");
            }
        });
    }
}

/// Thread that loads the body and signals.
fn loader(
    state: &Arc<RwLock<SurverState>>,
    mut body_cont: viewers::ReadBodyContinuation<std::io::BufReader<std::fs::File>>,
    file_index: usize,
    rx: &std::sync::mpsc::Receiver<LoaderMessage>,
) -> Result<()> {
    loop {
        // load the body of the file
        let start_load_body = web_time::Instant::now();
        let (filename, hierarchy, body_progress) = {
            let state_guard = state
                .read()
                .expect("State lock poisoned in loader before body load");
            let file_info = &state_guard.file_infos[file_index];
            let hierarchy = file_info
                .hierarchy
                .clone()
                .ok_or_else(|| anyhow!("Waveform loader has no hierarchy"))?;
            (
                file_info.filename.clone(),
                hierarchy,
                file_info.body_progress.clone(),
            )
        };

        // Parse body without holding the state lock to reduce contention with request handling.
        let body_result = viewers::read_body(body_cont, &hierarchy, Some(body_progress))
            .map_err(|e| anyhow!("{e:?}"))
            .with_context(|| format!("Failed to parse body of wave file: {filename}"))?;

        info!(
            "Loaded body of {} in {:?}",
            filename,
            start_load_body.elapsed()
        );

        // update state with body results
        {
            let mut state_guard = state
                .write()
                .expect("State lock poisoned in loader after body load");
            let file_info = &mut state_guard.file_infos[file_index];
            file_info.timetable = body_result.time_table;
            file_info.signals.clear(); // Clear old signals on reload
            if let Ok(meta) = fs::metadata(&file_info.filename) {
                file_info.last_modification_time = Some(meta.modified()?);
                info!(
                    "File modification time of {} set to {}",
                    filename,
                    file_info.modification_time_string()
                );
            }
            file_info.last_reload_time = Some(Instant::now());
            file_info.reloading = false;
            file_info.last_reload_ok = true;
            file_info.notify.notify_waiters();
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
                        let state_guard = state
                            .read()
                            .expect("State lock poisoned in loader signal request");
                        ids.iter()
                            .filter(|id| {
                                !state_guard.file_infos[file_index].signals.contains_key(id)
                            })
                            .copied()
                            .collect::<Vec<_>>()
                    };

                    // check if there is anything left to do
                    if filtered_ids.is_empty() {
                        continue;
                    }

                    // load signals without holding the lock
                    filtered_ids.sort();
                    filtered_ids.dedup();
                    let result = {
                        let state_guard = state
                            .read()
                            .expect("State lock poisoned in loader signal request");
                        source.load_signals(
                            &filtered_ids,
                            state_guard.file_infos[file_index]
                                .hierarchy
                                .as_deref()
                                .ok_or_else(|| anyhow!("Waveform loader has no hierarchy"))?,
                            true,
                        )
                    };

                    // store signals
                    {
                        let mut state_guard = state
                            .write()
                            .expect("State lock poisoned in loader when storing signals");
                        for signal in result {
                            state_guard.file_infos[file_index]
                                .signals
                                .insert(signal.signal_ref(), signal);
                        }
                        state_guard.file_infos[file_index].notify.notify_waiters();
                    }
                }
                LoaderMessage::Reload => {
                    let state_guard = state
                        .read()
                        .expect("State lock poisoned in loader before reload");
                    info!(
                        "Reloading waveform file: {}",
                        state_guard.file_infos[file_index].filename
                    );
                    // Reset progress counter
                    state_guard.file_infos[file_index]
                        .body_progress
                        .store(0, Ordering::SeqCst);

                    // Re-read header to get new body continuation
                    let header_result = wellen::viewers::read_header_from_file(
                        state_guard.file_infos[file_index].filename.clone(),
                        &WELLEN_SURFER_DEFAULT_OPTIONS,
                    )
                    .map_err(|e| anyhow!("{e:?}"))
                    .with_context(|| {
                        format!(
                            "Failed to reload wave file: {}",
                            state_guard.file_infos[file_index].filename
                        )
                    })?;

                    body_cont = header_result.body;
                    break; // Break inner loop to reload the body
                }
            }
        }
    }
}
