pub mod cache;
pub mod model;
pub mod sources;
pub mod view;

pub use cache::{
    TableCache, TableCacheEntry, TableCacheError, TableCacheKey, TableRuntimeState,
    build_table_cache, fuzzy_match,
};
pub use model::{
    AnalysisKind, AnalysisParams, SelectionUpdate, TableAction, TableCell, TableColumn,
    TableColumnConfig, TableColumnKey, TableModel, TableModelKey, TableModelSpec, TableRowId,
    TableSchema, TableSearchMode, TableSearchSpec, TableSelection, TableSelectionMode,
    TableSortDirection, TableSortKey, TableSortSpec, TableTileId, TableTileState, TableViewConfig,
    format_selection_count, selection_on_click_multi, selection_on_click_single,
    selection_on_ctrl_click, selection_on_shift_click, sort_indicator, sort_spec_on_click,
    sort_spec_on_shift_click,
};
pub use view::draw_table_tile;

#[cfg(test)]
mod tests;
