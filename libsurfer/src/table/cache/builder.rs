use super::super::model::{
    MaterializePurpose, MaterializedWindow, SearchTextMode, TableCell, TableModel, TableRowId,
    TableSearchMode, TableSearchSpec, TableSelection, TableSortDirection, TableSortKey,
    TableSortSpec, find_type_search_match, normalize_search_specs,
};
use super::state::{TableCache, TableCacheError};
use regex::RegexBuilder;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

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

const SEARCH_PROBE_CHUNK_SIZE: usize = 256;

fn is_cancelled(token: &Option<Arc<AtomicBool>>) -> bool {
    token
        .as_ref()
        .is_some_and(|t| t.load(std::sync::atomic::Ordering::Relaxed))
}

#[derive(Debug, Clone)]
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
        (TableSortKey::Text(left), TableSortKey::Text(right)) => numeric_sort::cmp(left, right),
        (TableSortKey::Bytes(left), TableSortKey::Bytes(right)) => left.cmp(right),
        _ => Ordering::Equal,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClauseTarget {
    AllColumns,
    Column(usize),
}

#[derive(Debug, Clone)]
struct CompiledFilterClause {
    matcher: TableFilter,
    target: ClauseTarget,
}

fn compile_filter_clauses(
    specs: &[TableSearchSpec],
    model: &dyn TableModel,
) -> Result<Vec<CompiledFilterClause>, TableCacheError> {
    let schema = model.schema();
    specs.iter().try_fold(Vec::new(), |mut compiled, spec| {
        let target = match &spec.column {
            None => Some(ClauseTarget::AllColumns),
            Some(column_key) => schema
                .columns
                .iter()
                .position(|column| column.key == *column_key)
                .map(ClauseTarget::Column),
        };

        if let Some(target) = target {
            let matcher = TableFilter::new(spec)?;
            if matcher.is_active() {
                compiled.push(CompiledFilterClause { matcher, target });
            }
        }

        Ok(compiled)
    })
}

fn table_cell_to_search_text(cell: TableCell) -> String {
    match cell {
        TableCell::Text(text) => text,
        TableCell::RichText(text) => text.text().to_string(),
    }
}

fn probe_row_search_text(
    model: &dyn TableModel,
    row_id: TableRowId,
    search_window: Option<&MaterializedWindow>,
) -> String {
    search_window
        .and_then(|window| window.search_text(row_id))
        .map(str::to_owned)
        .unwrap_or_else(|| model.search_text(row_id))
}

fn collect_filtered_rows(
    model: &dyn TableModel,
    base_rows: &[(TableRowId, usize)],
    clauses: &[CompiledFilterClause],
    search_text_mode: SearchTextMode,
    cancelled: &Option<Arc<AtomicBool>>,
) -> Result<
    (
        Vec<(TableRowId, usize)>,
        Option<HashMap<TableRowId, String>>,
        bool,
    ),
    TableCacheError,
> {
    let mut filtered_rows = Vec::with_capacity(base_rows.len());
    let mut eager_search_texts = (search_text_mode == SearchTextMode::Eager).then(HashMap::new);
    let has_all_columns_clause = clauses
        .iter()
        .any(|clause| clause.target == ClauseTarget::AllColumns);
    let distinct_column_indices: Vec<usize> = clauses
        .iter()
        .filter_map(|clause| match clause.target {
            ClauseTarget::Column(index) => Some(index),
            ClauseTarget::AllColumns => None,
        })
        .fold(Vec::new(), |mut indices, index| {
            if !indices.contains(&index) {
                indices.push(index);
            }
            indices
        });

    for chunk in base_rows.chunks(SEARCH_PROBE_CHUNK_SIZE) {
        if is_cancelled(cancelled) {
            return Err(TableCacheError::Cancelled);
        }

        let chunk_row_ids: Vec<TableRowId> = chunk.iter().map(|(row_id, _)| *row_id).collect();
        let search_window = (has_all_columns_clause
            || (clauses.is_empty() && search_text_mode == SearchTextMode::Eager))
            .then(|| {
                model.materialize_window(&chunk_row_ids, &[], MaterializePurpose::SearchProbe)
            });
        let column_window = (!distinct_column_indices.is_empty()).then(|| {
            model.materialize_window(
                &chunk_row_ids,
                &distinct_column_indices,
                MaterializePurpose::Render,
            )
        });

        for &(row_id, base_index) in chunk {
            let mut row_search_text: Option<String> = None;
            let mut row_column_texts: HashMap<usize, String> = HashMap::new();

            let row_matches = clauses.iter().all(|clause| match clause.target {
                ClauseTarget::AllColumns => {
                    let row_text = row_search_text.get_or_insert_with(|| {
                        probe_row_search_text(model, row_id, search_window.as_ref())
                    });
                    clause.matcher.matches(row_text)
                }
                ClauseTarget::Column(column_index) => {
                    let column_text = row_column_texts.entry(column_index).or_insert_with(|| {
                        let cell = column_window
                            .as_ref()
                            .and_then(|window| window.cell(row_id, column_index))
                            .cloned()
                            .unwrap_or_else(|| model.cell(row_id, column_index));
                        table_cell_to_search_text(cell)
                    });
                    clause.matcher.matches(column_text)
                }
            });

            if row_matches {
                filtered_rows.push((row_id, base_index));
                if let Some(search_texts) = eager_search_texts.as_mut()
                    && (clauses.is_empty() || has_all_columns_clause)
                {
                    let text = row_search_text.unwrap_or_else(|| {
                        probe_row_search_text(model, row_id, search_window.as_ref())
                    });
                    search_texts.insert(row_id, text);
                }
            }
        }
    }

    let needs_post_filter_probe =
        search_text_mode == SearchTextMode::Eager && !clauses.is_empty() && !has_all_columns_clause;

    if needs_post_filter_probe {
        for chunk in filtered_rows.chunks(SEARCH_PROBE_CHUNK_SIZE) {
            if is_cancelled(cancelled) {
                return Err(TableCacheError::Cancelled);
            }
            let chunk_row_ids: Vec<TableRowId> = chunk.iter().map(|(row_id, _)| *row_id).collect();
            let search_window =
                model.materialize_window(&chunk_row_ids, &[], MaterializePurpose::SearchProbe);
            if let Some(search_texts) = eager_search_texts.as_mut() {
                for &(row_id, _) in chunk {
                    let text = probe_row_search_text(model, row_id, Some(&search_window));
                    search_texts.insert(row_id, text);
                }
            }
        }
    }

    Ok((filtered_rows, eager_search_texts, needs_post_filter_probe))
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
    build_table_cache_with_pinned_filters(model, display_filter, vec![], view_sort, cancelled)
}

/// Build a table cache by filtering and sorting the model rows, including pinned filters.
///
/// Effective filtering uses AND semantics across:
/// - all non-empty pinned filters
/// - the current display filter (if non-empty)
///
/// Duplicate filter clauses are removed while preserving first-seen order.
pub fn build_table_cache_with_pinned_filters(
    model: Arc<dyn TableModel>,
    display_filter: TableSearchSpec,
    pinned_filters: Vec<TableSearchSpec>,
    view_sort: Vec<TableSortSpec>,
    cancelled: Option<Arc<AtomicBool>>,
) -> Result<TableCache, TableCacheError> {
    let schema = model.schema();

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

    let effective_filter_specs = normalize_search_specs(
        pinned_filters
            .into_iter()
            .chain((!display_filter.text.is_empty()).then_some(display_filter))
            .collect(),
    );
    let compiled_clauses = compile_filter_clauses(&effective_filter_specs, model.as_ref())?;
    let (filtered_rows, search_text_map, _did_post_filter_probe) = collect_filtered_rows(
        model.as_ref(),
        &base_rows,
        &compiled_clauses,
        model.search_text_mode(),
        &cancelled,
    )?;

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
    let search_texts = search_text_map.map(|search_text_map| {
        row_ids
            .iter()
            .map(|row_id| {
                search_text_map
                    .get(row_id)
                    .cloned()
                    .unwrap_or_else(|| model.search_text(*row_id))
            })
            .collect()
    });
    let row_index = row_ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();
    Ok(TableCache {
        row_ids,
        row_index,
        search_texts,
    })
}
