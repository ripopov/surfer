use super::super::model::{
    MaterializePurpose, TableCell, TableColumnConfig, TableColumnKey, TableModel, TableRowId,
    TableSchema, TableSelection, visible_columns,
};

/// Formats selected rows as tab-separated values for clipboard.
/// Only includes visible columns in their current display order.
/// Uses `materialize_window` for batch cell materialization.
/// Does not include header row.
pub fn format_rows_as_tsv(
    model: &dyn TableModel,
    selected_rows: &[TableRowId],
    visible_columns: &[TableColumnKey],
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
                TableCell::Text(s) => s,
                TableCell::RichText(rt) => rt.text().to_string(),
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
    model: &dyn TableModel,
    schema: &TableSchema,
    selected_rows: &[TableRowId],
    visible_columns: &[TableColumnKey],
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
                TableCell::Text(s) => s,
                TableCell::RichText(rt) => rt.text().to_string(),
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
    model: &dyn TableModel,
    schema: &TableSchema,
    row_order: &[TableRowId],
    selection: &TableSelection,
    columns_config: &[TableColumnConfig],
    include_header: bool,
) -> String {
    if selection.is_empty() {
        return String::new();
    }

    let export_columns: Vec<TableColumnKey> = if columns_config.is_empty() {
        schema
            .columns
            .iter()
            .map(|column| column.key.clone())
            .collect()
    } else {
        visible_columns(columns_config)
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
