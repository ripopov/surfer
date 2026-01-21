use super::super::model::{
    ScrollTarget, TableColumnKey, TableModel, TableModelKey, TableRowId, TableSearchMode,
    TableSearchSpec, TableSelection, TableSortSpec,
};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// Debounce delay for filter application (milliseconds).
pub const FILTER_DEBOUNCE_MS: u64 = 200;

/// Draft filter state for debounced live search.
///
/// This struct mirrors `TableSearchSpec` fields (mode, case_sensitive, text, column) plus
/// a timestamp for debounce tracking. If `TableSearchSpec` gains new fields,
/// update `FilterDraft` accordingly.
#[derive(Debug, Clone)]
pub struct FilterDraft {
    pub text: String,
    pub mode: TableSearchMode,
    pub case_sensitive: bool,
    pub column: Option<TableColumnKey>,
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
            column: spec.column.clone(),
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
            column: self.column.clone(),
        }
    }

    /// Returns true if the draft differs from the applied filter.
    #[must_use]
    pub fn is_dirty(&self, applied: &TableSearchSpec) -> bool {
        self.text != applied.text
            || self.mode != applied.mode
            || self.case_sensitive != applied.case_sensitive
            || self.column != applied.column
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
            column: None,
            last_changed: None,
        }
    }
}

/// Cache key for table data.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TableCacheKey {
    pub model_key: TableModelKey,
    pub display_filter: TableSearchSpec,
    pub pinned_filters: Vec<TableSearchSpec>,
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
    AfterActivation(TableRowId),
}

/// Scroll state stored in TableRuntimeState.
#[derive(Debug, Clone, Default)]
pub struct TableScrollState {
    /// Target row to scroll to (set after sort/filter/activation).
    pub scroll_target: Option<ScrollTarget>,
    /// Previous generation for detecting waveform reload.
    pub last_generation: u64,
    /// Pending scroll operation (set when sort/filter changes, processed after cache rebuild).
    pub pending_scroll_op: Option<PendingScrollOp>,
}

impl TableScrollState {
    /// Consumes and returns the scroll target, resetting it to None.
    pub fn take_scroll_target(&mut self) -> Option<ScrollTarget> {
        self.scroll_target.take()
    }

    /// Sets a new scroll target.
    pub fn set_scroll_target(&mut self, target: ScrollTarget) {
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
    pub selection: TableSelection,
    /// Type-to-search state for keyboard navigation.
    pub type_search: TypeSearchState,
    /// Scroll state for tracking scroll targets and generation changes.
    pub scroll_state: TableScrollState,
    /// Draft filter state for debounced live search.
    pub filter_draft: Option<FilterDraft>,
    /// Cached count of selected rows not in the current visible set.
    pub hidden_selection_count: usize,
    /// Cached table model (Arc clone is O(1), avoids per-frame recreation).
    pub model: Option<Arc<dyn TableModel>>,
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
