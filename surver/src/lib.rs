//! External access to the Surver server.
use std::{sync::LazyLock, time::SystemTime};

use ftr_parser::types::{
    BlockMeta, GeneratorId, NameId, StreamId, Timescale, Transaction, TxGenerator, TxRelation,
    TxStream,
};
use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
mod server;
#[cfg(not(target_arch = "wasm32"))]
pub use server::surver_main;

pub const HTTP_SERVER_KEY: &str = "Server";
pub const HTTP_SERVER_VALUE_SURFER: &str = "Surfer";
pub const X_WELLEN_VERSION: &str = "x-wellen-version";
pub const X_SURFER_VERSION: &str = "x-surfer-version";
pub const SURFER_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const WELLEN_VERSION: &str = wellen::VERSION;
pub const TRANSACTION_PAGE_PROTOCOL_VERSION: u16 = 1;
pub const TRANSACTION_RELATION_PAGE_RECORDS: usize = 65_536;
pub const TRANSACTION_DICTIONARY_PAGE_RECORDS: usize = 4_096;

pub const WELLEN_SURFER_DEFAULT_OPTIONS: wellen::LoadOptions = wellen::LoadOptions {
    multi_thread: true,
    remove_scopes_with_empty_name: true,
};

#[derive(Debug, Deserialize)]
pub struct SurverConfig {
    /// IP address to bind the HTTP server to
    pub bind_address: String,
    /// Default port for the HTTP server
    pub port: u16,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SurverStatus {
    pub wellen_version: String,
    pub surfer_version: String,
    /// Versioned feature discovery. Older servers omit this field and are
    /// treated as waveform-only rather than failing during a later request.
    #[serde(default)]
    pub capabilities: SurverCapabilities,
    pub file_infos: Vec<SurverFileInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SurverCapabilities {
    pub discovery_version: u16,
    pub waveform_signals: bool,
    pub transaction_pages: Option<TransactionPageCapability>,
}

impl Default for SurverCapabilities {
    fn default() -> Self {
        Self {
            discovery_version: 1,
            waveform_signals: true,
            transaction_pages: None,
        }
    }
}

impl SurverCapabilities {
    #[must_use]
    pub fn konata_unavailable_reason(&self) -> Option<String> {
        self.transaction_pages.is_none().then(|| {
            "Remote Konata view unavailable: this Surver does not expose versioned transaction pages"
                .to_string()
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransactionPageCapability {
    pub protocol_version: u16,
    pub formats: Vec<String>,
    pub revisioned: bool,
    pub byte_ranges: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum SurverFileKind {
    #[default]
    Waveform,
    Transaction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionManifest {
    pub protocol_version: u16,
    pub source_revision: u64,
    pub time_scale: Timescale,
    pub max_timestamp: u64,
    pub streams: Vec<TxStream>,
    pub generators: Vec<TxGenerator>,
    pub dictionary_pages: u64,
    pub relation_pages: u64,
    pub relation_count: u64,
}

impl TransactionManifest {
    #[must_use]
    pub fn stream(&self, stream_id: StreamId) -> Option<&TxStream> {
        self.streams.iter().find(|stream| stream.id == stream_id)
    }

    #[must_use]
    pub fn generator(&self, generator_id: GeneratorId) -> Option<&TxGenerator> {
        self.generators
            .iter()
            .find(|generator| generator.id == generator_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionDictionaryPage {
    pub source_revision: u64,
    pub page_id: u64,
    pub entries: Vec<(NameId, std::sync::Arc<str>)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionRelationPage {
    pub source_revision: u64,
    pub page_id: u64,
    pub relations: Vec<TxRelation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionRecordPage {
    pub source_revision: u64,
    pub stream_id: StreamId,
    pub page_id: u64,
    pub block: BlockMeta,
    pub transactions: Vec<Transaction>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SurverFileInfo {
    pub bytes: u64,
    pub bytes_loaded: u64,
    pub filename: String,
    #[serde(default)]
    pub kind: SurverFileKind,
    pub format: Option<wellen::FileFormat>,
    pub reloading: bool,
    pub last_load_ok: bool,
    pub last_modification_time: Option<SystemTime>,
}

impl SurverFileInfo {
    #[must_use]
    pub fn modification_time_string(&self) -> String {
        modification_time_string(self.last_modification_time)
    }
}

pub static BINCODE_OPTIONS: LazyLock<bincode::DefaultOptions> =
    LazyLock::new(bincode::DefaultOptions::new);

pub(crate) fn modification_time_string(mtime: Option<SystemTime>) -> String {
    if let Some(mtime) = mtime {
        let dur = mtime
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        return chrono::DateTime::<chrono::Utc>::from_timestamp(
            dur.as_secs().cast_signed(),
            dur.subsec_nanos(),
        )
        .map_or_else(
            || "Incorrect timestamp".to_string(),
            |dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        );
    }
    "unknown".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_from_an_older_server_defaults_to_waveform_only() {
        let status: SurverStatus = serde_json::from_value(serde_json::json!({
            "wellen_version": WELLEN_VERSION,
            "surfer_version": SURFER_VERSION,
            "file_infos": [],
        }))
        .unwrap();
        assert!(status.capabilities.waveform_signals);
        assert!(status.capabilities.transaction_pages.is_none());
        assert!(status.capabilities.konata_unavailable_reason().is_some());
    }
}
