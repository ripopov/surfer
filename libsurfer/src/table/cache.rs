use super::model::{TableModelKey, TableRowId, TableSearchSpec, TableSortKey, TableSortSpec};
use std::sync::OnceLock;

/// Cache key for table data.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TableCacheKey {
    pub model_key: TableModelKey,
    pub display_filter: TableSearchSpec,
    pub view_sort: Vec<TableSortSpec>,
    pub generation: u64,
}

/// Cached table rows and per-row data.
#[derive(Debug, Clone)]
pub struct TableCache {
    pub row_ids: Vec<TableRowId>,
    pub search_texts: Vec<String>,
    pub sort_keys: Vec<Vec<TableSortKey>>,
}

/// Runtime, non-serialized cache handle.
pub struct TableCacheEntry {
    pub inner: OnceLock<TableCache>,
    pub cache_key: TableCacheKey,
    pub generation: u64,
}

/// Error type for cache build failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableCacheError {
    /// The referenced model (variable, stream, etc.) was not found.
    ModelNotFound { description: String },
    /// Search pattern compilation failed (e.g., invalid regex).
    InvalidSearch { pattern: String, reason: String },
    /// Underlying waveform/transaction data is not available.
    DataUnavailable,
    /// Cache build was cancelled (e.g., tile closed during build).
    Cancelled,
}
