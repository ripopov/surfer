use std::sync::Arc;

use ftr_parser::types::TransactionId;

use crate::{
    konata::{KonataModel, KonataModelSpec, KonataStage, StageFlags},
    table::{
        SearchTextMode, TableAction, TableCacheError, TableCell, TableColumn, TableColumnKey,
        TableModel, TableModelContext, TableRowId, TableSchema, TableSortKey,
    },
    transaction_container::TransactionRef,
};

use super::konata_instructions::resolve_konata_model;

/// Lazy event-table facade over the paged Konata projection. Unlike the raw
/// FTR event table, this never makes the generic transaction graph resident.
pub struct KonataEventTableModel {
    spec: KonataModelSpec,
    model: Arc<KonataModel>,
    parent_row: Option<usize>,
    row_count: usize,
}

impl KonataEventTableModel {
    pub fn new(
        spec: KonataModelSpec,
        parent_tx: Option<u64>,
        ctx: &TableModelContext<'_>,
    ) -> Result<Self, TableCacheError> {
        let model = resolve_konata_model(&spec, ctx)?;
        let parent_row = parent_tx
            .map(|transaction| {
                model.row_for_transaction(transaction).ok_or_else(|| {
                    TableCacheError::ModelNotFound {
                        description: format!(
                            "Pipeline parent transaction {transaction} is unavailable"
                        ),
                    }
                })
            })
            .transpose()?;
        let row_count = parent_row.map_or_else(
            || model.stage_count(),
            |row| model.stages_for_row_blocking(row).len(),
        );
        Ok(Self {
            spec,
            model,
            parent_row,
            row_count,
        })
    }

    fn stage_at(&self, index: usize) -> Option<(usize, KonataStage)> {
        self.parent_row.map_or_else(
            || self.model.stage_by_ordinal_blocking(index),
            |row| {
                self.model
                    .stages_for_row_blocking(row)
                    .get(index)
                    .cloned()
                    .map(|stage| (row, stage))
            },
        )
    }

    fn stage_for_id(&self, id: TableRowId) -> Option<(usize, KonataStage)> {
        let result = self.model.stage_for_event_blocking(id.0)?;
        self.parent_row
            .is_none_or(|parent| parent == result.0)
            .then_some(result)
    }

    fn parent_label(&self, row: usize) -> String {
        self.model.rows.label(row).map_or_else(
            || format!("tx#{}", self.model.rows.tx_id[row]),
            |label| self.model.string(label).to_string(),
        )
    }

    fn warnings(stage: &KonataStage) -> String {
        [
            (StageFlags::OUT_OF_RANGE, "outside parent"),
            (StageFlags::END_BEFORE_START, "end before start"),
            (StageFlags::UNNAMED, "unnamed"),
            (StageFlags::MULTIPLE_PARENTS, "multiple parents"),
        ]
        .into_iter()
        .filter_map(|(flag, label)| stage.flags.contains(flag).then_some(label))
        .collect::<Vec<_>>()
        .join(", ")
    }

    fn annotations(&self, stage: &KonataStage) -> String {
        self.model
            .annotations_for_stage_blocking(stage)
            .iter()
            .map(|annotation| self.model.annotation_text(annotation))
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn text(value: impl ToString) -> TableCell {
        TableCell::Text(value.to_string())
    }
}

impl TableModel for KonataEventTableModel {
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
                column("start", "Start", 90.0, true),
                column("end", "End", 90.0, true),
                column("duration", "Duration", 90.0, true),
                column("name", "Stage", 100.0, true),
                column("parent_id", "Parent ID", 90.0, true),
                column("parent_tx", "Parent tx", 100.0, false),
                column("parent", "Parent", 300.0, true),
                column("lane", "Lane", 90.0, true),
                column("warnings", "Warnings", 170.0, true),
                column("annotations", "Annotations", 320.0, false),
            ],
        }
    }

    fn row_count(&self) -> usize {
        self.row_count
    }

    fn row_id_at(&self, index: usize) -> Option<TableRowId> {
        self.stage_at(index)
            .map(|(_, stage)| TableRowId(stage.event_tx))
    }

    fn search_text_mode(&self) -> SearchTextMode {
        SearchTextMode::LazyProbe
    }

    fn cell(&self, id: TableRowId, col: usize) -> TableCell {
        let Some((row, stage)) = self.stage_for_id(id) else {
            return Self::text("");
        };
        match col {
            0 => Self::text(stage.start),
            1 => Self::text(stage.end),
            2 => Self::text(stage.end.saturating_sub(stage.start)),
            3 => Self::text(self.model.stage_name(&stage)),
            4 => Self::text(row),
            5 => Self::text(self.model.rows.tx_id[row]),
            6 => Self::text(self.parent_label(row)),
            7 => Self::text(self.model.lane_name(stage.lane)),
            8 => Self::text(Self::warnings(&stage)),
            9 => Self::text(self.annotations(&stage)),
            _ => Self::text(""),
        }
    }

    fn sort_key(&self, id: TableRowId, col: usize) -> TableSortKey {
        let Some((row, stage)) = self.stage_for_id(id) else {
            return TableSortKey::None;
        };
        match col {
            0 => TableSortKey::Numeric(stage.start as f64),
            1 => TableSortKey::Numeric(stage.end as f64),
            2 => TableSortKey::Numeric(stage.end.saturating_sub(stage.start) as f64),
            4 => TableSortKey::Numeric(row as f64),
            5 => TableSortKey::Numeric(self.model.rows.tx_id[row] as f64),
            _ => match self.cell(id, col) {
                TableCell::Text(text) => TableSortKey::Text(text),
                TableCell::RichText(text) => TableSortKey::Text(text.text().to_string()),
            },
        }
    }

    fn search_text(&self, id: TableRowId) -> String {
        let Some((row, stage)) = self.stage_for_id(id) else {
            return String::new();
        };
        format!(
            "{} {} ID {row} tx {} {} {} {}",
            stage.start,
            self.model.stage_name(&stage),
            self.model.rows.tx_id[row],
            self.parent_label(row),
            Self::warnings(&stage),
            self.annotations(&stage),
        )
    }

    fn on_activate(&self, id: TableRowId) -> TableAction {
        TableAction::FocusTransaction(
            self.spec.source,
            TransactionRef {
                id: TransactionId(id.0),
            },
        )
    }
}
