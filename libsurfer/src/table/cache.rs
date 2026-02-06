use super::model::{
    MaterializePurpose, SearchTextMode, TableModel, TableModelKey, TableRowId, TableSearchMode,
    TableSearchSpec, TableSelection, TableSortDirection, TableSortKey, TableSortSpec,
    find_type_search_match,
};
use regex::RegexBuilder;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
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
    /// Durable cache core used by render, selection, scroll and row addressing.
    pub row_ids: Vec<TableRowId>,
    pub row_index: HashMap<TableRowId, usize>,
    /// Optional eager search text cache aligned with `row_ids`.
    /// Lazy models keep this as `None` and probe on demand.
    pub search_texts: Option<Vec<String>>,
}

const SEARCH_PROBE_CHUNK_SIZE: usize = 256;

fn is_cancelled(token: &Option<Arc<AtomicBool>>) -> bool {
    token
        .as_ref()
        .is_some_and(|t| t.load(std::sync::atomic::Ordering::Relaxed))
}

/// Runtime, non-serialized cache handle.
#[derive(Debug)]
pub struct TableCacheEntry {
    pub inner: OnceLock<TableCache>,
    pub cache_key: TableCacheKey,
    pub generation: u64,
    pub revision: u64,
}

impl TableCacheEntry {
    #[must_use]
    pub fn new(cache_key: TableCacheKey, generation: u64, revision: u64) -> Self {
        Self {
            inner: OnceLock::new(),
            cache_key,
            generation,
            revision,
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
    /// Monotonic revision counter, incremented on each `BuildTableCache` request.
    pub table_revision: u64,
    /// Cooperative cancellation token for in-flight async cache builds.
    pub cancel_token: Arc<AtomicBool>,
}

impl std::fmt::Debug for TableRuntimeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TableRuntimeState")
            .field("cache_key", &self.cache_key)
            .field("cache", &self.cache)
            .field("last_error", &self.last_error)
            .field("selection", &self.selection)
            .field("type_search", &self.type_search)
            .field("scroll_state", &self.scroll_state)
            .field("filter_draft", &self.filter_draft)
            .field("hidden_selection_count", &self.hidden_selection_count)
            .field("model", &self.model.as_ref().map(|_| "..."))
            .field("table_revision", &self.table_revision)
            .field(
                "cancel_token",
                &self.cancel_token.load(std::sync::atomic::Ordering::Relaxed),
            )
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

fn filter_rows_with_eager_search_texts(
    model: &dyn TableModel,
    base_rows: &[(TableRowId, usize)],
    filter: &TableFilter,
    cancelled: &Option<Arc<AtomicBool>>,
) -> Result<(Vec<(TableRowId, usize)>, Vec<String>), TableCacheError> {
    let mut filtered_rows = Vec::with_capacity(base_rows.len());
    let mut search_texts = Vec::with_capacity(base_rows.len());

    for chunk in base_rows.chunks(SEARCH_PROBE_CHUNK_SIZE) {
        if is_cancelled(cancelled) {
            return Err(TableCacheError::Cancelled);
        }

        let chunk_row_ids: Vec<TableRowId> = chunk.iter().map(|(row_id, _)| *row_id).collect();
        let search_window =
            model.materialize_window(&chunk_row_ids, &[], MaterializePurpose::SearchProbe);

        for &(row_id, base_index) in chunk {
            let search_text = search_window
                .search_text(row_id)
                .map(str::to_owned)
                .unwrap_or_else(|| model.search_text(row_id));
            if filter.matches(&search_text) {
                filtered_rows.push((row_id, base_index));
                search_texts.push(search_text);
            }
        }
    }

    Ok((filtered_rows, search_texts))
}

fn filter_rows_with_lazy_search_probes(
    model: &dyn TableModel,
    base_rows: &[(TableRowId, usize)],
    filter: &TableFilter,
    cancelled: &Option<Arc<AtomicBool>>,
) -> Result<Vec<(TableRowId, usize)>, TableCacheError> {
    if !filter.is_active() {
        return Ok(base_rows.to_vec());
    }

    let mut filtered_rows = Vec::with_capacity(base_rows.len());
    for chunk in base_rows.chunks(SEARCH_PROBE_CHUNK_SIZE) {
        if is_cancelled(cancelled) {
            return Err(TableCacheError::Cancelled);
        }

        let chunk_row_ids: Vec<TableRowId> = chunk.iter().map(|(row_id, _)| *row_id).collect();
        let search_window =
            model.materialize_window(&chunk_row_ids, &[], MaterializePurpose::SearchProbe);

        for &(row_id, base_index) in chunk {
            let search_text = search_window
                .search_text(row_id)
                .map(str::to_owned)
                .unwrap_or_else(|| model.search_text(row_id));
            if filter.matches(&search_text) {
                filtered_rows.push((row_id, base_index));
            }
        }
    }

    Ok(filtered_rows)
}

fn build_row_entries(
    model: &dyn TableModel,
    filtered_rows: &[(TableRowId, usize)],
    sort_columns: &[(usize, TableSortDirection)],
    cancelled: &Option<Arc<AtomicBool>>,
) -> Result<Vec<RowEntry>, TableCacheError> {
    if sort_columns.is_empty() {
        return Ok(filtered_rows
            .iter()
            .map(|&(row_id, base_index)| RowEntry {
                row_id,
                base_index,
                sort_keys: Vec::new(),
            })
            .collect());
    }

    if is_cancelled(cancelled) {
        return Err(TableCacheError::Cancelled);
    }

    let row_ids: Vec<TableRowId> = filtered_rows.iter().map(|(row_id, _)| *row_id).collect();
    let sort_col_indices: Vec<usize> = sort_columns.iter().map(|(col, _)| *col).collect();
    let sort_window =
        model.materialize_window(&row_ids, &sort_col_indices, MaterializePurpose::SortProbe);

    Ok(filtered_rows
        .iter()
        .map(|&(row_id, base_index)| {
            let sort_keys = sort_columns
                .iter()
                .map(|(col, _)| {
                    sort_window
                        .sort_key(row_id, *col)
                        .cloned()
                        .unwrap_or_else(|| model.sort_key(row_id, *col))
                })
                .collect();
            RowEntry {
                row_id,
                base_index,
                sort_keys,
            }
        })
        .collect())
}

fn type_search_start_index(
    current_selection: &TableSelection,
    row_index: &HashMap<TableRowId, usize>,
    len: usize,
) -> usize {
    current_selection
        .anchor
        .and_then(|anchor| row_index.get(&anchor).copied())
        .map_or(0, |idx| (idx + 1) % len)
}

fn type_search_matches_query(query_lower: &str, text: &str) -> bool {
    let text_lower = text.to_lowercase();
    text_lower.starts_with(query_lower) || text_lower.contains(query_lower)
}

/// Finds the best matching row for type-to-search using eager cache data when available.
/// Falls back to lazy search probes for models that opt out of eager search text storage.
#[must_use]
pub fn find_type_search_match_in_cache(
    query: &str,
    current_selection: &TableSelection,
    cache: &TableCache,
    model: &dyn TableModel,
) -> Option<TableRowId> {
    if query.is_empty() || cache.row_ids.is_empty() {
        return None;
    }

    if let Some(search_texts) = &cache.search_texts {
        return find_type_search_match(
            query,
            current_selection,
            &cache.row_ids,
            search_texts,
            &cache.row_index,
        );
    }

    let query_lower = query.to_lowercase();
    let start_idx =
        type_search_start_index(current_selection, &cache.row_index, cache.row_ids.len());
    let wrapped_indices: Vec<usize> = (start_idx..cache.row_ids.len())
        .chain(0..start_idx)
        .collect();

    for index_chunk in wrapped_indices.chunks(SEARCH_PROBE_CHUNK_SIZE) {
        let chunk_row_ids: Vec<TableRowId> =
            index_chunk.iter().map(|&idx| cache.row_ids[idx]).collect();
        let search_window =
            model.materialize_window(&chunk_row_ids, &[], MaterializePurpose::SearchProbe);

        for &idx in index_chunk {
            let row_id = cache.row_ids[idx];
            let search_text = search_window
                .search_text(row_id)
                .map(str::to_owned)
                .unwrap_or_else(|| model.search_text(row_id));
            if type_search_matches_query(&query_lower, &search_text) {
                return Some(row_id);
            }
        }
    }

    None
}

/// Build a table cache by filtering and sorting the model rows.
///
/// If `cancelled` is provided and set to `true` during execution, the build
/// will return `Err(TableCacheError::Cancelled)` at the next check point.
pub fn build_table_cache(
    model: Arc<dyn TableModel>,
    display_filter: TableSearchSpec,
    view_sort: Vec<TableSortSpec>,
    cancelled: Option<Arc<AtomicBool>>,
) -> Result<TableCache, TableCacheError> {
    let schema = model.schema();
    let filter = TableFilter::new(&display_filter)?;

    let mut sort_columns: Vec<(usize, TableSortDirection)> = Vec::new();
    for spec in &view_sort {
        if let Some(idx) = schema.columns.iter().position(|col| col.key == spec.key) {
            sort_columns.push((idx, spec.direction));
        }
    }

    if is_cancelled(&cancelled) {
        return Err(TableCacheError::Cancelled);
    }

    let base_rows: Vec<(TableRowId, usize)> = (0..model.row_count())
        .filter_map(|index| model.row_id_at(index).map(|row_id| (row_id, index)))
        .collect();

    let (filtered_rows, search_texts) = match model.search_text_mode() {
        SearchTextMode::Eager => {
            let (rows, search_texts) = filter_rows_with_eager_search_texts(
                model.as_ref(),
                &base_rows,
                &filter,
                &cancelled,
            )?;
            (rows, Some(search_texts))
        }
        SearchTextMode::LazyProbe => (
            filter_rows_with_lazy_search_probes(model.as_ref(), &base_rows, &filter, &cancelled)?,
            None,
        ),
    };

    if is_cancelled(&cancelled) {
        return Err(TableCacheError::Cancelled);
    }

    let mut rows = build_row_entries(model.as_ref(), &filtered_rows, &sort_columns, &cancelled)?;

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
        row_index,
        search_texts,
    })
}

// ========================
// Clipboard Formatting Functions
// ========================

/// Formats selected rows as tab-separated values for clipboard.
/// Only includes visible columns in their current display order.
/// Uses `materialize_window` for batch cell materialization.
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

    // Batch-materialize all requested cells
    let materialized =
        model.materialize_window(selected_rows, &col_indices, MaterializePurpose::Clipboard);

    let mut output = String::new();
    for (row_num, &row_id) in selected_rows.iter().enumerate() {
        if row_num > 0 {
            output.push('\n');
        }

        for (col_num, &col_idx) in col_indices.iter().enumerate() {
            if col_num > 0 {
                output.push('\t');
            }

            let cell = materialized
                .cell(row_id, col_idx)
                .cloned()
                .unwrap_or_else(|| model.cell(row_id, col_idx));
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
/// Uses `materialize_window` for batch cell materialization.
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

    let col_indices: Vec<usize> = col_info.iter().map(|(idx, _)| *idx).collect();

    // Batch-materialize all requested cells
    let materialized =
        model.materialize_window(selected_rows, &col_indices, MaterializePurpose::Clipboard);

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

            let cell = materialized
                .cell(row_id, col_idx)
                .cloned()
                .unwrap_or_else(|| model.cell(row_id, col_idx));
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

/// Builds clipboard payload for table copy operations.
///
/// Row export order follows `row_order` (display order from cache), filtered by `selection`.
/// Column export order follows `columns_config` visibility/order. If `columns_config` is empty,
/// all schema columns are exported in schema order.
#[must_use]
pub fn build_table_copy_payload(
    model: &dyn super::model::TableModel,
    schema: &super::model::TableSchema,
    row_order: &[TableRowId],
    selection: &super::model::TableSelection,
    columns_config: &[super::model::TableColumnConfig],
    include_header: bool,
) -> String {
    if selection.is_empty() {
        return String::new();
    }

    let export_columns: Vec<super::model::TableColumnKey> = if columns_config.is_empty() {
        schema
            .columns
            .iter()
            .map(|column| column.key.clone())
            .collect()
    } else {
        super::model::visible_columns(columns_config)
    };

    if export_columns.is_empty() {
        return String::new();
    }

    let selected_rows: Vec<TableRowId> = row_order
        .iter()
        .copied()
        .filter(|row_id| selection.rows.contains(row_id))
        .collect();

    if selected_rows.is_empty() {
        return String::new();
    }

    if include_header {
        format_rows_as_tsv_with_header(model, schema, &selected_rows, &export_columns)
    } else {
        format_rows_as_tsv(model, &selected_rows, &export_columns)
    }
}
