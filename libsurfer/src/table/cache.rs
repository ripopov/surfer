use super::model::{
    TableModel, TableModelKey, TableRowId, TableSearchMode, TableSearchSpec, TableSortDirection,
    TableSortKey, TableSortSpec,
};
use regex::RegexBuilder;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// Debounce delay for filter application (milliseconds).
pub const FILTER_DEBOUNCE_MS: u64 = 200;

/// Draft filter state for debounced live search.
///
/// This struct mirrors `TableSearchSpec` fields (mode, case_sensitive, text) plus
/// a timestamp for debounce tracking. If `TableSearchSpec` gains new fields,
/// update `FilterDraft` accordingly.
#[derive(Debug, Clone)]
pub struct FilterDraft {
    pub text: String,
    pub mode: TableSearchMode,
    pub case_sensitive: bool,
    pub last_changed: Option<Instant>,
}

impl FilterDraft {
    /// Creates a draft from an applied filter spec.
    #[must_use]
    pub fn from_spec(spec: &TableSearchSpec) -> Self {
        Self {
            text: spec.text.clone(),
            mode: spec.mode,
            case_sensitive: spec.case_sensitive,
            last_changed: None,
        }
    }

    /// Converts the draft back to a `TableSearchSpec`.
    #[must_use]
    pub fn to_spec(&self) -> TableSearchSpec {
        TableSearchSpec {
            text: self.text.clone(),
            mode: self.mode,
            case_sensitive: self.case_sensitive,
        }
    }

    /// Returns true if the draft differs from the applied filter.
    #[must_use]
    pub fn is_dirty(&self, applied: &TableSearchSpec) -> bool {
        self.text != applied.text
            || self.mode != applied.mode
            || self.case_sensitive != applied.case_sensitive
    }

    /// Returns true if the debounce period has elapsed since last change.
    /// Accepts `now` parameter for deterministic testing.
    #[must_use]
    pub fn debounce_elapsed(&self, now: Instant) -> bool {
        self.last_changed
            .is_some_and(|t| now.duration_since(t) >= Duration::from_millis(FILTER_DEBOUNCE_MS))
    }

    /// Convenience method using current time.
    #[must_use]
    pub fn debounce_elapsed_now(&self) -> bool {
        self.debounce_elapsed(Instant::now())
    }
}

impl Default for FilterDraft {
    fn default() -> Self {
        Self {
            text: String::new(),
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            last_changed: None,
        }
    }
}

/// Returns true if `needle` characters appear in `haystack` in order (subsequence).
/// For example: "abc" matches "aXbYcZ" but not "bac".
pub fn fuzzy_match(needle: &str, needle_lower: &str, haystack: &str, case_sensitive: bool) -> bool {
    if needle.is_empty() {
        return true;
    }

    let needle_chars: Vec<char> = if case_sensitive {
        needle.chars().collect()
    } else {
        needle_lower.chars().collect()
    };

    let haystack_lower;
    let haystack_chars: Box<dyn Iterator<Item = char>> = if case_sensitive {
        Box::new(haystack.chars())
    } else {
        haystack_lower = haystack.to_lowercase();
        Box::new(haystack_lower.chars())
    };

    let mut needle_idx = 0;
    for hay_char in haystack_chars {
        if needle_idx < needle_chars.len() && hay_char == needle_chars[needle_idx] {
            needle_idx += 1;
        }
    }

    needle_idx == needle_chars.len()
}

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
    pub row_index: HashMap<TableRowId, usize>,
}

/// Runtime, non-serialized cache handle.
#[derive(Debug)]
pub struct TableCacheEntry {
    pub inner: OnceLock<TableCache>,
    pub cache_key: TableCacheKey,
    pub generation: u64,
}

impl TableCacheEntry {
    #[must_use]
    pub fn new(cache_key: TableCacheKey, generation: u64) -> Self {
        Self {
            inner: OnceLock::new(),
            cache_key,
            generation,
        }
    }

    pub fn is_ready(&self) -> bool {
        self.inner.get().is_some()
    }

    pub fn get(&self) -> Option<&TableCache> {
        self.inner.get()
    }

    pub fn set(&self, cache: TableCache) {
        let _ = self.inner.set(cache);
    }
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

/// Type-to-search state stored in TableRuntimeState.
#[derive(Debug, Clone, Default)]
pub struct TypeSearchState {
    /// Accumulated keystrokes for type-to-search.
    pub buffer: String,
    /// Time of last keystroke (for timeout/reset).
    pub last_keystroke: Option<std::time::Instant>,
}

impl TypeSearchState {
    /// Timeout after which buffer resets (1 second).
    pub const TIMEOUT_MS: u128 = 1000;

    /// Adds a character to the buffer, resetting if timeout elapsed.
    pub fn push_char(&mut self, c: char, now: std::time::Instant) -> &str {
        if self.is_timed_out(now) {
            self.buffer.clear();
        }
        self.buffer.push(c);
        self.last_keystroke = Some(now);
        &self.buffer
    }

    /// Clears the buffer.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.last_keystroke = None;
    }

    /// Returns true if buffer should be reset due to timeout.
    #[must_use]
    pub fn is_timed_out(&self, now: std::time::Instant) -> bool {
        self.last_keystroke
            .is_some_and(|last| now.duration_since(last).as_millis() > Self::TIMEOUT_MS)
    }
}

/// Pending scroll operation type.
/// Used to determine scroll behavior after cache rebuild.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingScrollOp {
    /// Sort changed - scroll to keep first selected row visible.
    AfterSort,
    /// Filter changed - scroll to top if selected row hidden.
    AfterFilter,
    /// Activation - ensure activated row is visible.
    AfterActivation(super::model::TableRowId),
}

/// Scroll state stored in TableRuntimeState.
#[derive(Debug, Clone, Default)]
pub struct TableScrollState {
    /// Target row to scroll to (set after sort/filter/activation).
    pub scroll_target: Option<super::model::ScrollTarget>,
    /// Previous generation for detecting waveform reload.
    pub last_generation: u64,
    /// Pending scroll operation (set when sort/filter changes, processed after cache rebuild).
    pub pending_scroll_op: Option<PendingScrollOp>,
}

impl TableScrollState {
    /// Consumes and returns the scroll target, resetting it to None.
    pub fn take_scroll_target(&mut self) -> Option<super::model::ScrollTarget> {
        self.scroll_target.take()
    }

    /// Sets a new scroll target.
    pub fn set_scroll_target(&mut self, target: super::model::ScrollTarget) {
        self.scroll_target = Some(target);
    }

    /// Consumes and returns the pending scroll operation, resetting it to None.
    pub fn take_pending_scroll_op(&mut self) -> Option<PendingScrollOp> {
        self.pending_scroll_op.take()
    }

    /// Sets a pending scroll operation.
    pub fn set_pending_scroll_op(&mut self, op: PendingScrollOp) {
        self.pending_scroll_op = Some(op);
    }
}

/// Runtime state for a table tile (non-serialized).
#[derive(Default)]
pub struct TableRuntimeState {
    pub cache_key: Option<TableCacheKey>,
    pub cache: Option<Arc<TableCacheEntry>>,
    pub last_error: Option<TableCacheError>,
    /// Runtime selection state (keyed by TableRowId for stability across sort/filter).
    pub selection: super::model::TableSelection,
    /// Vertical scroll offset in pixels.
    pub scroll_offset: f32,
    /// Type-to-search state for keyboard navigation.
    pub type_search: TypeSearchState,
    /// Scroll state for tracking scroll targets and generation changes.
    pub scroll_state: TableScrollState,
    /// Draft filter state for debounced live search.
    pub filter_draft: Option<FilterDraft>,
    /// Cached count of selected rows not in the current visible set.
    pub hidden_selection_count: usize,
    /// Cached table model (Arc clone is O(1), avoids per-frame recreation).
    pub model: Option<Arc<dyn super::model::TableModel>>,
}

impl std::fmt::Debug for TableRuntimeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TableRuntimeState")
            .field("cache_key", &self.cache_key)
            .field("cache", &self.cache)
            .field("last_error", &self.last_error)
            .field("selection", &self.selection)
            .field("scroll_offset", &self.scroll_offset)
            .field("type_search", &self.type_search)
            .field("scroll_state", &self.scroll_state)
            .field("filter_draft", &self.filter_draft)
            .field("hidden_selection_count", &self.hidden_selection_count)
            .field("model", &self.model.as_ref().map(|_| "..."))
            .finish()
    }
}

impl TableRuntimeState {
    /// Recomputes `hidden_selection_count` from current selection and cache.
    pub fn update_hidden_count(&mut self) {
        self.hidden_selection_count = self
            .cache
            .as_ref()
            .and_then(|entry| entry.get())
            .map(|cache| {
                self.selection
                    .rows
                    .iter()
                    .filter(|id| !cache.row_index.contains_key(id))
                    .count()
            })
            .unwrap_or(0);
    }
}

struct TableFilter {
    mode: TableSearchMode,
    case_sensitive: bool,
    text: String,
    text_lower: String,
    regex: Option<regex::Regex>,
}

impl TableFilter {
    fn new(spec: &TableSearchSpec) -> Result<Self, TableCacheError> {
        let text = spec.text.clone();
        let text_lower = text.to_lowercase();
        let regex = match spec.mode {
            TableSearchMode::Regex if !text.is_empty() => {
                let built = RegexBuilder::new(&text)
                    .case_insensitive(!spec.case_sensitive)
                    .build()
                    .map_err(|err| TableCacheError::InvalidSearch {
                        pattern: text.clone(),
                        reason: err.to_string(),
                    })?;
                Some(built)
            }
            _ => None,
        };

        Ok(Self {
            mode: spec.mode,
            case_sensitive: spec.case_sensitive,
            text,
            text_lower,
            regex,
        })
    }

    fn is_active(&self) -> bool {
        !self.text.is_empty()
    }

    fn matches(&self, haystack: &str) -> bool {
        if !self.is_active() {
            return true;
        }

        match self.mode {
            TableSearchMode::Contains => {
                if self.case_sensitive {
                    haystack.contains(&self.text)
                } else {
                    haystack.to_lowercase().contains(&self.text_lower)
                }
            }
            TableSearchMode::Exact => {
                if self.case_sensitive {
                    haystack == self.text
                } else {
                    haystack.to_lowercase() == self.text_lower
                }
            }
            TableSearchMode::Regex => self
                .regex
                .as_ref()
                .is_some_and(|regex| regex.is_match(haystack)),
            TableSearchMode::Fuzzy => {
                fuzzy_match(&self.text, &self.text_lower, haystack, self.case_sensitive)
            }
        }
    }
}

struct RowEntry {
    row_id: TableRowId,
    base_index: usize,
    search_text: String,
    sort_keys: Vec<TableSortKey>,
}

fn sort_key_rank(key: &TableSortKey) -> u8 {
    match key {
        TableSortKey::None => 3,
        TableSortKey::Numeric(_) => 0,
        TableSortKey::Text(_) => 1,
        TableSortKey::Bytes(_) => 2,
    }
}

fn compare_sort_keys(a: &TableSortKey, b: &TableSortKey) -> Ordering {
    let rank_a = sort_key_rank(a);
    let rank_b = sort_key_rank(b);
    if rank_a != rank_b {
        return rank_a.cmp(&rank_b);
    }

    match (a, b) {
        (TableSortKey::None, TableSortKey::None) => Ordering::Equal,
        (TableSortKey::Numeric(left), TableSortKey::Numeric(right)) => left.total_cmp(right),
        (TableSortKey::Text(left), TableSortKey::Text(right)) => left.cmp(right),
        (TableSortKey::Bytes(left), TableSortKey::Bytes(right)) => left.cmp(right),
        _ => Ordering::Equal,
    }
}

/// Build a table cache by filtering and sorting the model rows.
pub fn build_table_cache(
    model: Arc<dyn TableModel>,
    display_filter: TableSearchSpec,
    view_sort: Vec<TableSortSpec>,
) -> Result<TableCache, TableCacheError> {
    let schema = model.schema();
    let filter = TableFilter::new(&display_filter)?;

    let mut sort_columns: Vec<(usize, TableSortDirection)> = Vec::new();
    for spec in &view_sort {
        if let Some(idx) = schema.columns.iter().position(|col| col.key == spec.key) {
            sort_columns.push((idx, spec.direction));
        }
    }

    let mut rows: Vec<RowEntry> = Vec::new();
    for index in 0..model.row_count() {
        let Some(row_id) = model.row_id_at(index) else {
            continue;
        };

        let search_text = model.search_text(row_id);
        if !filter.matches(&search_text) {
            continue;
        }

        let sort_keys = sort_columns
            .iter()
            .map(|(col, _)| model.sort_key(row_id, *col))
            .collect::<Vec<_>>();

        rows.push(RowEntry {
            row_id,
            base_index: index,
            search_text,
            sort_keys,
        });
    }

    if !sort_columns.is_empty() {
        rows.sort_by(|left, right| {
            for (idx, (_col, direction)) in sort_columns.iter().enumerate() {
                let ord = compare_sort_keys(&left.sort_keys[idx], &right.sort_keys[idx]);
                if ord != Ordering::Equal {
                    return match direction {
                        TableSortDirection::Ascending => ord,
                        TableSortDirection::Descending => ord.reverse(),
                    };
                }
            }
            left.base_index.cmp(&right.base_index)
        });
    }

    let row_ids: Vec<TableRowId> = rows.iter().map(|row| row.row_id).collect();
    let row_index = row_ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();
    Ok(TableCache {
        row_ids,
        search_texts: rows.iter().map(|row| row.search_text.clone()).collect(),
        row_index,
    })
}

// ========================
// Clipboard Formatting Functions
// ========================

/// Formats selected rows as tab-separated values for clipboard.
/// Only includes visible columns in their current display order.
/// Does not include header row.
pub fn format_rows_as_tsv(
    model: &dyn super::model::TableModel,
    selected_rows: &[TableRowId],
    visible_columns: &[super::model::TableColumnKey],
) -> String {
    if selected_rows.is_empty() {
        return String::new();
    }

    let schema = model.schema();

    // Map column keys to indices
    let col_indices: Vec<usize> = visible_columns
        .iter()
        .filter_map(|key| schema.columns.iter().position(|col| &col.key == key))
        .collect();

    let mut output = String::new();
    for (row_num, &row_id) in selected_rows.iter().enumerate() {
        if row_num > 0 {
            output.push('\n');
        }

        for (col_num, &col_idx) in col_indices.iter().enumerate() {
            if col_num > 0 {
                output.push('\t');
            }

            let cell = model.cell(row_id, col_idx);
            let text = match cell {
                super::model::TableCell::Text(s) => s,
                super::model::TableCell::RichText(rt) => rt.text().to_string(),
            };

            // Escape tabs and newlines in cell values
            let escaped = text.replace(['\t', '\n'], " ");
            output.push_str(&escaped);
        }
    }

    output
}

/// Formats selected rows with a header row as tab-separated values.
/// Includes column labels as first row.
pub fn format_rows_as_tsv_with_header(
    model: &dyn super::model::TableModel,
    schema: &super::model::TableSchema,
    selected_rows: &[TableRowId],
    visible_columns: &[super::model::TableColumnKey],
) -> String {
    if selected_rows.is_empty() {
        return String::new();
    }

    // Map column keys to indices and labels
    let col_info: Vec<(usize, &str)> = visible_columns
        .iter()
        .filter_map(|key| {
            schema
                .columns
                .iter()
                .position(|col| &col.key == key)
                .map(|idx| (idx, schema.columns[idx].label.as_str()))
        })
        .collect();

    let mut output = String::new();

    // Header row
    for (col_num, (_, label)) in col_info.iter().enumerate() {
        if col_num > 0 {
            output.push('\t');
        }
        output.push_str(label);
    }

    // Data rows
    for &row_id in selected_rows {
        output.push('\n');

        for (col_num, &(col_idx, _)) in col_info.iter().enumerate() {
            if col_num > 0 {
                output.push('\t');
            }

            let cell = model.cell(row_id, col_idx);
            let text = match cell {
                super::model::TableCell::Text(s) => s,
                super::model::TableCell::RichText(rt) => rt.text().to_string(),
            };

            let escaped = text.replace(['\t', '\n'], " ");
            output.push_str(&escaped);
        }
    }

    output
}
