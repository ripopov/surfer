use crate::displayed_item::{DisplayedItem, FieldFormat};
use crate::table::{
    TableAction, TableCacheError, TableCell, TableColumn, TableColumnKey, TableModel,
    TableModelContext, TableRowId, TableSchema, TableSortKey,
};
use crate::time::TimeFormatter;
use crate::translation::{AnyTranslator, TranslationResultExt, TranslatorList};
use crate::wave_container::{SignalAccessor, VariableMeta, VariableRef, VariableRefExt};
use num::BigInt;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;
use surfer_translation_types::{Translator, VariableValue};

const TIME_COLUMN_KEY: &str = "time";
const VALUE_COLUMN_KEY: &str = "value";

pub struct SignalChangeListModel {
    field: Vec<String>,
    meta: VariableMeta,
    accessor: SignalAccessor,
    translator: AnyTranslator,
    translators: TranslatorList,
    root_format: Option<String>,
    field_formats: Vec<FieldFormat>,
    time_formatter: TimeFormatter,
    rows: OnceLock<TransitionRows>,
}

struct TransitionRows {
    rows: Vec<TransitionRow>,
    index_by_id: HashMap<TableRowId, usize>,
}

struct TransitionRow {
    row_id: TableRowId,
    time_u64: u64,
    time: BigInt,
    time_text: String,
    value_text: String,
    value_numeric: Option<f64>,
    search_text: String,
}

impl SignalChangeListModel {
    pub fn new(
        variable: VariableRef,
        field: Vec<String>,
        ctx: &TableModelContext<'_>,
    ) -> Result<Self, TableCacheError> {
        let waves = ctx.waves.ok_or(TableCacheError::DataUnavailable)?;
        let wave_container = waves
            .inner
            .as_waves()
            .ok_or(TableCacheError::DataUnavailable)?;

        let updated_variable = wave_container
            .update_variable_ref(&variable)
            .ok_or_else(|| TableCacheError::ModelNotFound {
                description: format!("Signal not found: {}", variable.full_path_string()),
            })?;

        let signal_id = wave_container.signal_id(&updated_variable).map_err(|err| {
            TableCacheError::ModelNotFound {
                description: err.to_string(),
            }
        })?;

        if !wave_container.is_signal_loaded(&signal_id) {
            return Err(TableCacheError::DataUnavailable);
        }

        let accessor = wave_container
            .signal_accessor(signal_id)
            .map_err(|_| TableCacheError::DataUnavailable)?;

        let meta = wave_container
            .variable_meta(&updated_variable)
            .map_err(|err| TableCacheError::ModelNotFound {
                description: err.to_string(),
            })?;

        let displayed_variable = waves.displayed_items.values().find_map(|item| match item {
            DisplayedItem::Variable(var) if var.variable_ref == updated_variable => Some(var),
            _ => None,
        });

        let root_format = displayed_variable.and_then(|var| var.format.clone());
        let field_formats = displayed_variable
            .map(|var| var.field_formats.clone())
            .unwrap_or_default();

        let format_override = displayed_variable
            .and_then(|var| var.get_format(&field))
            .cloned();

        let translator_name = format_override.unwrap_or_else(|| {
            if field.is_empty() {
                waves.select_preferred_translator(&meta, ctx.translators)
            } else {
                ctx.translators.default.clone()
            }
        });

        let translator = ctx.translators.clone_translator(&translator_name);

        let time_formatter = TimeFormatter::new(
            &wave_container.metadata().timescale,
            &ctx.wanted_timeunit,
            &ctx.time_format,
        );

        Ok(Self {
            field,
            meta,
            accessor,
            translator,
            translators: ctx.translators.clone(),
            root_format,
            field_formats,
            time_formatter,
            rows: OnceLock::new(),
        })
    }

    fn rows(&self) -> &TransitionRows {
        self.rows.get_or_init(|| self.build_rows())
    }

    fn build_rows(&self) -> TransitionRows {
        let mut rows = Vec::new();
        let mut index_by_id = HashMap::new();
        let mut duplicates: HashMap<u64, u64> = HashMap::new();

        for (time_u64, value) in self.accessor.iter_changes() {
            let seq = duplicates.entry(time_u64).or_insert(0);
            let row_id = if *seq == 0 {
                TableRowId(time_u64)
            } else {
                TableRowId(hash_row_id(time_u64, *seq))
            };
            *seq += 1;

            let time = BigInt::from(time_u64);
            let time_text = self.time_formatter.format(&time);
            let (value_text, value_numeric) = self.format_value(&value);
            let search_text = format!("{time_text} {value_text}");

            let row = TransitionRow {
                row_id,
                time_u64,
                time,
                time_text,
                value_text,
                value_numeric,
                search_text,
            };
            index_by_id.insert(row_id, rows.len());
            rows.push(row);
        }

        TransitionRows { rows, index_by_id }
    }

    fn format_value(&self, value: &VariableValue) -> (String, Option<f64>) {
        let numeric = if self.field.is_empty() {
            self.translator.translate_numeric(&self.meta, value)
        } else {
            None
        };

        match self.translator.translate(&self.meta, value) {
            Ok(translated) => {
                let fields = translated.format_flat(
                    &self.root_format,
                    &self.field_formats,
                    &self.translators,
                );
                let match_field = fields.iter().find(|res| res.names == self.field);
                match match_field.and_then(|res| res.value.as_ref()) {
                    Some(value) => (value.value.clone(), numeric),
                    None => ("-".to_string(), numeric),
                }
            }
            Err(_) => ("-".to_string(), numeric),
        }
    }

    fn row_by_id(&self, row: TableRowId) -> Option<&TransitionRow> {
        let rows = self.rows();
        rows.index_by_id
            .get(&row)
            .and_then(|idx| rows.rows.get(*idx))
    }
}

impl TableModel for SignalChangeListModel {
    fn schema(&self) -> TableSchema {
        TableSchema {
            columns: vec![
                TableColumn {
                    key: TableColumnKey::Str(TIME_COLUMN_KEY.to_string()),
                    label: "Time".to_string(),
                    default_width: Some(120.0),
                    default_visible: true,
                    default_resizable: true,
                },
                TableColumn {
                    key: TableColumnKey::Str(VALUE_COLUMN_KEY.to_string()),
                    label: "Value".to_string(),
                    default_width: Some(200.0),
                    default_visible: true,
                    default_resizable: true,
                },
            ],
        }
    }

    fn row_count(&self) -> usize {
        self.rows().rows.len()
    }

    fn row_id_at(&self, index: usize) -> Option<TableRowId> {
        self.rows().rows.get(index).map(|row| row.row_id)
    }

    fn cell(&self, row: TableRowId, col: usize) -> TableCell {
        let Some(row) = self.row_by_id(row) else {
            return TableCell::Text(String::new());
        };

        match col {
            0 => TableCell::Text(row.time_text.clone()),
            1 => TableCell::Text(row.value_text.clone()),
            _ => TableCell::Text(String::new()),
        }
    }

    fn sort_key(&self, row: TableRowId, col: usize) -> TableSortKey {
        let Some(row) = self.row_by_id(row) else {
            return TableSortKey::None;
        };

        match col {
            0 => TableSortKey::Numeric(row.time_u64 as f64),
            1 => match row.value_numeric {
                Some(value) => TableSortKey::Numeric(value),
                None => TableSortKey::Text(row.value_text.clone()),
            },
            _ => TableSortKey::None,
        }
    }

    fn search_text(&self, row: TableRowId) -> String {
        self.row_by_id(row)
            .map(|row| row.search_text.clone())
            .unwrap_or_default()
    }

    fn on_activate(&self, row: TableRowId) -> TableAction {
        self.row_by_id(row)
            .map(|row| TableAction::CursorSet(row.time.clone()))
            .unwrap_or(TableAction::None)
    }
}

fn hash_row_id(time: u64, seq: u64) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    time.hash(&mut hasher);
    seq.hash(&mut hasher);
    hasher.finish()
}
