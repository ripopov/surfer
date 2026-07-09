//! Event table model for the FTR transaction event convention.
//!
//! Displays the events of a parent generator (the transactions of its
//! matching `.events` generator) with the promoted `name` attribute and the
//! resolved parent transaction as first-class columns.

use crate::source::SourceId;
use crate::table::{
    TableAction, TableCacheError, TableCell, TableColumn, TableColumnKey, TableModel,
    TableModelContext, TableRowId, TableSchema, TableSortKey,
};
use crate::time::TimeFormatter;
use crate::transaction_container::{TransactionRef, TransactionStreamRef};
use crate::transaction_events::{EVENT_NAME_ATTRIBUTE, event_name};
use ftr_parser::types::AttributeType;
use num::BigInt;
use std::collections::HashMap;

// Fixed column keys
const TIME_COLUMN_KEY: &str = "time";
const DURATION_COLUMN_KEY: &str = "duration";
const NAME_COLUMN_KEY: &str = "name";
const PARENT_COLUMN_KEY: &str = "parent";
/// Number of fixed columns before the dynamic attribute columns.
const FIXED_COLUMNS: usize = 4;

/// Maximum search text length per row to prevent memory bloat.
const MAX_SEARCH_TEXT_LEN: usize = 1024;

/// Shown in the parent column for orphan events.
const ORPHAN_PARENT_LABEL: &str = "—";

/// Table model over the events of one parent generator.
pub struct EventTableModel {
    source: SourceId,
    rows: Vec<EventRow>,
    index_by_id: HashMap<TableRowId, usize>,
    /// Attribute column names discovered from the events, in discovery
    /// order. The promoted `name` BEGIN attribute is not repeated here.
    attribute_columns: Vec<String>,
}

struct EventRow {
    row_id: TableRowId,
    tx_ref: TransactionRef,
    time: u64,
    duration: u64,
    name: String,
    parent: String,
    /// Values in the same order as `attribute_columns`.
    attribute_values: Vec<String>,
    time_text: String,
    duration_text: String,
    search_text: String,
}

impl EventTableModel {
    /// Creates the model for the events of `generator` (a parent generator
    /// with a matching `.events` generator).
    ///
    /// # Errors
    /// Returns `TableCacheError::DataUnavailable` when no transaction data is
    /// loaded and `TableCacheError::ModelNotFound` when the generator or its
    /// events generator cannot be resolved.
    pub fn new(
        source: SourceId,
        generator: TransactionStreamRef,
        ctx: &TableModelContext<'_>,
    ) -> Result<Self, TableCacheError> {
        if !ctx.ftr_events_enabled {
            return Err(TableCacheError::ModelNotFound {
                description: "FTR event support is disabled".to_string(),
            });
        }
        let waves = ctx.waves.ok_or(TableCacheError::DataUnavailable)?;
        let transactions = waves
            .transactions_for_source(source)
            .ok_or(TableCacheError::DataUnavailable)?;

        let parent_gen_id = generator
            .gen_id
            .ok_or_else(|| TableCacheError::ModelNotFound {
                description: format!("Generator reference missing gen_id: {}", generator.name),
            })?;

        let events_gen_id = transactions
            .event_index()
            .conforming_events_generator_of(parent_gen_id)
            .ok_or_else(|| TableCacheError::ModelNotFound {
                description: format!("Generator has no events generator: {}", generator.name),
            })?;

        let events_generator = transactions.get_generator(events_gen_id).ok_or_else(|| {
            TableCacheError::ModelNotFound {
                description: format!("Events generator not found for: {}", generator.name),
            }
        })?;

        let timescale = transactions.metadata().timescale;
        let time_formatter = TimeFormatter::new(&timescale, &ctx.wanted_timeunit, &ctx.time_format);

        // First pass: discover attribute columns (the promoted name BEGIN
        // attribute stays out of the dynamic columns)
        let mut attribute_columns: Vec<String> = vec![];
        for tx in &events_generator.transactions {
            for attr in &tx.attributes {
                let is_promoted_name = matches!(attr.kind, AttributeType::BEGIN)
                    && attr.name.as_ref() == EVENT_NAME_ATTRIBUTE;
                if !is_promoted_name && !attribute_columns.iter().any(|c| c == attr.name.as_ref()) {
                    attribute_columns.push(attr.name.to_string());
                }
            }
        }

        let index = transactions.event_index();
        let mut rows: Vec<EventRow> = events_generator
            .transactions
            .iter()
            .map(|tx| {
                let tx_id = tx.get_tx_id();
                let time = tx.get_start_time();
                let end_time = tx.get_end_time();
                let duration = end_time.saturating_sub(time);

                let name = event_name(tx).unwrap_or_default();
                let parent = index
                    .event_info(tx_id)
                    .and_then(|info| {
                        let parent_gen = transactions.get_generator(info.parent_gen)?;
                        Some(format!("{} #{}", parent_gen.name, info.parent_tx))
                    })
                    .unwrap_or_else(|| ORPHAN_PARENT_LABEL.to_string());

                let attribute_values = attribute_columns
                    .iter()
                    .map(|column| {
                        tx.attributes
                            .iter()
                            .find(|attr| attr.name.as_ref() == column)
                            .map(ftr_parser::types::Attribute::value)
                            .unwrap_or_default()
                    })
                    .collect::<Vec<_>>();

                let time_text = time_formatter.format(&BigInt::from(time));
                let duration_text = time_formatter.format(&BigInt::from(duration));

                let mut search_text = format!("{time_text} {duration_text} {name} {parent}");
                for value in attribute_values.iter().filter(|value| !value.is_empty()) {
                    search_text.push(' ');
                    search_text.push_str(value);
                }
                search_text.truncate(MAX_SEARCH_TEXT_LEN);

                EventRow {
                    row_id: TableRowId(tx_id.0 as u64),
                    tx_ref: TransactionRef { id: tx_id },
                    time,
                    duration,
                    name,
                    parent,
                    attribute_values,
                    time_text,
                    duration_text,
                    search_text,
                }
            })
            .collect();

        rows.sort_by_key(|row| row.time);

        let index_by_id = rows
            .iter()
            .enumerate()
            .map(|(i, row)| (row.row_id, i))
            .collect();

        Ok(Self {
            source,
            rows,
            index_by_id,
            attribute_columns,
        })
    }

    fn row_by_id(&self, row: TableRowId) -> Option<&EventRow> {
        self.index_by_id
            .get(&row)
            .and_then(|idx| self.rows.get(*idx))
    }
}

impl TableModel for EventTableModel {
    fn schema(&self) -> TableSchema {
        let mut columns = vec![
            TableColumn {
                key: TableColumnKey::Str(TIME_COLUMN_KEY.to_string()),
                label: "Time".to_string(),
                default_width: Some(100.0),
                default_visible: true,
                default_resizable: true,
            },
            TableColumn {
                key: TableColumnKey::Str(DURATION_COLUMN_KEY.to_string()),
                label: "Duration".to_string(),
                default_width: Some(80.0),
                default_visible: true,
                default_resizable: true,
            },
            TableColumn {
                key: TableColumnKey::Str(NAME_COLUMN_KEY.to_string()),
                label: "Name".to_string(),
                default_width: Some(120.0),
                default_visible: true,
                default_resizable: true,
            },
            TableColumn {
                key: TableColumnKey::Str(PARENT_COLUMN_KEY.to_string()),
                label: "Parent".to_string(),
                default_width: Some(140.0),
                default_visible: true,
                default_resizable: true,
            },
        ];

        for attr_name in &self.attribute_columns {
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
        self.rows.len()
    }

    fn row_id_at(&self, index: usize) -> Option<TableRowId> {
        self.rows.get(index).map(|row| row.row_id)
    }

    fn cell(&self, row: TableRowId, col: usize) -> TableCell {
        let Some(row) = self.row_by_id(row) else {
            return TableCell::Text(String::new());
        };

        match col {
            0 => TableCell::Text(row.time_text.clone()),
            1 => TableCell::Text(row.duration_text.clone()),
            2 => TableCell::Text(row.name.clone()),
            3 => TableCell::Text(row.parent.clone()),
            _ => TableCell::Text(
                row.attribute_values
                    .get(col - FIXED_COLUMNS)
                    .cloned()
                    .unwrap_or_default(),
            ),
        }
    }

    fn sort_key(&self, row: TableRowId, col: usize) -> TableSortKey {
        let Some(row) = self.row_by_id(row) else {
            return TableSortKey::None;
        };

        match col {
            0 => row
                .time
                .to_string()
                .parse::<f64>()
                .map(TableSortKey::Numeric)
                .unwrap_or_else(|_| TableSortKey::Text(row.time_text.clone())),
            1 => row
                .duration
                .to_string()
                .parse::<f64>()
                .map(TableSortKey::Numeric)
                .unwrap_or_else(|_| TableSortKey::Text(row.duration_text.clone())),
            2 => TableSortKey::Text(row.name.clone()),
            3 => TableSortKey::Text(row.parent.clone()),
            _ => TableSortKey::Text(
                row.attribute_values
                    .get(col - FIXED_COLUMNS)
                    .cloned()
                    .unwrap_or_default(),
            ),
        }
    }

    fn search_text(&self, row: TableRowId) -> String {
        self.row_by_id(row)
            .map(|row| row.search_text.clone())
            .unwrap_or_default()
    }

    fn on_activate(&self, row: TableRowId) -> TableAction {
        self.row_by_id(row)
            .map(|row| TableAction::FocusTransaction(self.source, row.tx_ref.clone()))
            .unwrap_or(TableAction::None)
    }
}
