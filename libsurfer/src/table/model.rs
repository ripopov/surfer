use crate::transaction_container::{StreamScopeRef, TransactionRef, TransactionStreamRef};
use crate::wave_container::VariableRef;
use egui::RichText;
use num::BigInt;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Unique identifier for a table tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TableTileId(pub u64);

/// Stable row identity for selection and caching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TableRowId(pub u64);

/// Serializable model selector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TableModelSpec {
    SignalChangeList {
        variable: VariableRef,
        field: Vec<String>,
    },
    TransactionTrace {
        stream: StreamScopeRef,
        generator: Option<TransactionStreamRef>,
    },
    /// Source-level search that produces a derived table model from waveform data.
    /// Named `source_query` to distinguish from view-level `display_filter`.
    SearchResults {
        source_query: TableSearchSpec,
    },
    /// Deferred to v2: AnalysisKind and AnalysisParams will define derived metrics.
    AnalysisResults {
        kind: AnalysisKind,
        params: AnalysisParams,
    },
    Virtual {
        rows: usize,
        columns: usize,
        seed: u64,
    },
    Custom {
        key: String,
        payload: String,
    },
}

/// Serializable view configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableViewConfig {
    pub title: String,
    pub columns: Vec<TableColumnConfig>,
    pub sort: Vec<TableSortSpec>,
    /// View-level filter applied to current table cache (post-source search).
    /// Named `display_filter` to distinguish from model-level `source_query`.
    pub display_filter: TableSearchSpec,
    pub selection_mode: TableSelectionMode,
    /// When true, reduces vertical padding from 4px to 2px and uses smaller font.
    pub dense_rows: bool,
    /// When true, header row stays visible during vertical scroll.
    /// When false, header scrolls with content (rarely desired).
    pub sticky_header: bool,
}

/// Stable key for identifying model instances in caches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TableModelKey(pub u64);

/// Schema definition for table columns.
#[derive(Debug, Clone)]
pub struct TableSchema {
    pub columns: Vec<TableColumn>,
}

/// Schema entry for a column.
#[derive(Debug, Clone)]
pub struct TableColumn {
    pub key: TableColumnKey,
    pub label: String,
    pub default_width: Option<f32>,
    pub default_visible: bool,
    pub default_resizable: bool,
}

/// Stable column identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TableColumnKey {
    Str(String),
    Id(u64),
}

/// Serializable column view configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableColumnConfig {
    pub key: TableColumnKey,
    pub width: Option<f32>,
    pub visible: bool,
    pub resizable: bool,
}

/// Column sort specification.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TableSortSpec {
    pub key: TableColumnKey,
    pub direction: TableSortDirection,
}

/// Sort order for a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TableSortDirection {
    Ascending,
    Descending,
}

/// Search configuration for filter operations.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TableSearchSpec {
    pub mode: TableSearchMode,
    pub case_sensitive: bool,
    pub text: String,
}

/// Search match mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TableSearchMode {
    Contains,
    Exact,
    Regex,
}

/// Selection behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TableSelectionMode {
    None,
    Single,
    Multi,
}

/// Runtime selection state (not serialized).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TableSelection {
    pub rows: BTreeSet<TableRowId>,
    pub anchor: Option<TableRowId>,
}

/// Display-ready cell content.
#[derive(Debug, Clone)]
pub enum TableCell {
    Text(String),
    RichText(RichText),
}

/// Sortable value for stable ordering.
#[derive(Debug, Clone, PartialEq)]
pub enum TableSortKey {
    None,
    Numeric(f64),
    Text(String),
    Bytes(Vec<u8>),
}

/// Activation result for a row.
#[derive(Debug, Clone)]
pub enum TableAction {
    None,
    CursorSet(BigInt),
    FocusTransaction(TransactionRef),
    SelectSignal(VariableRef),
}

/// Deferred analysis kind (v2).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AnalysisKind {
    Placeholder,
}

/// Deferred analysis parameters (v2).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AnalysisParams {
    pub payload: String,
}

pub trait TableModel: Send + Sync {
    fn schema(&self) -> TableSchema;
    fn row_count(&self) -> usize;
    fn row_id_at(&self, index: usize) -> Option<TableRowId>;
    fn cell(&self, row: TableRowId, col: usize) -> TableCell;
    fn sort_key(&self, row: TableRowId, col: usize) -> TableSortKey;
    fn search_text(&self, row: TableRowId) -> String;
    fn on_activate(&self, row: TableRowId) -> TableAction;
}
