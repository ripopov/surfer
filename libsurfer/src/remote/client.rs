use super::HierarchyResponse;
use bincode::Options;
use eyre::Result;
use eyre::{bail, eyre};
use tracing::info;
use wellen::CompressedTimeTable;

use surver::{
    Status, BINCODE_OPTIONS, HTTP_SERVER_KEY, HTTP_SERVER_VALUE_SURFER, SURFER_VERSION,
    WELLEN_VERSION, X_SURFER_VERSION, X_WELLEN_VERSION,
};

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
        info!("Surfer version on the server: {surfer_version} does not match client version {SURFER_VERSION}");
    }
    let wellen_version = response
        .headers()
        .get(X_WELLEN_VERSION)
        .ok_or(eyre!("no wellen version header"))?
        .to_str()?;
    if wellen_version != WELLEN_VERSION {
        bail!("Version incompatibility! The server uses wellen {wellen_version}, our client uses wellen {WELLEN_VERSION}");
    }
    Ok(())
}

pub async fn get_status(server: String) -> Result<Status> {
    let client = reqwest::Client::new();
    let response = client.get(format!("{server}/get_status")).send().await?;
    check_response(&server, &response)?;
    let body = response.text().await?;
    let status = serde_json::from_str::<Status>(&body)?;
    Ok(status)
}

pub async fn get_hierarchy(server: String) -> Result<HierarchyResponse> {
    let client = reqwest::Client::new();
    let response = client.get(format!("{server}/get_hierarchy")).send().await?;
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

pub async fn get_time_table(server: String) -> Result<Vec<wellen::Time>> {
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{server}/get_time_table"))
        .send()
        .await?;
    check_response(&server, &response)?;
    let compressed_data = response.bytes().await?;
    let compressed: CompressedTimeTable = BINCODE_OPTIONS.deserialize(&compressed_data)?;
    let table = compressed.uncompress();
    Ok(table)
}

pub async fn get_signals(
    server: String,
    signals: &[wellen::SignalRef],
) -> Result<Vec<(wellen::SignalRef, wellen::Signal)>> {
    // Hyper supports URLs of 65534 bytes
    const MAX_URL_LENGTH: usize = (u16::MAX - 1) as usize;

    if signals.is_empty() {
        return Ok(vec![]);
    }

    let base_url = format!("{server}/get_signals");
    let base_len = base_url.len();

    let mut all_results = Vec::with_capacity(signals.len());
    let mut current_batch = Vec::new();
    let mut current_url_len = base_len;

    for signal in signals.iter() {
        // Each signal adds: "/" + digits
        let signal_len = signal.index().checked_ilog10().unwrap_or(0) as usize + 2; // +1 for '/', +1 as ilog10 rounds down

        // Check if adding this signal would exceed the limit
        if current_url_len + signal_len > MAX_URL_LENGTH && !current_batch.is_empty() {
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

async fn get_signals_batch(
    base_url: &str,
    signals: &[wellen::SignalRef],
) -> Result<Vec<(wellen::SignalRef, wellen::Signal)>> {
    let client = reqwest::Client::new();
    let mut url = base_url.to_string();
    for signal in signals.iter() {
        url.push_str(&format!("/{}", signal.index()));
    }

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
        out.push((signal.signal_ref(), signal));
    }
    // for the final signal, we expect to consume all bytes
    let compressed: wellen::CompressedSignal = BINCODE_OPTIONS.deserialize_from(&mut reader)?;
    let signal = compressed.uncompress();
    out.push((signal.signal_ref(), signal));
    Ok(out)
}
