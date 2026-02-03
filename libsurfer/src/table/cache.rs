use super::model::{
    TableModel, TableModelKey, TableRowId, TableSearchMode, TableSearchSpec, TableSortDirection,
    TableSortKey, TableSortSpec,
};
use regex::RegexBuilder;
use std::cmp::Ordering;
use std::sync::{Arc, OnceLock};

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

/// Runtime state for a table tile (non-serialized).
#[derive(Debug, Default)]
pub struct TableRuntimeState {
    pub cache_key: Option<TableCacheKey>,
    pub cache: Option<Arc<TableCacheEntry>>,
    pub last_error: Option<TableCacheError>,
    /// Runtime selection state (keyed by TableRowId for stability across sort/filter).
    pub selection: super::model::TableSelection,
    /// Vertical scroll offset in pixels.
    pub scroll_offset: f32,
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
        (TableSortKey::Numeric(left), TableSortKey::Numeric(right)) => {
            left.partial_cmp(right).unwrap_or(Ordering::Equal)
        }
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

    Ok(TableCache {
        row_ids: rows.iter().map(|row| row.row_id).collect(),
        search_texts: rows.iter().map(|row| row.search_text.clone()).collect(),
        sort_keys: rows.iter().map(|row| row.sort_keys.clone()).collect(),
    })
}
