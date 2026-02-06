use super::cache::TableCacheError;
use crate::config::SurferTheme;
use crate::table::sources::{
    MultiSignalChangeListModel, SignalAnalysisResultsModel, SignalChangeListModel,
    TransactionTraceModelWithData, VirtualTableModel,
};
use crate::time::{TimeFormat, TimeUnit};
use crate::transaction_container::{TransactionRef, TransactionStreamRef};
use crate::translation::TranslatorList;
use crate::wave_container::{VariableRef, VariableRefExt};
use crate::wave_data::WaveData;
use egui::RichText;
use num::BigInt;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
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
    MultiSignalChangeList {
        variables: Vec<MultiSignalEntry>,
    },
    /// Transaction trace table for a specific generator.
    /// Each generator has its own attribute schema, so tables are per-generator.
    TransactionTrace {
        generator: TransactionStreamRef,
    },
    /// Source-level search that produces a derived table model from waveform data.
    /// Named `source_query` to distinguish from view-level `display_filter`.
    SearchResults {
        source_query: TableSearchSpec,
    },
    /// Derived analysis models and their typed parameters.
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultiSignalEntry {
    pub variable: VariableRef,
    pub field: Vec<String>,
}

impl TableModelSpec {
    /// Create a table model instance from this specification.
    pub fn create_model(
        &self,
        ctx: &TableModelContext<'_>,
    ) -> Result<Arc<dyn TableModel>, TableCacheError> {
        match self {
            Self::Virtual {
                rows,
                columns,
                seed,
            } => Ok(Arc::new(VirtualTableModel::new(*rows, *columns, *seed))),
            Self::SignalChangeList { variable, field } => {
                SignalChangeListModel::new(variable.clone(), field.clone(), ctx)
                    .map(|model| Arc::new(model) as Arc<dyn TableModel>)
            }
            Self::TransactionTrace { generator } => {
                TransactionTraceModelWithData::new(generator.clone(), ctx)
                    .map(|model| Arc::new(model) as Arc<dyn TableModel>)
            }
            Self::MultiSignalChangeList { variables } => {
                MultiSignalChangeListModel::new(variables.clone(), ctx)
                    .map(|model| Arc::new(model) as Arc<dyn TableModel>)
            }
            Self::AnalysisResults {
                kind: AnalysisKind::SignalAnalysisV1,
                params: AnalysisParams::SignalAnalysisV1 { config },
            } => SignalAnalysisResultsModel::new(config.clone(), ctx)
                .map(|model| Arc::new(model) as Arc<dyn TableModel>),
            // Other model types will be implemented in later stages
            Self::SearchResults { .. } | Self::AnalysisResults { .. } | Self::Custom { .. } => {
                Err(TableCacheError::ModelNotFound {
                    description: "Model type not yet implemented".to_string(),
                })
            }
        }
    }

    /// Returns a default view configuration for this model type.
    #[must_use]
    pub fn default_view_config(&self, _ctx: &TableModelContext<'_>) -> TableViewConfig {
        match self {
            Self::SignalChangeList { variable, field } => {
                let mut config = TableViewConfig::default();
                let mut title = variable.full_path_string();
                if !field.is_empty() {
                    title.push('.');
                    title.push_str(&field.join("."));
                }
                config.title = format!("Signal change list: {title}");
                config.sort = vec![TableSortSpec {
                    key: TableColumnKey::Str("time".to_string()),
                    direction: TableSortDirection::Ascending,
                }];
                config.selection_mode = TableSelectionMode::Single;
                config.activate_on_select = true;
                config
            }
            Self::TransactionTrace { generator } => TableViewConfig {
                title: format!("Transactions: {}", generator.name),
                // Default sort: ascending by start time
                sort: vec![TableSortSpec {
                    key: TableColumnKey::Str("start".to_string()),
                    direction: TableSortDirection::Ascending,
                }],
                selection_mode: TableSelectionMode::Single,
                activate_on_select: true,
                ..Default::default()
            },
            Self::MultiSignalChangeList { .. } => TableViewConfig {
                title: "Multi-signal change list".to_string(),
                sort: vec![TableSortSpec {
                    key: TableColumnKey::Str("time".to_string()),
                    direction: TableSortDirection::Ascending,
                }],
                selection_mode: TableSelectionMode::Single,
                activate_on_select: true,
                ..Default::default()
            },
            Self::AnalysisResults {
                kind: AnalysisKind::SignalAnalysisV1,
                params: AnalysisParams::SignalAnalysisV1 { config },
            } => TableViewConfig {
                title: format!(
                    "Signal Analysis: {}",
                    config.sampling.signal.full_path_string()
                ),
                sort: vec![TableSortSpec {
                    key: TableColumnKey::Str("interval_end".to_string()),
                    direction: TableSortDirection::Ascending,
                }],
                selection_mode: TableSelectionMode::Single,
                activate_on_select: true,
                ..Default::default()
            },
            _ => TableViewConfig::default(),
        }
    }
}

/// Context for building table models from specifications.
pub struct TableModelContext<'a> {
    pub waves: Option<&'a WaveData>,
    pub translators: &'a TranslatorList,
    pub wanted_timeunit: TimeUnit,
    pub time_format: TimeFormat,
    pub theme: &'a SurferTheme,
    pub cache_generation: u64,
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
    /// When true, selecting a row immediately triggers activation (on_activate).
    /// Useful for tables where row selection should update external state (e.g., cursor).
    #[serde(default)]
    pub activate_on_select: bool,
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
            activate_on_select: false,
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
    row_index: &HashMap<TableRowId, usize>,
) -> SelectionUpdate {
    // Find positions in visible order
    let anchor = current.anchor;

    // If clicked row is not visible, do nothing
    let Some(&clicked_idx) = row_index.get(&clicked) else {
        return SelectionUpdate {
            selection: current.clone(),
            changed: false,
        };
    };

    // Find anchor position, or use clicked as anchor if not found/visible
    let anchor_idx = anchor.and_then(|a| row_index.get(&a).copied());

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

/// Purpose hint for batch materialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MaterializePurpose {
    Render,
    SortProbe,
    SearchProbe,
    Clipboard,
}

/// Search text strategy for cache building and type-to-search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SearchTextMode {
    /// Build and retain eager `search_texts` aligned with visible row order.
    Eager,
    /// Probe search text on demand; cache stores row ordering/index only.
    LazyProbe,
}

/// Batch materialization output for row/column probe requests.
#[derive(Debug, Clone, Default)]
pub struct MaterializedWindow {
    cells: HashMap<(TableRowId, usize), TableCell>,
    sort_keys: HashMap<(TableRowId, usize), TableSortKey>,
    search_texts: HashMap<TableRowId, String>,
}

impl MaterializedWindow {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_cell(&mut self, row: TableRowId, col: usize, cell: TableCell) {
        self.cells.insert((row, col), cell);
    }

    #[must_use]
    pub fn cell(&self, row: TableRowId, col: usize) -> Option<&TableCell> {
        self.cells.get(&(row, col))
    }

    pub fn insert_sort_key(&mut self, row: TableRowId, col: usize, sort_key: TableSortKey) {
        self.sort_keys.insert((row, col), sort_key);
    }

    #[must_use]
    pub fn sort_key(&self, row: TableRowId, col: usize) -> Option<&TableSortKey> {
        self.sort_keys.get(&(row, col))
    }

    pub fn insert_search_text(&mut self, row: TableRowId, text: String) {
        self.search_texts.insert(row, text);
    }

    #[must_use]
    pub fn search_text(&self, row: TableRowId) -> Option<&str> {
        self.search_texts.get(&row).map(String::as_str)
    }
}

/// Config for signal-analysis table model.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SignalAnalysisConfig {
    pub sampling: SignalAnalysisSamplingConfig,
    pub signals: Vec<SignalAnalysisSignal>,
    #[serde(default)]
    pub run_revision: u64,
}

/// Sampling signal selection for signal analysis.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SignalAnalysisSamplingConfig {
    pub signal: VariableRef,
}

/// Per-signal analysis configuration.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SignalAnalysisSignal {
    pub variable: VariableRef,
    #[serde(default)]
    pub field: Vec<String>,
    pub translator: String,
}

/// Resolved sampling behavior (derived from sampling signal metadata).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SignalAnalysisSamplingMode {
    Event,
    PosEdge,
    AnyChange,
}

/// Kind of analysis model.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AnalysisKind {
    Placeholder,
    SignalAnalysisV1,
}

/// Parameters for analysis model kinds.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AnalysisParams {
    Placeholder { payload: String },
    SignalAnalysisV1 { config: SignalAnalysisConfig },
}

pub trait TableModel: Send + Sync {
    fn schema(&self) -> TableSchema;
    fn row_count(&self) -> usize;
    fn row_id_at(&self, index: usize) -> Option<TableRowId>;
    fn search_text_mode(&self) -> SearchTextMode {
        SearchTextMode::Eager
    }
    fn materialize_window(
        &self,
        row_ids: &[TableRowId],
        visible_cols: &[usize],
        purpose: MaterializePurpose,
    ) -> MaterializedWindow {
        let mut window = MaterializedWindow::new();
        match purpose {
            MaterializePurpose::Render | MaterializePurpose::Clipboard => {
                for &row_id in row_ids {
                    for &col in visible_cols {
                        window.insert_cell(row_id, col, self.cell(row_id, col));
                    }
                }
            }
            MaterializePurpose::SortProbe => {
                for &row_id in row_ids {
                    for &col in visible_cols {
                        window.insert_sort_key(row_id, col, self.sort_key(row_id, col));
                    }
                }
            }
            MaterializePurpose::SearchProbe => {
                for &row_id in row_ids {
                    window.insert_search_text(row_id, self.search_text(row_id));
                }
            }
        }
        window
    }
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

// ========================
// Keyboard Navigation Functions
// ========================

/// Result of a keyboard navigation operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationResult {
    /// The row to navigate to (select and scroll into view).
    pub target_row: Option<TableRowId>,
    /// Whether the operation changed the selection.
    pub selection_changed: bool,
    /// New selection state (if changed).
    pub new_selection: Option<TableSelection>,
}

/// Helper to find the position of the anchor or last selected row in visible rows.
fn find_anchor_position(
    selection: &TableSelection,
    _visible_rows: &[TableRowId],
    row_index: &HashMap<TableRowId, usize>,
) -> Option<usize> {
    // First try anchor
    if let Some(anchor) = selection.anchor
        && let Some(&pos) = row_index.get(&anchor)
    {
        return Some(pos);
    }
    // Fallback to any selected row (use the last one in BTreeSet order)
    selection
        .rows
        .iter()
        .rev()
        .find_map(|r| row_index.get(r).copied())
}

/// Computes the target row when pressing Up arrow.
/// Returns the previous row in display order, or stays at first row.
/// With no selection, selects the last row.
#[must_use]
pub fn navigate_up(
    current_selection: &TableSelection,
    visible_rows: &[TableRowId],
    row_index: &HashMap<TableRowId, usize>,
) -> NavigationResult {
    if visible_rows.is_empty() {
        return NavigationResult {
            target_row: None,
            selection_changed: false,
            new_selection: None,
        };
    }

    let current_pos = find_anchor_position(current_selection, visible_rows, row_index);

    let target_idx = match current_pos {
        Some(0) => 0, // Already at first row, stay there
        Some(pos) => pos - 1,
        None => visible_rows.len() - 1, // No selection: go to last row
    };

    let target_row = visible_rows[target_idx];

    // Check if selection actually changed
    if current_selection.len() == 1
        && current_selection.contains(target_row)
        && current_selection.anchor == Some(target_row)
    {
        return NavigationResult {
            target_row: Some(target_row),
            selection_changed: false,
            new_selection: None,
        };
    }

    let mut new_selection = TableSelection::new();
    new_selection.rows.insert(target_row);
    new_selection.anchor = Some(target_row);

    NavigationResult {
        target_row: Some(target_row),
        selection_changed: true,
        new_selection: Some(new_selection),
    }
}

/// Computes the target row when pressing Down arrow.
/// Returns the next row in display order, or stays at last row.
/// With no selection, selects the first row.
#[must_use]
pub fn navigate_down(
    current_selection: &TableSelection,
    visible_rows: &[TableRowId],
    row_index: &HashMap<TableRowId, usize>,
) -> NavigationResult {
    if visible_rows.is_empty() {
        return NavigationResult {
            target_row: None,
            selection_changed: false,
            new_selection: None,
        };
    }

    let current_pos = find_anchor_position(current_selection, visible_rows, row_index);

    let target_idx = match current_pos {
        Some(pos) if pos >= visible_rows.len() - 1 => visible_rows.len() - 1, // At last row, stay
        Some(pos) => pos + 1,
        None => 0, // No selection: go to first row
    };

    let target_row = visible_rows[target_idx];

    // Check if selection actually changed
    if current_selection.len() == 1
        && current_selection.contains(target_row)
        && current_selection.anchor == Some(target_row)
    {
        return NavigationResult {
            target_row: Some(target_row),
            selection_changed: false,
            new_selection: None,
        };
    }

    let mut new_selection = TableSelection::new();
    new_selection.rows.insert(target_row);
    new_selection.anchor = Some(target_row);

    NavigationResult {
        target_row: Some(target_row),
        selection_changed: true,
        new_selection: Some(new_selection),
    }
}

/// Computes the target row when pressing Page Up.
/// Moves up by `page_size` rows, or to first row if fewer remain.
#[must_use]
pub fn navigate_page_up(
    current_selection: &TableSelection,
    visible_rows: &[TableRowId],
    row_index: &HashMap<TableRowId, usize>,
    page_size: usize,
) -> NavigationResult {
    if visible_rows.is_empty() {
        return NavigationResult {
            target_row: None,
            selection_changed: false,
            new_selection: None,
        };
    }

    let current_pos = find_anchor_position(current_selection, visible_rows, row_index);

    let target_idx = match current_pos {
        Some(pos) => pos.saturating_sub(page_size),
        None => visible_rows.len() - 1, // No selection: go to last row
    };

    let target_row = visible_rows[target_idx];

    let mut new_selection = TableSelection::new();
    new_selection.rows.insert(target_row);
    new_selection.anchor = Some(target_row);

    NavigationResult {
        target_row: Some(target_row),
        selection_changed: true,
        new_selection: Some(new_selection),
    }
}

/// Computes the target row when pressing Page Down.
/// Moves down by `page_size` rows, or to last row if fewer remain.
#[must_use]
pub fn navigate_page_down(
    current_selection: &TableSelection,
    visible_rows: &[TableRowId],
    row_index: &HashMap<TableRowId, usize>,
    page_size: usize,
) -> NavigationResult {
    if visible_rows.is_empty() {
        return NavigationResult {
            target_row: None,
            selection_changed: false,
            new_selection: None,
        };
    }

    let current_pos = find_anchor_position(current_selection, visible_rows, row_index);

    let target_idx = match current_pos {
        Some(pos) => (pos + page_size).min(visible_rows.len() - 1),
        None => 0, // No selection: go to first row
    };

    let target_row = visible_rows[target_idx];

    let mut new_selection = TableSelection::new();
    new_selection.rows.insert(target_row);
    new_selection.anchor = Some(target_row);

    NavigationResult {
        target_row: Some(target_row),
        selection_changed: true,
        new_selection: Some(new_selection),
    }
}

/// Computes the target row when pressing Home.
/// Returns the first row in display order.
#[must_use]
pub fn navigate_home(visible_rows: &[TableRowId]) -> NavigationResult {
    if visible_rows.is_empty() {
        return NavigationResult {
            target_row: None,
            selection_changed: false,
            new_selection: None,
        };
    }

    let target_row = visible_rows[0];

    let mut new_selection = TableSelection::new();
    new_selection.rows.insert(target_row);
    new_selection.anchor = Some(target_row);

    NavigationResult {
        target_row: Some(target_row),
        selection_changed: true,
        new_selection: Some(new_selection),
    }
}

/// Computes the target row when pressing End.
/// Returns the last row in display order.
#[must_use]
pub fn navigate_end(visible_rows: &[TableRowId]) -> NavigationResult {
    if visible_rows.is_empty() {
        return NavigationResult {
            target_row: None,
            selection_changed: false,
            new_selection: None,
        };
    }

    let target_row = visible_rows[visible_rows.len() - 1];

    let mut new_selection = TableSelection::new();
    new_selection.rows.insert(target_row);
    new_selection.anchor = Some(target_row);

    NavigationResult {
        target_row: Some(target_row),
        selection_changed: true,
        new_selection: Some(new_selection),
    }
}

/// Computes the result of Shift+navigation (extends selection).
/// Used with Shift+Up, Shift+Down, Shift+Page Up/Down, Shift+Home/End.
/// Selects all rows between anchor and target (inclusive).
#[must_use]
pub fn navigate_extend_selection(
    current_selection: &TableSelection,
    target_row: TableRowId,
    visible_rows: &[TableRowId],
    row_index: &HashMap<TableRowId, usize>,
) -> NavigationResult {
    let Some(&target_idx) = row_index.get(&target_row) else {
        return NavigationResult {
            target_row: None,
            selection_changed: false,
            new_selection: None,
        };
    };

    // Get anchor position, or use target as anchor if no anchor exists
    let anchor = current_selection.anchor;
    let anchor_idx = anchor.and_then(|a| row_index.get(&a).copied());

    let (start_idx, end_idx, final_anchor) = match anchor_idx {
        Some(a_idx) => {
            let start = a_idx.min(target_idx);
            let end = a_idx.max(target_idx);
            (start, end, anchor)
        }
        None => {
            // No anchor - just select target and set as anchor
            (target_idx, target_idx, Some(target_row))
        }
    };

    let mut new_selection = TableSelection::new();
    for idx in start_idx..=end_idx {
        if let Some(&row_id) = visible_rows.get(idx) {
            new_selection.rows.insert(row_id);
        }
    }
    new_selection.anchor = final_anchor;

    NavigationResult {
        target_row: Some(target_row),
        selection_changed: true,
        new_selection: Some(new_selection),
    }
}

/// Finds the best matching row for type-to-search.
/// Returns the first row whose search_text starts with or contains the query.
/// Prefers rows after the current selection for "find next" behavior.
/// Case-insensitive matching.
#[must_use]
pub fn find_type_search_match(
    query: &str,
    current_selection: &TableSelection,
    visible_rows: &[TableRowId],
    search_texts: &[String],
    row_index: &HashMap<TableRowId, usize>,
) -> Option<TableRowId> {
    if query.is_empty() || visible_rows.is_empty() {
        return None;
    }

    let query_lower = query.to_lowercase();

    // Find current selection position to start search from
    let start_idx = current_selection
        .anchor
        .and_then(|a| row_index.get(&a).copied())
        .map(|pos| (pos + 1) % visible_rows.len())
        .unwrap_or(0);

    // Search starting from position after current selection, wrapping around
    let indices = (start_idx..visible_rows.len()).chain(0..start_idx);

    for idx in indices {
        if idx < search_texts.len() {
            let text_lower = search_texts[idx].to_lowercase();

            // First try prefix match, then contains match
            if text_lower.starts_with(&query_lower) || text_lower.contains(&query_lower) {
                return Some(visible_rows[idx]);
            }
        }
    }

    None
}

// ========================
// Stage 10: Scroll Target Functions
// ========================

/// Scroll target after a table operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScrollTarget {
    /// Keep current scroll position.
    Preserve,
    /// Scroll to make the given row visible.
    ToRow(TableRowId),
    /// Scroll to the top of the table.
    ToTop,
    /// Scroll to the bottom of the table.
    ToBottom,
}

/// Computes scroll target after sort change.
/// - If selection exists and first selected row is visible, scroll to it.
/// - Otherwise, preserve approximate scroll position.
#[must_use]
pub fn scroll_target_after_sort(
    selection: &TableSelection,
    _new_visible_rows: &[TableRowId],
    row_index: &HashMap<TableRowId, usize>,
) -> ScrollTarget {
    if selection.is_empty() {
        return ScrollTarget::Preserve;
    }

    // Find first selected row in new visible order using index
    let first_selected = selection
        .rows
        .iter()
        .filter_map(|r| row_index.get(r).map(|&idx| (idx, *r)))
        .min_by_key(|&(idx, _)| idx);

    if let Some((_, row_id)) = first_selected {
        return ScrollTarget::ToRow(row_id);
    }

    // Selected rows not in visible set (should not happen unless filter hid them)
    ScrollTarget::Preserve
}

/// Computes scroll target after filter change.
/// - If selected row is still visible, scroll to it.
/// - If selected row is hidden, scroll to top.
/// - If no selection, preserve position.
#[must_use]
pub fn scroll_target_after_filter(
    selection: &TableSelection,
    _new_visible_rows: &[TableRowId],
    row_index: &HashMap<TableRowId, usize>,
) -> ScrollTarget {
    if selection.is_empty() {
        return ScrollTarget::Preserve;
    }

    // Find first visible selected row using index
    let first_visible_selected = selection
        .rows
        .iter()
        .filter_map(|r| row_index.get(r).map(|&idx| (idx, *r)))
        .min_by_key(|&(idx, _)| idx);

    if let Some((_, row_id)) = first_visible_selected {
        return ScrollTarget::ToRow(row_id);
    }

    // All selected rows are hidden - scroll to top
    ScrollTarget::ToTop
}

/// Computes scroll target after activation.
/// Always scrolls to make activated row visible.
#[must_use]
pub fn scroll_target_after_activation(activated_row: TableRowId) -> ScrollTarget {
    ScrollTarget::ToRow(activated_row)
}

// ========================
// Stage 10: Column Configuration Functions
// ========================

/// Minimum column width in pixels to prevent invisible columns.
pub const MIN_COLUMN_WIDTH: f32 = 20.0;

/// Result of a column resize operation.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnResizeResult {
    /// Updated column configurations.
    pub columns: Vec<TableColumnConfig>,
    /// Whether any column was actually resized.
    pub changed: bool,
}

/// Updates column width in configuration.
/// Enforces minimum width constraint.
#[must_use]
pub fn resize_column(
    columns: &[TableColumnConfig],
    column_key: &TableColumnKey,
    new_width: f32,
    min_width: f32,
) -> ColumnResizeResult {
    let min_width = min_width.max(MIN_COLUMN_WIDTH);
    let clamped_width = new_width.max(min_width);

    let mut result = columns.to_vec();
    let mut changed = false;

    for col in &mut result {
        if &col.key == column_key {
            let current_width = col.width.unwrap_or(100.0);
            // Use approximate equality for floats
            if (current_width - clamped_width).abs() > 0.1 {
                col.width = Some(clamped_width);
                changed = true;
            }
            break;
        }
    }

    ColumnResizeResult {
        columns: result,
        changed,
    }
}

/// Toggles column visibility.
/// Returns updated column configuration.
/// Will not hide the last visible column.
#[must_use]
pub fn toggle_column_visibility(
    columns: &[TableColumnConfig],
    column_key: &TableColumnKey,
) -> Vec<TableColumnConfig> {
    let visible_count = columns.iter().filter(|c| c.visible).count();

    let mut result = columns.to_vec();
    for col in &mut result {
        if &col.key == column_key {
            // Don't hide the last visible column
            if col.visible && visible_count <= 1 {
                break;
            }
            col.visible = !col.visible;
            break;
        }
    }

    result
}

/// Returns list of visible column keys in order.
#[must_use]
pub fn visible_columns(columns: &[TableColumnConfig]) -> Vec<TableColumnKey> {
    columns
        .iter()
        .filter(|c| c.visible)
        .map(|c| c.key.clone())
        .collect()
}

/// Returns list of hidden column keys.
#[must_use]
pub fn hidden_columns(columns: &[TableColumnConfig]) -> Vec<TableColumnKey> {
    columns
        .iter()
        .filter(|c| !c.visible)
        .map(|c| c.key.clone())
        .collect()
}

// ========================
// Stage 10: Generation Tracking
// ========================

/// Checks if cache generation has changed and selection should be cleared.
/// Returns true if generation changed (indicating waveform reload).
#[must_use]
pub fn should_clear_selection_on_generation_change(
    current_generation: u64,
    previous_generation: u64,
) -> bool {
    current_generation != previous_generation
}
