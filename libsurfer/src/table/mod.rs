pub mod cache;
pub mod model;
pub mod sources;
pub mod view;

pub use cache::{
    TableCache, TableCacheEntry, TableCacheError, TableCacheKey, TableRuntimeState,
    TypeSearchState, build_table_cache, format_rows_as_tsv, format_rows_as_tsv_with_header,
    fuzzy_match,
};
pub use model::{
    AnalysisKind, AnalysisParams, NavigationResult, SelectionUpdate, TableAction, TableCell,
    TableColumn, TableColumnConfig, TableColumnKey, TableModel, TableModelKey, TableModelSpec,
    TableRowId, TableSchema, TableSearchMode, TableSearchSpec, TableSelection, TableSelectionMode,
    TableSortDirection, TableSortKey, TableSortSpec, TableTileId, TableTileState, TableViewConfig,
    find_type_search_match, format_selection_count, navigate_down, navigate_end,
    navigate_extend_selection, navigate_home, navigate_page_down, navigate_page_up, navigate_up,
    selection_on_click_multi, selection_on_click_single, selection_on_ctrl_click,
    selection_on_shift_click, sort_indicator, sort_spec_on_click, sort_spec_on_shift_click,
};
pub use view::draw_table_tile;

#[cfg(test)]
mod tests;
