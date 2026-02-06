use crate::displayed_item::{DisplayedItem, FieldFormat};
use crate::table::sources::multi_signal_index::MergedIndex;
use crate::table::{
    MultiSignalEntry, SearchTextMode, TableAction, TableCacheError, TableCell, TableColumn,
    TableColumnKey, TableModel, TableModelContext, TableRowId, TableSchema, TableSortKey,
};
use crate::time::TimeFormatter;
use crate::translation::{AnyTranslator, TranslationResultExt, TranslatorList};
use crate::wave_container::{SignalAccessor, VariableMeta, VariableRefExt};
use egui::RichText;
use num::BigInt;
use std::sync::OnceLock;
use surfer_translation_types::Translator;
use tracing::warn;

const TIME_COLUMN_KEY: &str = "time";
const SIGNAL_COLUMN_KEY_PREFIX: &str = "sig:v1:";

/// Resolved per-signal entry metadata for the multi-signal model.
struct ResolvedSignalEntry {
    meta: VariableMeta,
    field: Vec<String>,
    accessor: SignalAccessor,
    translator: AnyTranslator,
    translators: TranslatorList,
    root_format: Option<String>,
    field_formats: Vec<FieldFormat>,
    column_key: String,
    display_label: String,
}

pub struct MultiSignalChangeListModel {
    entries: Vec<ResolvedSignalEntry>,
    time_formatter: TimeFormatter,
    index: OnceLock<MergedIndex>,
}

impl MultiSignalChangeListModel {
    pub fn new(
        variables: Vec<MultiSignalEntry>,
        ctx: &TableModelContext<'_>,
    ) -> Result<Self, TableCacheError> {
        let waves = ctx.waves.ok_or(TableCacheError::DataUnavailable)?;
        let wave_container = waves
            .inner
            .as_waves()
            .ok_or(TableCacheError::DataUnavailable)?;

        let deduped =
            crate::table::sources::multi_signal_index::dedup_multi_signal_entries(variables);

        let mut entries = Vec::new();
        for entry in &deduped {
            let Some(updated_variable) = wave_container.update_variable_ref(&entry.variable) else {
                warn!(
                    "Multi-signal change list: signal not found, skipping: {}",
                    entry.variable.full_path_string()
                );
                continue;
            };

            let signal_id = match wave_container.signal_id(&updated_variable) {
                Ok(id) => id,
                Err(err) => {
                    warn!(
                        "Multi-signal change list: signal id lookup failed, skipping {}: {err}",
                        updated_variable.full_path_string()
                    );
                    continue;
                }
            };

            if !wave_container.is_signal_loaded(&signal_id) {
                warn!(
                    "Multi-signal change list: signal not loaded, skipping: {}",
                    updated_variable.full_path_string()
                );
                continue;
            }

            let accessor = match wave_container.signal_accessor(signal_id) {
                Ok(acc) => acc,
                Err(_) => {
                    warn!(
                        "Multi-signal change list: accessor unavailable, skipping: {}",
                        updated_variable.full_path_string()
                    );
                    continue;
                }
            };

            let meta = match wave_container.variable_meta(&updated_variable) {
                Ok(m) => m,
                Err(err) => {
                    warn!(
                        "Multi-signal change list: variable meta failed, skipping {}: {err}",
                        updated_variable.full_path_string()
                    );
                    continue;
                }
            };

            let displayed_variable = waves.displayed_items.values().find_map(|item| match item {
                DisplayedItem::Variable(var) if var.variable_ref == updated_variable => Some(var),
                _ => None,
            });

            let root_format = displayed_variable.and_then(|var| var.format.clone());
            let field_formats = displayed_variable
                .map(|var| var.field_formats.clone())
                .unwrap_or_default();

            let format_override = displayed_variable
                .and_then(|var| var.get_format(&entry.field))
                .cloned();

            let translator_name = format_override.unwrap_or_else(|| {
                if entry.field.is_empty() {
                    waves.select_preferred_translator(&meta, ctx.translators)
                } else {
                    ctx.translators.default.clone()
                }
            });

            let translator = ctx.translators.clone_translator(&translator_name);

            let column_key =
                encode_signal_column_key(&updated_variable.full_path_string(), &entry.field);

            let display_label =
                build_display_label(&updated_variable.full_path_string(), &entry.field);

            entries.push(ResolvedSignalEntry {
                meta,
                field: entry.field.clone(),
                accessor,
                translator,
                translators: ctx.translators.clone(),
                root_format,
                field_formats,
                column_key,
                display_label,
            });
        }

        if entries.is_empty() {
            return Err(TableCacheError::ModelNotFound {
                description: "No valid signals found for multi-signal change list".to_string(),
            });
        }

        let time_formatter = TimeFormatter::new(
            &wave_container.metadata().timescale,
            &ctx.wanted_timeunit,
            &ctx.time_format,
        );

        Ok(Self {
            entries,
            time_formatter,
            index: OnceLock::new(),
        })
    }

    fn index(&self) -> &MergedIndex {
        self.index.get_or_init(|| self.build_index())
    }

    fn build_index(&self) -> MergedIndex {
        let transition_iters = self
            .entries
            .iter()
            .map(|entry| entry.accessor.iter_changes().map(|(time, _)| time));
        MergedIndex::from_transition_time_iters(transition_iters)
    }

    /// Classify a signal cell at a given row timestamp.
    ///
    /// Returns `(CellState, run_len)` where `run_len > 1` indicates collapsed same-time runs.
    fn classify_cell(&self, signal_idx: usize, time_u64: u64) -> (CellState, u16) {
        let index = self.index();
        if let Some(run) = index.exact_run(signal_idx, time_u64) {
            (CellState::Transition, run.run_len)
        } else if index.previous_run(signal_idx, time_u64).is_some() {
            (CellState::Held, 1)
        } else {
            (CellState::NoData, 1)
        }
    }

    /// Format a signal value using its translator, mirroring `SignalChangeListModel::format_value`.
    fn format_signal_value(
        entry: &ResolvedSignalEntry,
        value: &surfer_translation_types::VariableValue,
    ) -> (String, Option<f64>) {
        let numeric = if entry.field.is_empty() {
            entry.translator.translate_numeric(&entry.meta, value)
        } else {
            None
        };

        match entry.translator.translate(&entry.meta, value) {
            Ok(translated) => {
                let fields = translated.format_flat(
                    &entry.root_format,
                    &entry.field_formats,
                    &entry.translators,
                );
                let match_field = fields.iter().find(|res| res.names == entry.field);
                match match_field.and_then(|res| res.value.as_ref()) {
                    Some(val) => (val.value.clone(), numeric),
                    None => ("\u{2014}".to_string(), numeric),
                }
            }
            Err(_) => ("\u{2014}".to_string(), numeric),
        }
    }

    /// Materialize a signal cell's display text and numeric value.
    fn materialize_signal_cell(
        &self,
        signal_idx: usize,
        time_u64: u64,
    ) -> (CellState, String, Option<f64>, u16) {
        let (state, run_len) = self.classify_cell(signal_idx, time_u64);
        let entry = &self.entries[signal_idx];

        match state {
            CellState::Transition | CellState::Held => {
                match entry.accessor.query_at_time(time_u64) {
                    Some(value) => {
                        let (text, numeric) = Self::format_signal_value(entry, &value);
                        if state == CellState::Transition && run_len > 1 {
                            let collapsed = format!("{text} (+{})", run_len - 1);
                            (state, collapsed, numeric, run_len)
                        } else {
                            (state, text, numeric, run_len)
                        }
                    }
                    None => (CellState::NoData, EM_DASH.to_string(), None, run_len),
                }
            }
            CellState::NoData => (CellState::NoData, EM_DASH.to_string(), None, run_len),
        }
    }
}

/// Em dash used for no-data cells.
const EM_DASH: &str = "\u{2014}";

/// Cell state classification for multi-signal table cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CellState {
    /// Signal has a transition at this exact time.
    Transition,
    /// Signal holds its value from a previous transition.
    Held,
    /// No data exists for this signal at or before this time.
    NoData,
}

impl TableModel for MultiSignalChangeListModel {
    fn schema(&self) -> TableSchema {
        let mut columns = Vec::with_capacity(1 + self.entries.len());

        columns.push(TableColumn {
            key: TableColumnKey::Str(TIME_COLUMN_KEY.to_string()),
            label: "Time".to_string(),
            default_width: Some(120.0),
            default_visible: true,
            default_resizable: true,
        });

        for entry in &self.entries {
            columns.push(TableColumn {
                key: TableColumnKey::Str(entry.column_key.clone()),
                label: entry.display_label.clone(),
                default_width: Some(150.0),
                default_visible: true,
                default_resizable: true,
            });
        }

        TableSchema { columns }
    }

    fn row_count(&self) -> usize {
        self.index().row_ids.len()
    }

    fn row_id_at(&self, index: usize) -> Option<TableRowId> {
        self.index().row_ids.get(index).copied()
    }

    fn search_text_mode(&self) -> SearchTextMode {
        SearchTextMode::LazyProbe
    }

    fn cell(&self, row: TableRowId, col: usize) -> TableCell {
        if col == 0 {
            let time = BigInt::from(row.0);
            return TableCell::Text(self.time_formatter.format(&time));
        }

        let signal_idx = col - 1;
        if signal_idx >= self.entries.len() {
            return TableCell::Text(String::new());
        }

        let (state, text, _, _) = self.materialize_signal_cell(signal_idx, row.0);
        match state {
            CellState::Transition => TableCell::Text(text),
            CellState::Held | CellState::NoData => TableCell::RichText(RichText::new(text).weak()),
        }
    }

    fn sort_key(&self, row: TableRowId, col: usize) -> TableSortKey {
        if col == 0 {
            return TableSortKey::Numeric(row.0 as f64);
        }

        let signal_idx = col - 1;
        if signal_idx >= self.entries.len() {
            return TableSortKey::None;
        }

        let (state, text, numeric, _) = self.materialize_signal_cell(signal_idx, row.0);
        match state {
            CellState::NoData => TableSortKey::None,
            CellState::Transition | CellState::Held => match numeric {
                Some(n) => TableSortKey::Numeric(n),
                None => TableSortKey::Text(text),
            },
        }
    }

    fn search_text(&self, row: TableRowId) -> String {
        let time = BigInt::from(row.0);
        let mut parts = vec![self.time_formatter.format(&time)];

        for signal_idx in 0..self.entries.len() {
            let (_, text, _, _) = self.materialize_signal_cell(signal_idx, row.0);
            parts.push(text);
        }

        parts.join(" ")
    }

    fn on_activate(&self, row: TableRowId) -> TableAction {
        let index = self.index();
        if index.row_index.contains_key(&row) {
            TableAction::CursorSet(BigInt::from(row.0))
        } else {
            TableAction::None
        }
    }
}

/// Encode a signal column key using percent-encoding for path separators.
///
/// Format: `sig:v1:<escaped-path>#<escaped-field>`
///
/// The encoding uses percent-encoding for `.`, `#`, `%`, and `/` characters
/// to ensure the key is reversible and unambiguous.
pub fn encode_signal_column_key(full_path: &str, field: &[String]) -> String {
    let escaped_path = percent_encode_component(full_path);
    let escaped_field = field
        .iter()
        .map(|f| percent_encode_component(f))
        .collect::<Vec<_>>()
        .join(".");
    format!("{SIGNAL_COLUMN_KEY_PREFIX}{escaped_path}#{escaped_field}")
}

/// Decode a signal column key back into `(full_path, field)`.
///
/// Returns `None` if the key does not match the expected format.
pub fn decode_signal_column_key(key: &str) -> Option<(String, Vec<String>)> {
    let rest = key.strip_prefix(SIGNAL_COLUMN_KEY_PREFIX)?;
    let (escaped_path, escaped_field) = rest.split_once('#')?;
    let full_path = percent_decode_component(escaped_path);
    let field = if escaped_field.is_empty() {
        vec![]
    } else {
        escaped_field
            .split('.')
            .map(percent_decode_component)
            .collect()
    };
    Some((full_path, field))
}

/// Percent-encode reserved characters: `%`, `.`, `#`, `/`.
fn percent_encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '%' => out.push_str("%25"),
            '.' => out.push_str("%2E"),
            '#' => out.push_str("%23"),
            '/' => out.push_str("%2F"),
            _ => out.push(ch),
        }
    }
    out
}

/// Decode percent-encoded component.
fn percent_decode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            match hex.as_str() {
                "25" => out.push('%'),
                "2E" | "2e" => out.push('.'),
                "23" => out.push('#'),
                "2F" | "2f" => out.push('/'),
                _ => {
                    out.push('%');
                    out.push_str(&hex);
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Build a human-readable display label from variable path and field.
fn build_display_label(full_path: &str, field: &[String]) -> String {
    if field.is_empty() {
        full_path.to_string()
    } else {
        format!("{}.{}", full_path, field.join("."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_key_encode_decode_round_trip_simple() {
        let path = "tb.dut.counter";
        let field: Vec<String> = vec![];
        let key = encode_signal_column_key(path, &field);
        assert_eq!(key, "sig:v1:tb%2Edut%2Ecounter#");

        let (decoded_path, decoded_field) = decode_signal_column_key(&key).unwrap();
        assert_eq!(decoded_path, path);
        assert_eq!(decoded_field, field);
    }

    #[test]
    fn column_key_encode_decode_round_trip_with_field() {
        let path = "tb.dut.counter";
        let field = vec!["value".to_string(), "lsb".to_string()];
        let key = encode_signal_column_key(path, &field);
        assert_eq!(key, "sig:v1:tb%2Edut%2Ecounter#value.lsb");

        let (decoded_path, decoded_field) = decode_signal_column_key(&key).unwrap();
        assert_eq!(decoded_path, path);
        assert_eq!(decoded_field, field);
    }

    #[test]
    fn column_key_encode_special_chars() {
        let path = "a%b#c/d.e";
        let field = vec!["f.g".to_string()];
        let key = encode_signal_column_key(path, &field);

        let (decoded_path, decoded_field) = decode_signal_column_key(&key).unwrap();
        assert_eq!(decoded_path, path);
        assert_eq!(decoded_field, field);
    }

    #[test]
    fn column_key_decode_invalid_prefix_returns_none() {
        assert!(decode_signal_column_key("invalid:key").is_none());
        assert!(decode_signal_column_key("sig:v2:path#field").is_none());
    }

    #[test]
    fn column_key_decode_missing_hash_returns_none() {
        assert!(decode_signal_column_key("sig:v1:nohash").is_none());
    }

    #[test]
    fn display_label_no_field() {
        assert_eq!(build_display_label("tb.clk", &[]), "tb.clk");
    }

    #[test]
    fn display_label_with_field() {
        assert_eq!(
            build_display_label("tb.counter", &["value".to_string()]),
            "tb.counter.value"
        );
    }
}
