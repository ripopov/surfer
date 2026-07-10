use std::sync::Arc;

use crate::{
    konata::{KonataModel, KonataModelSpec},
    table::{
        SearchTextMode, TableAction, TableCell, TableColumn, TableColumnKey, TableModel,
        TableModelContext, TableRowId, TableSchema, TableSortKey,
    },
    transaction_container::TransactionRef,
};
use ftr_parser::types::TransactionId;

use super::super::TableCacheError;

pub struct KonataInstructionTableModel {
    spec: KonataModelSpec,
    model: Arc<KonataModel>,
}

impl KonataInstructionTableModel {
    pub fn new(
        spec: KonataModelSpec,
        ctx: &TableModelContext<'_>,
    ) -> Result<Self, TableCacheError> {
        let model = resolve_konata_model(&spec, ctx)?;
        Ok(Self { spec, model })
    }

    fn physical_row(&self, row: TableRowId) -> Option<usize> {
        self.model.row_for_transaction(row.0)
    }

    fn text_cell(value: impl ToString) -> TableCell {
        TableCell::Text(value.to_string())
    }
}

pub(crate) fn resolve_konata_model(
    spec: &KonataModelSpec,
    ctx: &TableModelContext<'_>,
) -> Result<Arc<KonataModel>, TableCacheError> {
    let parent_generator = spec
        .generator
        .gen_id
        .ok_or_else(|| TableCacheError::ModelNotFound {
            description: "Konata instruction table requires a generator".to_string(),
        })?;
    let generation = ctx.source_generation(spec.source);
    ctx.konata_models
        .iter()
        .find(|(key, _)| {
            key.source == spec.source
                && key.stream == spec.generator.stream_id
                && key.parent_generator == parent_generator
                && key.generation == generation
        })
        .and_then(|(_, entry)| entry.model())
        .ok_or_else(|| TableCacheError::ModelNotFound {
            description: "Konata projection is not ready".to_string(),
        })
}

impl TableModel for KonataInstructionTableModel {
    fn schema(&self) -> TableSchema {
        let column = |key: &str, label: &str, width, visible| TableColumn {
            key: TableColumnKey::Str(key.to_string()),
            label: label.to_string(),
            default_width: Some(width),
            default_visible: visible,
            default_resizable: true,
        };
        TableSchema {
            columns: vec![
                column("id", "ID", 70.0, true),
                column("sid", "SID", 80.0, true),
                column("tid", "TID", 70.0, true),
                column("rid", "RID", 80.0, true),
                column("fetch", "Fetch", 90.0, true),
                column("retire", "Retire", 90.0, true),
                column("duration", "Duration", 90.0, true),
                column("flushed", "Flushed", 70.0, true),
                column("label", "Label", 340.0, true),
                column("detail", "Detail", 280.0, false),
                column("stages", "Stages", 260.0, false),
            ],
        }
    }

    fn row_count(&self) -> usize {
        self.model.row_count()
    }

    fn row_id_at(&self, index: usize) -> Option<TableRowId> {
        self.model.rows.tx_id.get(index).copied().map(TableRowId)
    }

    fn search_text_mode(&self) -> SearchTextMode {
        SearchTextMode::LazyProbe
    }

    fn cell(&self, row: TableRowId, col: usize) -> TableCell {
        let Some(index) = self.physical_row(row) else {
            return Self::text_cell("");
        };
        match col {
            0 => Self::text_cell(index),
            1 => Self::text_cell(
                self.model
                    .rows
                    .sid(index)
                    .map_or_else(String::new, |v| v.to_string()),
            ),
            2 => Self::text_cell(
                self.model
                    .rows
                    .tid(index)
                    .map_or("", |tid| self.model.thread_name(tid)),
            ),
            3 => Self::text_cell(
                self.model
                    .rows
                    .rid(index)
                    .map_or_else(String::new, |v| v.to_string()),
            ),
            4 => Self::text_cell(self.model.rows.begin[index]),
            5 => Self::text_cell(self.model.rows.end[index]),
            6 => Self::text_cell(
                self.model.rows.end[index].saturating_sub(self.model.rows.begin[index]),
            ),
            7 => Self::text_cell(match self.model.rows.flushed[index] {
                crate::konata::FlushState::True => "true",
                crate::konata::FlushState::False => "false",
                crate::konata::FlushState::Unknown => "unknown",
            }),
            8 => Self::text_cell(
                self.model
                    .rows
                    .label(index)
                    .map_or("", |label| self.model.string(label)),
            ),
            9 => Self::text_cell(
                self.model
                    .rows
                    .detail(index)
                    .map_or("", |detail| self.model.string(detail)),
            ),
            10 => Self::text_cell(
                self.model
                    .stages_for_row_blocking(index)
                    .iter()
                    .map(|stage| self.model.stage_name(stage))
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            _ => Self::text_cell(""),
        }
    }

    fn sort_key(&self, row: TableRowId, col: usize) -> TableSortKey {
        let Some(index) = self.physical_row(row) else {
            return TableSortKey::None;
        };
        match col {
            0 => TableSortKey::Numeric(index as f64),
            1 => self
                .model
                .rows
                .sid(index)
                .map_or(TableSortKey::None, |v| TableSortKey::Numeric(v as f64)),
            2 => self
                .model
                .rows
                .tid(index)
                .map_or(TableSortKey::None, |tid| {
                    TableSortKey::Text(self.model.thread_name(tid).to_string())
                }),
            3 => self
                .model
                .rows
                .rid(index)
                .map_or(TableSortKey::None, |v| TableSortKey::Numeric(v as f64)),
            4 => TableSortKey::Numeric(self.model.rows.begin[index] as f64),
            5 => TableSortKey::Numeric(self.model.rows.end[index] as f64),
            6 => TableSortKey::Numeric(
                self.model.rows.end[index].saturating_sub(self.model.rows.begin[index]) as f64,
            ),
            _ => match self.cell(row, col) {
                TableCell::Text(text) => TableSortKey::Text(text),
                TableCell::RichText(text) => TableSortKey::Text(text.text().to_string()),
            },
        }
    }

    fn search_text(&self, row: TableRowId) -> String {
        self.physical_row(row)
            .map_or_else(String::new, |row| self.model.search_text(row))
    }

    fn on_activate(&self, row: TableRowId) -> TableAction {
        TableAction::FocusTransaction(
            self.spec.source,
            TransactionRef {
                id: TransactionId(row.0),
            },
        )
    }
}
