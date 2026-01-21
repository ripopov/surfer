//! TransactionTrace table model for FTR transaction data.
//!
//! Displays transactions from a specific generator with columns for
//! Start, End, Duration, Type, and dynamic attribute columns.

use crate::table::{
    TableAction, TableCacheError, TableCell, TableColumn, TableColumnKey, TableModel,
    TableModelContext, TableRowId, TableSchema, TableSortKey,
};
use crate::time::TimeFormatter;
use crate::transaction_container::{TransactionRef, TransactionStreamRef};
use num::BigInt;
use num::BigUint;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

// Fixed column keys
const START_COLUMN_KEY: &str = "start";
const END_COLUMN_KEY: &str = "end";
const DURATION_COLUMN_KEY: &str = "duration";
const TYPE_COLUMN_KEY: &str = "type";

/// Maximum search text length per row to prevent memory bloat.
const MAX_SEARCH_TEXT_LEN: usize = 1024;

/// TransactionTrace table model for displaying FTR transactions from a single generator.
///
/// Shows transactions with fixed columns (Start, End, Duration, Type) followed by
/// dynamic attribute columns discovered from the transaction data.
pub struct TransactionTraceModel {
    generator: TransactionStreamRef,
    time_formatter: TimeFormatter,
    data: OnceLock<TransactionData>,
}

struct TransactionData {
    rows: Vec<TransactionRow>,
    index_by_id: HashMap<TableRowId, usize>,
    /// Attribute column names discovered from transactions, in discovery order.
    attribute_columns: Vec<String>,
}

struct TransactionRow {
    row_id: TableRowId,
    tx_ref: TransactionRef,
    start_time: BigUint,
    end_time: BigUint,
    duration: BigUint,
    tx_type: String,
    /// Attribute values in same order as TransactionData.attribute_columns.
    attribute_values: Vec<String>,
    // Pre-formatted strings for display and search
    start_time_text: String,
    end_time_text: String,
    duration_text: String,
    search_text: String,
}

impl TransactionTraceModel {
    /// Create a new TransactionTraceModel for a specific generator.
    ///
    /// # Arguments
    /// * `generator` - The generator to show transactions from (must have gen_id set)
    /// * `ctx` - Table model context with wave data and formatting settings
    ///
    /// # Errors
    /// Returns `TableCacheError::DataUnavailable` if transaction data is not loaded.
    /// Returns `TableCacheError::ModelNotFound` if the specified generator doesn't exist.
    pub fn new(
        generator: TransactionStreamRef,
        ctx: &TableModelContext<'_>,
    ) -> Result<Self, TableCacheError> {
        let waves = ctx.waves.ok_or(TableCacheError::DataUnavailable)?;
        let transactions = waves
            .inner
            .as_transactions()
            .ok_or(TableCacheError::DataUnavailable)?;

        // Validate generator exists
        let gen_id = generator
            .gen_id
            .ok_or_else(|| TableCacheError::ModelNotFound {
                description: format!("Generator reference missing gen_id: {}", generator.name),
            })?;

        if transactions.get_generator(gen_id).is_none() {
            return Err(TableCacheError::ModelNotFound {
                description: format!("Generator not found: {} (id: {})", generator.name, gen_id),
            });
        }

        // Create time formatter using transaction metadata
        let timescale = transactions.metadata().timescale;
        let time_formatter = TimeFormatter::new(&timescale, &ctx.wanted_timeunit, &ctx.time_format);

        Ok(Self {
            generator,
            time_formatter,
            data: OnceLock::new(),
        })
    }

    /// Get or build the transaction data lazily.
    fn data(&self) -> &TransactionData {
        self.data.get_or_init(|| {
            // This should not fail since we validated in new(), but we need wave data.
            // Since we don't store ctx, we need to work with minimal data.
            // The actual data building is deferred until we have access via create_model.
            TransactionData {
                rows: vec![],
                index_by_id: HashMap::new(),
                attribute_columns: vec![],
            }
        })
    }

    /// Build row data from transaction container.
    /// This is called externally after model creation with access to wave data.
    fn build_data(&self, ctx: &TableModelContext<'_>) -> TransactionData {
        let Some(waves) = ctx.waves else {
            return TransactionData {
                rows: vec![],
                index_by_id: HashMap::new(),
                attribute_columns: vec![],
            };
        };

        let Some(transactions) = waves.inner.as_transactions() else {
            return TransactionData {
                rows: vec![],
                index_by_id: HashMap::new(),
                attribute_columns: vec![],
            };
        };

        let Some(gen_id) = self.generator.gen_id else {
            return TransactionData {
                rows: vec![],
                index_by_id: HashMap::new(),
                attribute_columns: vec![],
            };
        };

        let Some(generator) = transactions.get_generator(gen_id) else {
            return TransactionData {
                rows: vec![],
                index_by_id: HashMap::new(),
                attribute_columns: vec![],
            };
        };

        let mut rows = Vec::new();
        let mut attribute_names_set = HashSet::new();
        let mut attribute_names_order = Vec::new();

        // First pass: discover attribute columns
        for tx in &generator.transactions {
            for attr in tx.attributes.iter() {
                if !attribute_names_set.contains(&attr.name) {
                    attribute_names_set.insert(attr.name.clone());
                    attribute_names_order.push(attr.name.clone());
                }
            }
        }

        // Second pass: build rows
        for tx in &generator.transactions {
            let tx_id = tx.get_tx_id();
            let start_time = tx.get_start_time();
            let end_time = tx.get_end_time();

            // Compute duration
            let duration = if end_time >= start_time {
                &end_time - &start_time
            } else {
                BigUint::from(0u32)
            };

            // Row ID is just the transaction ID (unique within generator)
            let row_id = TableRowId(tx_id as u64);

            // Format times
            let start_time_text = self
                .time_formatter
                .format(&BigInt::from(start_time.clone()));
            let end_time_text = self.time_formatter.format(&BigInt::from(end_time.clone()));
            let duration_text = self.time_formatter.format(&BigInt::from(duration.clone()));

            // Get transaction type - format as "tx#ID"
            let tx_type = format!("tx#{tx_id}");

            // Build attribute values in order
            let mut attribute_values = vec![String::new(); attribute_names_order.len()];
            for attr in tx.attributes.iter() {
                if let Some(idx) = attribute_names_order.iter().position(|n| n == &attr.name) {
                    attribute_values[idx] = attr.value();
                }
            }

            // Build search text (capped at MAX_SEARCH_TEXT_LEN)
            let mut search_text = format!(
                "{} {} {} {}",
                start_time_text, end_time_text, duration_text, tx_type
            );
            for val in &attribute_values {
                if !val.is_empty() {
                    search_text.push(' ');
                    search_text.push_str(val);
                }
            }
            if search_text.len() > MAX_SEARCH_TEXT_LEN {
                search_text.truncate(MAX_SEARCH_TEXT_LEN);
            }

            rows.push(TransactionRow {
                row_id,
                tx_ref: TransactionRef { id: tx_id },
                start_time,
                end_time,
                duration,
                tx_type,
                attribute_values,
                start_time_text,
                end_time_text,
                duration_text,
                search_text,
            });
        }

        // Sort rows by start time (base order for row_id_at)
        rows.sort_by(|a, b| a.start_time.cmp(&b.start_time));

        // Build index
        let index_by_id: HashMap<TableRowId, usize> = rows
            .iter()
            .enumerate()
            .map(|(i, r)| (r.row_id, i))
            .collect();

        TransactionData {
            rows,
            index_by_id,
            attribute_columns: attribute_names_order,
        }
    }

    fn row_by_id(&self, row: TableRowId) -> Option<&TransactionRow> {
        let data = self.data();
        data.index_by_id
            .get(&row)
            .and_then(|idx| data.rows.get(*idx))
    }
}

impl TableModel for TransactionTraceModel {
    fn schema(&self) -> TableSchema {
        let data = self.data();

        // Fixed columns in display order (no Generator column since table is per-generator)
        let mut columns = vec![
            TableColumn {
                key: TableColumnKey::Str(START_COLUMN_KEY.to_string()),
                label: "Start".to_string(),
                default_width: Some(100.0),
                default_visible: true,
                default_resizable: true,
            },
            TableColumn {
                key: TableColumnKey::Str(END_COLUMN_KEY.to_string()),
                label: "End".to_string(),
                default_width: Some(100.0),
                default_visible: true,
                default_resizable: true,
            },
            TableColumn {
                key: TableColumnKey::Str(DURATION_COLUMN_KEY.to_string()),
                label: "Duration".to_string(),
                default_width: Some(100.0),
                default_visible: true,
                default_resizable: true,
            },
            TableColumn {
                key: TableColumnKey::Str(TYPE_COLUMN_KEY.to_string()),
                label: "Type".to_string(),
                default_width: Some(100.0),
                default_visible: true,
                default_resizable: true,
            },
        ];

        // Add dynamic attribute columns
        for attr_name in &data.attribute_columns {
            columns.push(TableColumn {
                key: TableColumnKey::Str(format!("attr_{attr_name}")),
                label: attr_name.clone(),
                default_width: Some(100.0),
                default_visible: true,
                default_resizable: true,
            });
        }

        TableSchema { columns }
    }

    fn row_count(&self) -> usize {
        self.data().rows.len()
    }

    fn row_id_at(&self, index: usize) -> Option<TableRowId> {
        self.data().rows.get(index).map(|row| row.row_id)
    }

    fn cell(&self, row: TableRowId, col: usize) -> TableCell {
        let Some(row) = self.row_by_id(row) else {
            return TableCell::Text(String::new());
        };

        match col {
            0 => TableCell::Text(row.start_time_text.clone()),
            1 => TableCell::Text(row.end_time_text.clone()),
            2 => TableCell::Text(row.duration_text.clone()),
            3 => TableCell::Text(row.tx_type.clone()),
            _ => {
                // Attribute column
                let attr_idx = col - 4;
                TableCell::Text(
                    row.attribute_values
                        .get(attr_idx)
                        .cloned()
                        .unwrap_or_default(),
                )
            }
        }
    }

    fn sort_key(&self, row: TableRowId, col: usize) -> TableSortKey {
        let Some(row) = self.row_by_id(row) else {
            return TableSortKey::None;
        };

        match col {
            0 => {
                // Start time - numeric
                row.start_time
                    .to_string()
                    .parse::<f64>()
                    .map(TableSortKey::Numeric)
                    .unwrap_or(TableSortKey::Text(row.start_time_text.clone()))
            }
            1 => {
                // End time - numeric
                row.end_time
                    .to_string()
                    .parse::<f64>()
                    .map(TableSortKey::Numeric)
                    .unwrap_or(TableSortKey::Text(row.end_time_text.clone()))
            }
            2 => {
                // Duration - numeric
                row.duration
                    .to_string()
                    .parse::<f64>()
                    .map(TableSortKey::Numeric)
                    .unwrap_or(TableSortKey::Text(row.duration_text.clone()))
            }
            3 => TableSortKey::Text(row.tx_type.clone()),
            _ => {
                // Attribute column - text
                let attr_idx = col - 4;
                TableSortKey::Text(
                    row.attribute_values
                        .get(attr_idx)
                        .cloned()
                        .unwrap_or_default(),
                )
            }
        }
    }

    fn search_text(&self, row: TableRowId) -> String {
        self.row_by_id(row)
            .map(|row| row.search_text.clone())
            .unwrap_or_default()
    }

    fn on_activate(&self, row: TableRowId) -> TableAction {
        self.row_by_id(row)
            .map(|row| TableAction::FocusTransaction(row.tx_ref.clone()))
            .unwrap_or(TableAction::None)
    }
}

/// A variant of TransactionTraceModel that has pre-built data.
/// This is needed because the TableModel trait doesn't allow passing context
/// to schema() and other methods.
pub struct TransactionTraceModelWithData {
    inner: TransactionTraceModel,
}

impl TransactionTraceModelWithData {
    /// Create a new TransactionTraceModel and immediately build its data.
    pub fn new(
        generator: TransactionStreamRef,
        ctx: &TableModelContext<'_>,
    ) -> Result<Self, TableCacheError> {
        let inner = TransactionTraceModel::new(generator, ctx)?;
        let data = inner.build_data(ctx);
        let _ = inner.data.set(data);
        Ok(Self { inner })
    }
}

impl TableModel for TransactionTraceModelWithData {
    fn schema(&self) -> TableSchema {
        self.inner.schema()
    }

    fn row_count(&self) -> usize {
        self.inner.row_count()
    }

    fn row_id_at(&self, index: usize) -> Option<TableRowId> {
        self.inner.row_id_at(index)
    }

    fn cell(&self, row: TableRowId, col: usize) -> TableCell {
        self.inner.cell(row, col)
    }

    fn sort_key(&self, row: TableRowId, col: usize) -> TableSortKey {
        self.inner.sort_key(row, col)
    }

    fn search_text(&self, row: TableRowId) -> String {
        self.inner.search_text(row)
    }

    fn on_activate(&self, row: TableRowId) -> TableAction {
        self.inner.on_activate(row)
    }
}
