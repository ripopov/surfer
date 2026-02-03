use crate::table::sources::VirtualTableModel;
use crate::transaction_container::{StreamScopeRef, TransactionRef, TransactionStreamRef};
use crate::wave_container::VariableRef;
use egui::RichText;
use num::BigInt;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::Arc;

/// Unique identifier for a table tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TableTileId(pub u64);

/// Stable row identity for selection and caching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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

impl TableModelSpec {
    /// Create a table model instance from this specification.
    ///
    /// Returns `Some(model)` if the model can be created, `None` otherwise.
    /// Currently only `Virtual` is implemented; other variants will be added in later stages.
    pub fn create_model(&self) -> Option<Arc<dyn TableModel>> {
        match self {
            Self::Virtual {
                rows,
                columns,
                seed,
            } => Some(Arc::new(VirtualTableModel::new(*rows, *columns, *seed))),
            // Other model types will be implemented in later stages
            Self::SignalChangeList { .. }
            | Self::TransactionTrace { .. }
            | Self::SearchResults { .. }
            | Self::AnalysisResults { .. }
            | Self::Custom { .. } => None,
        }
    }
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

impl Default for TableViewConfig {
    fn default() -> Self {
        Self {
            title: "Table".to_string(),
            columns: vec![],
            sort: vec![],
            display_filter: TableSearchSpec::default(),
            selection_mode: TableSelectionMode::Single,
            dense_rows: false,
            sticky_header: true,
        }
    }
}

impl Default for TableSearchSpec {
    fn default() -> Self {
        Self {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: String::new(),
        }
    }
}

/// Serializable table tile state (model spec + view config).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableTileState {
    pub spec: TableModelSpec,
    pub config: TableViewConfig,
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
    /// Subsequence matching: "abc" matches "aXbYcZ" but not "bac".
    Fuzzy,
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

impl TableSelection {
    /// Creates an empty selection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if the selection is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Returns the number of selected rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Returns true if the given row is selected.
    #[must_use]
    pub fn contains(&self, row: TableRowId) -> bool {
        self.rows.contains(&row)
    }

    /// Clears all selection.
    pub fn clear(&mut self) {
        self.rows.clear();
        self.anchor = None;
    }

    /// Counts how many selected rows are in the visible set.
    #[must_use]
    pub fn count_visible(&self, visible_rows: &[TableRowId]) -> usize {
        let visible_set: BTreeSet<_> = visible_rows.iter().copied().collect();
        self.rows.intersection(&visible_set).count()
    }

    /// Counts how many selected rows are hidden (not in visible set).
    #[must_use]
    pub fn count_hidden(&self, visible_rows: &[TableRowId]) -> usize {
        self.len() - self.count_visible(visible_rows)
    }
}

/// Result of a selection click operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionUpdate {
    pub selection: TableSelection,
    pub changed: bool,
}

/// Computes selection update when a row is clicked in Single mode.
/// - Clears previous selection and selects the clicked row.
/// - Sets anchor to clicked row.
#[must_use]
pub fn selection_on_click_single(current: &TableSelection, clicked: TableRowId) -> SelectionUpdate {
    let already_selected = current.rows.len() == 1 && current.contains(clicked);
    if already_selected {
        SelectionUpdate {
            selection: current.clone(),
            changed: false,
        }
    } else {
        let mut new_selection = TableSelection::new();
        new_selection.rows.insert(clicked);
        new_selection.anchor = Some(clicked);
        SelectionUpdate {
            selection: new_selection,
            changed: true,
        }
    }
}

/// Computes selection update when a row is clicked in Multi mode (no modifiers).
/// - Clears previous selection and selects the clicked row.
/// - Sets anchor to clicked row.
#[must_use]
pub fn selection_on_click_multi(current: &TableSelection, clicked: TableRowId) -> SelectionUpdate {
    // Same behavior as single mode for plain click
    selection_on_click_single(current, clicked)
}

/// Computes selection update when Ctrl/Cmd+click in Multi mode.
/// - Toggles the clicked row without clearing others.
/// - Sets anchor to clicked row.
#[must_use]
pub fn selection_on_ctrl_click(current: &TableSelection, clicked: TableRowId) -> SelectionUpdate {
    let mut new_selection = current.clone();
    let was_selected = new_selection.rows.contains(&clicked);
    if was_selected {
        new_selection.rows.remove(&clicked);
    } else {
        new_selection.rows.insert(clicked);
    }
    new_selection.anchor = Some(clicked);

    SelectionUpdate {
        selection: new_selection,
        changed: true,
    }
}

/// Computes selection update when Shift+click in Multi mode.
/// - Selects range from anchor to clicked row (inclusive).
/// - If no anchor or anchor not visible, treats clicked row as anchor.
/// - Anchor is preserved (not moved to clicked row).
#[must_use]
pub fn selection_on_shift_click(
    current: &TableSelection,
    clicked: TableRowId,
    visible_rows: &[TableRowId],
) -> SelectionUpdate {
    // Find positions in visible order
    let clicked_pos = visible_rows.iter().position(|&r| r == clicked);
    let anchor = current.anchor;

    // If clicked row is not visible, do nothing
    let Some(clicked_idx) = clicked_pos else {
        return SelectionUpdate {
            selection: current.clone(),
            changed: false,
        };
    };

    // Find anchor position, or use clicked as anchor if not found/visible
    let anchor_idx = anchor.and_then(|a| visible_rows.iter().position(|&r| r == a));

    let (start_idx, end_idx, final_anchor) = match anchor_idx {
        Some(a_idx) => {
            let start = a_idx.min(clicked_idx);
            let end = a_idx.max(clicked_idx);
            (start, end, anchor)
        }
        None => {
            // No anchor or anchor not visible - just select clicked row
            (clicked_idx, clicked_idx, Some(clicked))
        }
    };

    // Build new selection with the range
    let mut new_selection = TableSelection::new();
    for idx in start_idx..=end_idx {
        if let Some(&row_id) = visible_rows.get(idx) {
            new_selection.rows.insert(row_id);
        }
    }
    new_selection.anchor = final_anchor;

    let changed = new_selection != *current;
    SelectionUpdate {
        selection: new_selection,
        changed,
    }
}

/// Formats the selection count for display.
/// Returns empty string if no selection.
#[must_use]
pub fn format_selection_count(total_selected: usize, hidden_count: usize) -> String {
    if total_selected == 0 {
        String::new()
    } else if hidden_count == 0 {
        format!("{total_selected} selected")
    } else {
        format!("{total_selected} selected ({hidden_count} hidden)")
    }
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

// ========================
// Sort Spec Helper Functions
// ========================

/// Computes the new sort spec when a column header is clicked (without Shift).
/// - If column is not in sort: set as primary ascending, clear other sorts
/// - If column is primary: toggle direction
/// - If column is secondary+: promote to primary ascending, clear others
pub fn sort_spec_on_click(
    current: &[TableSortSpec],
    clicked_key: &TableColumnKey,
) -> Vec<TableSortSpec> {
    // Check if the clicked column is already in the sort
    let position = current.iter().position(|spec| &spec.key == clicked_key);

    match position {
        Some(0) => {
            // Column is primary - toggle direction
            vec![TableSortSpec {
                key: clicked_key.clone(),
                direction: match current[0].direction {
                    TableSortDirection::Ascending => TableSortDirection::Descending,
                    TableSortDirection::Descending => TableSortDirection::Ascending,
                },
            }]
        }
        Some(_) => {
            // Column is secondary+ - promote to primary ascending, clear others
            vec![TableSortSpec {
                key: clicked_key.clone(),
                direction: TableSortDirection::Ascending,
            }]
        }
        None => {
            // Column not in sort - set as primary ascending
            vec![TableSortSpec {
                key: clicked_key.clone(),
                direction: TableSortDirection::Ascending,
            }]
        }
    }
}

/// Computes the new sort spec when a column header is Shift+clicked.
/// - If column is not in sort: append as new sort level (ascending)
/// - If column is in sort: toggle its direction (keep position)
pub fn sort_spec_on_shift_click(
    current: &[TableSortSpec],
    clicked_key: &TableColumnKey,
) -> Vec<TableSortSpec> {
    let position = current.iter().position(|spec| &spec.key == clicked_key);

    match position {
        Some(idx) => {
            // Column is in sort - toggle direction, keep position
            let mut result = current.to_vec();
            result[idx].direction = match result[idx].direction {
                TableSortDirection::Ascending => TableSortDirection::Descending,
                TableSortDirection::Descending => TableSortDirection::Ascending,
            };
            result
        }
        None => {
            // Column not in sort - append as new sort level
            let mut result = current.to_vec();
            result.push(TableSortSpec {
                key: clicked_key.clone(),
                direction: TableSortDirection::Ascending,
            });
            result
        }
    }
}

/// Returns the sort indicator text for a column header.
/// - Returns None if column is not in sort
/// - Returns "⬆" or "⬇" for single-column sort
/// - Returns "⬆1", "⬇2", etc. for multi-column sort
///
/// Uses arrow symbols that are included in the default egui fonts.
pub fn sort_indicator(sort: &[TableSortSpec], column_key: &TableColumnKey) -> Option<String> {
    let position = sort.iter().position(|spec| &spec.key == column_key)?;
    let spec = &sort[position];

    let arrow = match spec.direction {
        TableSortDirection::Ascending => "⬆",
        TableSortDirection::Descending => "⬇",
    };

    if sort.len() == 1 {
        // Single-column sort: just the arrow
        Some(arrow.to_string())
    } else {
        // Multi-column sort: arrow + priority number (1-indexed)
        Some(format!("{}{}", arrow, position + 1))
    }
}
