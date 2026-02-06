pub mod cache;
pub mod model;
pub mod sources;
pub mod view;

pub use cache::{
    FILTER_DEBOUNCE_MS, FilterDraft, PendingScrollOp, TableCache, TableCacheEntry, TableCacheError,
    TableCacheKey, TableRuntimeState, TableScrollState, TypeSearchState, build_table_cache,
    format_rows_as_tsv, format_rows_as_tsv_with_header, fuzzy_match,
};
pub use model::{
    AnalysisKind, AnalysisParams, ColumnResizeResult, MIN_COLUMN_WIDTH, MaterializePurpose,
    MaterializedWindow, MultiSignalEntry, NavigationResult, ScrollTarget, SelectionUpdate,
    TableAction, TableCell, TableColumn, TableColumnConfig, TableColumnKey, TableModel,
    TableModelContext, TableModelKey, TableModelSpec, TableRowId, TableSchema, TableSearchMode,
    TableSearchSpec, TableSelection, TableSelectionMode, TableSortDirection, TableSortKey,
    TableSortSpec, TableTileId, TableTileState, TableViewConfig, find_type_search_match,
    format_selection_count, hidden_columns, navigate_down, navigate_end, navigate_extend_selection,
    navigate_home, navigate_page_down, navigate_page_up, navigate_up, resize_column,
    scroll_target_after_activation, scroll_target_after_filter, scroll_target_after_sort,
    selection_on_click_multi, selection_on_click_single, selection_on_ctrl_click,
    selection_on_shift_click, should_clear_selection_on_generation_change, sort_indicator,
    sort_spec_on_click, sort_spec_on_shift_click, toggle_column_visibility, visible_columns,
};
pub use view::draw_table_tile;

#[cfg(test)]
mod tests;
