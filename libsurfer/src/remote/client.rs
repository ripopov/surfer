use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::mpsc::Sender;

use bincode::Options;
use eyre::{Result, WrapErr as _, anyhow, bail, eyre};
use ftr_parser::types::{FTR, GeneratorId, NameId, StreamId, Transaction};
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use thiserror::Error;
use tracing::{info, warn};
use wellen::CompressedTimeTable;

use surver::{
    BINCODE_OPTIONS, HTTP_SERVER_KEY, HTTP_SERVER_VALUE_SURFER, SURFER_VERSION, SurverFileKind,
    SurverStatus, TRANSACTION_PAGE_PROTOCOL_VERSION, TransactionDictionaryPage,
    TransactionManifest, TransactionRecordPage, TransactionRelationPage, WELLEN_VERSION,
    X_SURFER_VERSION, X_WELLEN_VERSION,
};

use super::HierarchyResponse;
use crate::async_util::{perform_async_work, sleep_ms};
use crate::channels::checked_send;
use crate::konata::{KonataRecordProjector, KonataRelationProjection};
use crate::message::Message;
use crate::transaction_container::{RemoteTransactionSource, TransactionContainer};
use crate::wave_source::{LoadOptions, WaveFormat, WaveSource};
use crate::wellen::{BodyResult, HeaderResult};

/// Returns a shared reqwest client to reuse HTTP connections and reduce TLS overhead.
fn get_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

#[derive(Debug, Error)]
pub enum ReloadError {
    #[error("File unchanged since last reload")]
    FileUnchanged,
    #[error("Unexpected response code: {0}")]
    UnexpectedStatus(StatusCode),
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("Parse error: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("Response validation error: {0}")]
    Validation(#[from] eyre::Report),
}

fn check_response(server_url: &str, response: &reqwest::Response) -> Result<()> {
    let server = response
        .headers()
        .get(HTTP_SERVER_KEY)
        .ok_or(eyre!("no server header"))?
        .to_str()?;
    if server != HTTP_SERVER_VALUE_SURFER {
        bail!("Unexpected server {server} from {server_url}");
    }
    let surfer_version = response
        .headers()
        .get(X_SURFER_VERSION)
        .ok_or(eyre!("no surfer version header"))?
        .to_str()?;
    if surfer_version != SURFER_VERSION {
        // this mismatch may be OK as long as the wellen version matches
        info!(
            "Surfer version on the server: {surfer_version} does not match client version {SURFER_VERSION}"
        );
    }
    let wellen_version = response
        .headers()
        .get(X_WELLEN_VERSION)
        .ok_or(eyre!("no wellen version header"))?
        .to_str()?;
    if wellen_version != WELLEN_VERSION {
        bail!(
            "Version incompatibility! The server uses wellen {wellen_version}, our client uses wellen {WELLEN_VERSION}"
        );
    }
    Ok(())
}

async fn get_status(server: String) -> Result<SurverStatus> {
    let client = get_client();
    let response = client.get(format!("{server}/get_status")).send().await?;
    check_response(&server, &response)?;
    let body = response.text().await?;
    let status = serde_json::from_str::<SurverStatus>(&body)?;
    Ok(status)
}

async fn get_transaction_payload<T: DeserializeOwned>(server: &str, url: String) -> Result<T> {
    let response = get_client().get(&url).send().await?;
    check_response(server, &response)?;
    if !response.status().is_success() {
        let status = response.status();
        let message = response.text().await.unwrap_or_default();
        bail!("Transaction-page request failed with {status}: {message}");
    }
    let compressed = response.bytes().await?;
    let bytes = lz4_flex::decompress_size_prepended(&compressed)?;
    Ok(BINCODE_OPTIONS.deserialize(&bytes)?)
}

async fn get_transaction_manifest(server: &str, file_index: usize) -> Result<TransactionManifest> {
    let manifest: TransactionManifest = get_transaction_payload(
        server,
        format!("{server}/{file_index}/get_transaction_manifest"),
    )
    .await?;
    if manifest.protocol_version != TRANSACTION_PAGE_PROTOCOL_VERSION {
        bail!(
            "Unsupported transaction-page protocol {}; expected {}",
            manifest.protocol_version,
            TRANSACTION_PAGE_PROTOCOL_VERSION
        );
    }
    Ok(manifest)
}

pub fn get_transactions_from_server(
    sender: Sender<Message>,
    server: String,
    load_options: LoadOptions,
    file_index: usize,
) {
    perform_async_work(async move {
        let result = get_transaction_manifest(&server, file_index)
            .await
            .with_context(|| format!("Failed to retrieve FTR manifest from {server}"));
        let message = match result {
            Ok(manifest) => {
                let manifest = Arc::new(manifest);
                let streams = manifest
                    .streams
                    .iter()
                    .cloned()
                    .map(|stream| (stream.id, stream))
                    .collect();
                let generators = manifest
                    .generators
                    .iter()
                    .cloned()
                    .map(|generator| (generator.id, generator))
                    .collect();
                let ftr = FTR::from_parts(
                    manifest.time_scale,
                    manifest.max_timestamp,
                    Default::default(),
                    streams,
                    generators,
                    Vec::new(),
                );
                Message::TransactionStreamsLoaded(
                    WaveSource::Url(server.clone()),
                    WaveFormat::Ftr,
                    TransactionContainer::new_remote(ftr, server.clone(), file_index, manifest),
                    load_options,
                )
            }
            Err(error) => Message::Error(error),
        };
        checked_send(&sender, message);
    });
}

pub(crate) async fn get_transaction_projection(
    remote: &RemoteTransactionSource,
    stream_id: StreamId,
    parent_generator: GeneratorId,
    event_generator: GeneratorId,
    cancel: &std::sync::atomic::AtomicBool,
    mut progress: impl FnMut(f32, &str),
    mut publish: impl FnMut(&[Transaction], &[Transaction]),
) -> Result<(KonataRecordProjector, KonataRelationProjection)> {
    let manifest = &remote.manifest;
    let revision = manifest.source_revision;
    let total_pages = manifest.dictionary_pages
        + manifest.relation_pages
        + manifest
            .stream(stream_id)
            .map_or(0, |stream| stream.tx_blocks.len() as u64);
    let mut completed = 0u64;
    let mut dictionary = std::collections::HashMap::<NameId, Arc<str>>::new();
    for page_id in 0..manifest.dictionary_pages {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            bail!("Remote Konata model build cancelled");
        }
        let page: TransactionDictionaryPage = get_transaction_payload(
            &remote.server,
            format!(
                "{}/{}/get_transaction_dictionary_page/{revision}/{page_id}",
                remote.server, remote.file_index
            ),
        )
        .await?;
        if page.source_revision != revision || page.page_id != page_id {
            bail!("Stale or mismatched transaction dictionary page {page_id}");
        }
        dictionary.extend(page.entries);
        completed += 1;
        progress(
            completed as f32 / total_pages.max(1) as f32,
            "Loading remote dictionary pages",
        );
    }

    let blocks = manifest
        .stream(stream_id)
        .ok_or_else(|| eyre!("Remote stream {stream_id} is unavailable"))?
        .tx_blocks
        .len();
    let mut records = KonataRecordProjector::new(parent_generator, event_generator, stream_id, 0);
    let mut progressive_parents = Vec::new();
    let mut progressive_events = Vec::new();
    for page_id in 0..blocks as u64 {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            bail!("Remote Konata model build cancelled");
        }
        let page: TransactionRecordPage = get_transaction_payload(
            &remote.server,
            format!(
                "{}/{}/get_transaction_page/{revision}/{}/{page_id}",
                remote.server, remote.file_index, stream_id.0
            ),
        )
        .await?;
        if page.source_revision != revision
            || page.stream_id != stream_id
            || page.page_id != page_id
        {
            bail!("Stale or mismatched transaction record page {page_id}");
        }
        records.push_cooperative(&page.transactions).await;
        let remaining = 4096usize.saturating_sub(progressive_parents.len());
        progressive_parents.extend(
            page.transactions
                .iter()
                .filter(|transaction| transaction.get_gen_id() == parent_generator)
                .take(remaining)
                .cloned(),
        );
        if progressive_events.is_empty()
            && let Some(event) = page
                .transactions
                .iter()
                .find(|transaction| transaction.get_gen_id() == event_generator)
                .cloned()
        {
            progressive_events.push(event);
        }
        publish(&progressive_parents, &progressive_events);
        completed += 1;
        progress(
            completed as f32 / total_pages.max(1) as f32,
            "Streaming remote transaction pages",
        );
    }
    let mut projector = records.relation_projector();
    for page_id in 0..manifest.relation_pages {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            bail!("Remote Konata model build cancelled");
        }
        let page: TransactionRelationPage = get_transaction_payload(
            &remote.server,
            format!(
                "{}/{}/get_transaction_relation_page/{revision}/{page_id}",
                remote.server, remote.file_index
            ),
        )
        .await?;
        if page.source_revision != revision || page.page_id != page_id {
            bail!("Stale or mismatched transaction relation page {page_id}");
        }
        projector.push(&page.relations);
        completed += 1;
        progress(
            completed as f32 / total_pages.max(1) as f32,
            "Joining remote relation pages",
        );
    }
    // Fetching dictionary fragments is part of the protocol integrity path;
    // record payloads retain their interned strings directly for this version.
    let _ = dictionary;
    Ok((records, projector.finish()))
}

async fn reload(
    server: String,
    file_index: usize,
) -> std::result::Result<SurverStatus, ReloadError> {
    let client = get_client();
    let response = client
        .get(format!("{server}/{file_index}/reload"))
        .send()
        .await?;
    check_response(&server, &response)?;
    let status_code = response.status();
    let body = response.text().await?;
    match status_code {
        StatusCode::NOT_MODIFIED => {
            info!("File unchanged, no reload needed");
            Err(ReloadError::FileUnchanged)
        }
        StatusCode::ACCEPTED => {
            info!("File reloaded at server");
            let status = serde_json::from_str::<SurverStatus>(&body)?;
            Ok(status)
        }
        code => {
            warn!("Unexpected response code: {code}");
            Err(ReloadError::UnexpectedStatus(code))
        }
    }
}

async fn get_hierarchy(server: String, file_index: usize) -> Result<HierarchyResponse> {
    let client = get_client();
    let response = client
        .get(format!("{server}/{file_index}/get_hierarchy"))
        .send()
        .await?;
    check_response(&server, &response)?;
    let compressed = response.bytes().await?;
    let raw = lz4_flex::decompress_size_prepended(&compressed)?;
    let mut reader = std::io::Cursor::new(raw);
    // first we read a value, expecting there to be more bytes
    let opts = BINCODE_OPTIONS.allow_trailing_bytes();
    let file_format: wellen::FileFormat = opts.deserialize_from(&mut reader)?;
    // the last value should consume all remaining bytes
    let hierarchy: wellen::Hierarchy = BINCODE_OPTIONS.deserialize_from(&mut reader)?;
    Ok(HierarchyResponse {
        hierarchy,
        file_format,
    })
}

async fn get_time_table(server: String, file_index: usize) -> Result<Vec<wellen::Time>> {
    let client = get_client();
    let response = client
        .get(format!("{server}/{file_index}/get_time_table"))
        .send()
        .await?;
    check_response(&server, &response)?;
    let compressed_data = response.bytes().await?;
    let compressed: CompressedTimeTable = BINCODE_OPTIONS.deserialize(&compressed_data)?;
    let table = compressed.uncompress();
    Ok(table)
}

// Helper to calculate URL length for a signal index
// Much more efficient than string conversion
// Extracted for testing
#[inline]
fn signal_url_len(index: usize) -> usize {
    index.checked_ilog10().unwrap_or(0) as usize + 2 // +1 for '/', +1 as ilog10 rounds down
}

pub async fn get_signals(
    server: String,
    signals: &[wellen::SignalRef],
    max_url_length: u16,
    file_index: usize,
) -> Result<Vec<wellen::Signal>> {
    if signals.is_empty() {
        return Ok(vec![]);
    }

    let max_url_length = max_url_length as usize;
    let base_url = format!("{server}/{file_index}/get_signals");
    let base_len = base_url.len();

    let mut all_results = Vec::with_capacity(signals.len());
    let mut current_batch = Vec::new();
    let mut current_url_len = base_len;

    for signal in signals {
        // Each signal adds: "/" + digits
        let signal_len = signal_url_len(signal.index());

        // Check if adding this signal would exceed the limit
        if current_url_len + signal_len > max_url_length && !current_batch.is_empty() {
            info!(
                "Fetching batch of {} signals due to URL length limit",
                current_batch.len()
            );
            // Fetch current batch
            let batch_results = get_signals_batch(&base_url, &current_batch).await?;
            all_results.extend(batch_results);

            // Start new batch
            current_batch.clear();
            current_url_len = base_len;
        }

        current_batch.push(*signal);
        current_url_len += signal_len;
    }

    // Fetch remaining batch
    if !current_batch.is_empty() {
        let batch_results = get_signals_batch(&base_url, &current_batch).await?;
        all_results.extend(batch_results);
    }

    Ok(all_results)
}

// Helper to format signal URL
// Extracted for testing
#[inline]
fn format_signal_url(base_url: &str, signals: &[wellen::SignalRef]) -> String {
    let mut url = base_url.to_string();
    for signal in signals {
        write!(url, "/{}", signal.index()).unwrap();
    }
    url
}

async fn get_signals_batch(
    base_url: &str,
    signals: &[wellen::SignalRef],
) -> Result<Vec<wellen::Signal>> {
    let client = get_client();
    let url = format_signal_url(base_url, signals);

    let response = client.get(url).send().await?;
    check_response(base_url, &response)?;
    let data = response.bytes().await?;
    let mut reader = std::io::Cursor::new(data);
    let num_ids: u64 = leb128::read::unsigned(&mut reader)?;
    if num_ids > signals.len() as u64 {
        bail!(
            "Too many signals in response: {num_ids}, expected {}",
            signals.len()
        );
    }
    if num_ids == 0 {
        return Ok(vec![]);
    }

    let opts = BINCODE_OPTIONS.allow_trailing_bytes();
    let mut out = Vec::with_capacity(num_ids as usize);
    for _ in 0..(num_ids - 1) {
        let compressed: wellen::CompressedSignal = opts.deserialize_from(&mut reader)?;
        let signal = compressed.uncompress();
        out.push(signal);
    }
    // for the final signal, we expect to consume all bytes
    let compressed: wellen::CompressedSignal = BINCODE_OPTIONS.deserialize_from(&mut reader)?;
    let signal = compressed.uncompress();
    out.push(signal);
    Ok(out)
}

pub fn get_hierarchy_from_server(
    sender: Sender<Message>,
    server: String,
    load_options: LoadOptions,
    file_index: usize,
) {
    let start = web_time::Instant::now();
    let source = WaveSource::Url(server.clone());

    perform_async_work(async move {
        let res = get_hierarchy(server.clone(), file_index)
            .await
            .map_err(|e| anyhow!("{e:?}"))
            .with_context(|| format!("Failed to retrieve hierarchy from remote server {server}"));

        let msg = match res {
            Ok(h) => {
                let header =
                    HeaderResult::Remote(Arc::new(h.hierarchy), h.file_format, server, file_index);
                Message::WaveHeaderLoaded(start, source, load_options, header)
            }
            Err(e) => Message::Error(e),
        };
        checked_send(&sender, msg);
    });
}

pub fn get_time_table_from_server(sender: Sender<Message>, server: String, file_index: usize) {
    let start = web_time::Instant::now();
    let source = WaveSource::Url(server.clone());

    perform_async_work(async move {
        let res = get_time_table(server.clone(), file_index)
            .await
            .map_err(|e| anyhow!("{e:?}"))
            .with_context(|| format!("Failed to retrieve time table from remote server {server}"));

        let msg = match res {
            Ok(table) => Message::WaveBodyLoaded(start, source, BodyResult::Remote(table, server)),
            Err(e) => Message::Error(e),
        };
        checked_send(&sender, msg);
    });
}

pub fn get_server_status(sender: Sender<Message>, server: String, delay_ms: u64) {
    let start = web_time::Instant::now();
    perform_async_work(async move {
        sleep_ms(delay_ms).await;
        let res = get_status(server.clone())
            .await
            .map_err(|e| anyhow!("{e:?}"))
            .with_context(|| format!("Failed to retrieve status from remote server {server}"));

        let msg = match res {
            Ok(status) => Message::SetSurverStatus(start, server, status),
            Err(e) => Message::Error(e),
        };
        checked_send(&sender, msg);
    });
}

pub fn server_reload(
    sender: Sender<Message>,
    server: String,
    load_options: LoadOptions,
    file_index: usize,
) {
    let start = web_time::Instant::now();
    perform_async_work(async move {
        let res = reload(server.clone(), file_index).await;
        let mut reloaded_kind = None;

        let msg = match res {
            Ok(status) => {
                reloaded_kind = status.file_infos.get(file_index).map(|file| file.kind);
                Message::SetSurverStatus(start, server.clone(), status)
            }
            Err(crate::remote::ReloadError::FileUnchanged) => Message::StopProgressTracker,
            Err(e) => {
                let err = anyhow!("{e:?}");
                Message::Error(err)
            }
        };
        checked_send(&sender, msg);
        match reloaded_kind {
            Some(SurverFileKind::Waveform) => {
                get_hierarchy_from_server(sender, server, load_options, file_index);
            }
            Some(SurverFileKind::Transaction) => {
                get_transactions_from_server(sender, server, load_options, file_index);
            }
            None => {}
        }
    });
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod transaction_tests {
    use std::{
        net::{IpAddr, Ipv4Addr, SocketAddr},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn capable_surver_pages_build_the_same_konata_projection() {
        let port = std::net::TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let file = project_root::get_project_root()
            .unwrap()
            .join("examples/kanata-sample-2.ftr");
        let token = "konataremoteclient".to_string();
        let started = Arc::new(AtomicBool::new(false));
        let task = {
            let started = started.clone();
            let token = token.clone();
            tokio::spawn(async move {
                let _ = surver::surver_main(
                    port,
                    "127.0.0.1".to_string(),
                    Some(token),
                    &[file.to_string_lossy().to_string()],
                    Some(started),
                )
                .await;
            })
        };
        for _ in 0..100 {
            if started.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(started.load(Ordering::SeqCst));

        let server = format!("http://127.0.0.1:{port}/{token}");
        let manifest = Arc::new(get_transaction_manifest(&server, 0).await.unwrap());
        let remote = RemoteTransactionSource {
            server,
            file_index: 0,
            manifest,
        };
        let cancel = AtomicBool::new(false);
        let (records, projection) = get_transaction_projection(
            &remote,
            StreamId(1),
            GeneratorId(10),
            GeneratorId(11),
            &cancel,
            |_, _| {},
            |_, _| {},
        )
        .await
        .unwrap();
        let model = records.finish(projection, false).await;
        assert_eq!(model.row_count(), 4_041);
        assert_eq!(model.stage_count(), 51_961);
        assert_eq!(model.quality.orphans, 0);
        task.abort();
    }
}

mod tests {
    #[test]
    fn test_signal_url_length_calculation() {
        use crate::remote::client::signal_url_len;
        // Test edge cases for digit calculation
        assert_eq!(signal_url_len(0), 2); // "/0" -> 2 chars
        assert_eq!(signal_url_len(1), 2); // "/1" -> 2 chars
        assert_eq!(signal_url_len(9), 2); // "/9" -> 2 chars
        assert_eq!(signal_url_len(10), 3); // "/10" -> 3 chars
        assert_eq!(signal_url_len(99), 3); // "/99" -> 3 chars
        assert_eq!(signal_url_len(100), 4); // "/100" -> 4 chars
        assert_eq!(signal_url_len(999), 4); // "/999" -> 4 chars
        assert_eq!(signal_url_len(1000), 5); // "/1000" -> 5 chars
        assert_eq!(signal_url_len(65535), 6); // "/65535" -> 6 chars
    }

    #[test]
    fn test_empty_signals_returns_empty() {
        use crate::remote::get_signals;
        // Create a mock async runtime for testing
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let signals: Vec<wellen::SignalRef> = vec![];
            let result = get_signals("http://localhost:8080".to_string(), &signals, 1000, 0).await;

            // Should return Ok with empty vec without making any network calls
            assert!(result.is_ok());
            assert_eq!(result.unwrap().len(), 0);
        });
    }

    #[test]
    fn test_boundary_signal_indices() {
        use crate::remote::client::signal_url_len;
        // Test that we handle boundary cases correctly
        let boundary_indices = vec![0, 1, 9, 10, 99, 100, 999, 1000, 9999, 10000];

        for idx in boundary_indices {
            let sig_ref = wellen::SignalRef::from_index(idx);
            let len = signal_url_len(sig_ref.unwrap().index());

            // Verify the calculated length matches actual string length
            let actual = format!("/{idx}");
            assert_eq!(
                len,
                actual.len(),
                "URL length calculation mismatch for index {}: expected {}, got {}",
                idx,
                actual.len(),
                len
            );
        }
    }

    #[test]
    fn test_url_construction_format() {
        use crate::remote::client::format_signal_url;
        // Verify URL format matches expected pattern
        let base_url = "http://localhost:8080/get_signals";
        let signals: Vec<wellen::SignalRef> = vec![
            wellen::SignalRef::from_index(1),
            wellen::SignalRef::from_index(42),
            wellen::SignalRef::from_index(999),
        ]
        .into_iter()
        .flatten()
        .collect();

        let url = format_signal_url(base_url, &signals);

        assert_eq!(url, "http://localhost:8080/get_signals/1/42/999");
    }
}
