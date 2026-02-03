use crate::table::{
    TableAction, TableCell, TableColumn, TableColumnKey, TableModel, TableRowId, TableSchema,
    TableSortKey,
};

/// Virtual table model for testing and demonstration.
///
/// Generates deterministic synthetic data based on seed, row count, and column count.
pub struct VirtualTableModel {
    rows: usize,
    columns: usize,
    seed: u64,
}

impl VirtualTableModel {
    pub fn new(rows: usize, columns: usize, seed: u64) -> Self {
        Self {
            rows,
            columns,
            seed,
        }
    }

    /// Generate a deterministic cell value based on seed, row, and column.
    fn generate_cell_value(&self, row: u64, col: usize) -> String {
        // Simple deterministic hash combining seed, row, and column
        let hash = self
            .seed
            .wrapping_mul(31)
            .wrapping_add(row)
            .wrapping_mul(17)
            .wrapping_add(col as u64);
        format!("R{}C{}_{:04x}", row, col, hash & 0xFFFF)
    }

    /// Generate a numeric sort key from the cell value.
    fn generate_sort_key_value(&self, row: u64, col: usize) -> f64 {
        // Use a deterministic numeric value based on seed, row, and column
        let hash = self
            .seed
            .wrapping_mul(31)
            .wrapping_add(row)
            .wrapping_mul(17)
            .wrapping_add(col as u64);
        (hash % 10000) as f64
    }
}

impl TableModel for VirtualTableModel {
    fn schema(&self) -> TableSchema {
        let columns = (0..self.columns)
            .map(|i| TableColumn {
                key: TableColumnKey::Str(format!("col_{i}")),
                label: format!("Col {i}"),
                default_width: Some(100.0),
                default_visible: true,
                default_resizable: true,
            })
            .collect();
        TableSchema { columns }
    }

    fn row_count(&self) -> usize {
        self.rows
    }

    fn row_id_at(&self, index: usize) -> Option<TableRowId> {
        if index < self.rows {
            Some(TableRowId(index as u64))
        } else {
            None
        }
    }

    fn cell(&self, row: TableRowId, col: usize) -> TableCell {
        if col < self.columns {
            TableCell::Text(self.generate_cell_value(row.0, col))
        } else {
            TableCell::Text(String::new())
        }
    }

    fn sort_key(&self, row: TableRowId, col: usize) -> TableSortKey {
        if col < self.columns {
            TableSortKey::Numeric(self.generate_sort_key_value(row.0, col))
        } else {
            TableSortKey::None
        }
    }

    fn search_text(&self, row: TableRowId) -> String {
        (0..self.columns)
            .map(|col| self.generate_cell_value(row.0, col))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn on_activate(&self, _row: TableRowId) -> TableAction {
        TableAction::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_model_row_count() {
        let model = VirtualTableModel::new(100, 5, 42);
        assert_eq!(model.row_count(), 100);

        let model_empty = VirtualTableModel::new(0, 3, 0);
        assert_eq!(model_empty.row_count(), 0);
    }

    #[test]
    fn virtual_model_row_id_at_sequential() {
        let model = VirtualTableModel::new(10, 3, 42);

        for i in 0..10 {
            let row_id = model.row_id_at(i);
            assert_eq!(row_id, Some(TableRowId(i as u64)));
        }

        // Out of bounds
        assert_eq!(model.row_id_at(10), None);
        assert_eq!(model.row_id_at(100), None);
    }

    #[test]
    fn virtual_model_cell_deterministic() {
        let model = VirtualTableModel::new(10, 5, 42);
        let row = TableRowId(3);

        // Same model, same row/col should produce same value
        let cell1 = model.cell(row, 2);
        let cell2 = model.cell(row, 2);

        let text1 = match cell1 {
            TableCell::Text(s) => s,
            _ => panic!("Expected Text cell"),
        };
        let text2 = match cell2 {
            TableCell::Text(s) => s,
            _ => panic!("Expected Text cell"),
        };

        assert_eq!(text1, text2);
        assert!(text1.contains("R3C2")); // Should include row and column info
    }

    #[test]
    fn virtual_model_same_params_identical_output() {
        let model1 = VirtualTableModel::new(10, 5, 42);
        let model2 = VirtualTableModel::new(10, 5, 42);

        for row_idx in 0..10 {
            let row = TableRowId(row_idx as u64);
            for col in 0..5 {
                let cell1 = match model1.cell(row, col) {
                    TableCell::Text(s) => s,
                    _ => panic!("Expected Text cell"),
                };
                let cell2 = match model2.cell(row, col) {
                    TableCell::Text(s) => s,
                    _ => panic!("Expected Text cell"),
                };
                assert_eq!(cell1, cell2, "Mismatch at row {row_idx}, col {col}");
            }
        }
    }

    #[test]
    fn virtual_model_different_seed_different_output() {
        let model1 = VirtualTableModel::new(10, 5, 42);
        let model2 = VirtualTableModel::new(10, 5, 99);

        let row = TableRowId(3);
        let cell1 = match model1.cell(row, 2) {
            TableCell::Text(s) => s,
            _ => panic!("Expected Text cell"),
        };
        let cell2 = match model2.cell(row, 2) {
            TableCell::Text(s) => s,
            _ => panic!("Expected Text cell"),
        };

        // Different seeds should produce different values (at least the hash portion)
        assert_ne!(cell1, cell2);
    }

    #[test]
    fn virtual_model_schema_columns() {
        let model = VirtualTableModel::new(10, 4, 42);
        let schema = model.schema();

        assert_eq!(schema.columns.len(), 4);

        for (i, col) in schema.columns.iter().enumerate() {
            assert_eq!(col.key, TableColumnKey::Str(format!("col_{i}")));
            assert_eq!(col.label, format!("Col {i}"));
            assert!(col.default_visible);
            assert!(col.default_resizable);
        }
    }

    #[test]
    fn virtual_model_search_text_includes_all_columns() {
        let model = VirtualTableModel::new(10, 3, 42);
        let row = TableRowId(5);
        let search_text = model.search_text(row);

        // search_text should include content from all columns
        for col in 0..3 {
            let cell_text = match model.cell(row, col) {
                TableCell::Text(s) => s,
                _ => panic!("Expected Text cell"),
            };
            assert!(
                search_text.contains(&cell_text),
                "search_text should contain col {col} value: {cell_text}"
            );
        }
    }

    #[test]
    fn virtual_model_sort_key_numeric() {
        let model = VirtualTableModel::new(10, 3, 42);
        let row = TableRowId(5);

        let sort_key = model.sort_key(row, 1);
        match sort_key {
            TableSortKey::Numeric(v) => {
                assert!(v >= 0.0);
                assert!(v < 10000.0);
            }
            _ => panic!("Expected Numeric sort key"),
        }

        // Out of bounds column
        let sort_key_oob = model.sort_key(row, 100);
        assert_eq!(sort_key_oob, TableSortKey::None);
    }

    #[test]
    fn virtual_model_on_activate_returns_none() {
        let model = VirtualTableModel::new(10, 3, 42);
        let action = model.on_activate(TableRowId(5));
        match action {
            TableAction::None => {}
            _ => panic!("Expected TableAction::None"),
        }
    }
}
