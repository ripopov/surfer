pub mod cache;
pub mod model;
pub mod sources;
pub mod view;

pub use cache::{TableCache, TableCacheEntry, TableCacheError, TableCacheKey};
pub use model::{
    AnalysisKind, AnalysisParams, TableAction, TableCell, TableColumn, TableColumnConfig,
    TableColumnKey, TableModel, TableModelKey, TableModelSpec, TableRowId, TableSchema,
    TableSearchMode, TableSearchSpec, TableSelection, TableSelectionMode, TableSortDirection,
    TableSortKey, TableSortSpec, TableTileId, TableViewConfig,
};
pub use view::draw_table_tile;

#[cfg(test)]
mod tests;
