use crate::table::{
    SignalAnalysisConfig, SignalAnalysisSamplingMode, TableAction, TableCacheError, TableCell,
    TableColumn, TableColumnKey, TableModel, TableModelContext, TableRowId, TableSchema,
    TableSortKey,
};
use crate::time::TimeFormatter;
use crate::translation::AnyTranslator;
use crate::wave_container::{
    SignalAccessor, VariableMeta, VariableRef, VariableRefExt, WaveContainer,
};
use num::{BigInt, One, ToPrimitive, Zero};
use std::collections::HashMap;
use surfer_translation_types::{Translator, VariableValue};

const GLOBAL_LABEL: &str = "GLOBAL";
const EM_DASH: &str = "\u{2014}";
const INTERVAL_END_COLUMN_KEY: &str = "interval_end";
const INFO_COLUMN_KEY: &str = "info";
const ANALYSIS_COLUMN_KEY_PREFIX: &str = "signal_analysis:v1:";
const METRIC_SUFFIXES: [&str; 4] = ["avg", "min", "max", "sum"];

/// Inclusive time range captured for a single analysis run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignalAnalysisTimeRange {
    pub start: u64,
    pub end: u64,
}

impl SignalAnalysisTimeRange {
    /// Create a valid inclusive time range (`start <= end`).
    #[must_use]
    pub fn new(start: u64, end: u64) -> Option<Self> {
        (start <= end).then_some(Self { start, end })
    }

    #[must_use]
    pub fn contains(&self, time: u64) -> bool {
        (self.start..=self.end).contains(&time)
    }
}

/// Marker normalized to u64 timestamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignalAnalysisMarker {
    pub id: u8,
    pub time: u64,
}

/// User-facing interval definition used by the analysis result model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalAnalysisInterval {
    pub start: u64,
    pub end: u64,
    pub label: String,
}

/// Numeric metric values computed for one `(interval, signal)` pair.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SignalAnalysisMetrics {
    pub count: u64,
    pub average: f64,
    pub min: f64,
    pub max: f64,
    pub sum: f64,
}

/// Aggregated metric output for all intervals and the global row.
#[derive(Debug, Clone, PartialEq)]
pub struct SignalAnalysisAccumulation {
    pub per_interval: Vec<Vec<Option<SignalAnalysisMetrics>>>,
    pub global: Vec<Option<SignalAnalysisMetrics>>,
}

struct ResolvedSignalAnalysisSignal {
    display_label: String,
    field: Vec<String>,
    meta: VariableMeta,
    accessor: SignalAccessor,
    translator: AnyTranslator,
}

struct SignalAnalysisResultRow {
    row_id: TableRowId,
    interval_end_u64: u64,
    interval_end: BigInt,
    interval_end_text: String,
    info: String,
    metric_values: Vec<Option<f64>>,
    metric_texts: Vec<String>,
    search_text: String,
}

/// Analysis-result table model backed by stage-2 pure computation helpers.
pub struct SignalAnalysisResultsModel {
    schema: TableSchema,
    rows: Vec<SignalAnalysisResultRow>,
    row_index: HashMap<TableRowId, usize>,
}

impl SignalAnalysisResultsModel {
    pub fn new(
        config: SignalAnalysisConfig,
        ctx: &TableModelContext<'_>,
    ) -> Result<Self, TableCacheError> {
        let waves = ctx.waves.ok_or(TableCacheError::DataUnavailable)?;
        let wave_container = waves
            .inner
            .as_waves()
            .ok_or(TableCacheError::DataUnavailable)?;

        if config.signals.is_empty() {
            return Err(TableCacheError::ModelNotFound {
                description: "Signal analysis requires at least one analyzed signal".to_string(),
            });
        }

        let (_sampling_variable, sampling_meta, sampling_accessor) =
            resolve_loaded_signal(wave_container, &config.sampling.signal)?;

        let sampling_mode = infer_sampling_mode(&sampling_meta);
        let trigger_times = collect_trigger_times(sampling_mode, sampling_accessor.iter_changes());

        let analyzed_signals = config
            .signals
            .iter()
            .map(|signal| {
                let (variable, meta, accessor) =
                    resolve_loaded_signal(wave_container, &signal.variable)?;
                Ok(ResolvedSignalAnalysisSignal {
                    display_label: signal_display_label(&variable, &signal.field),
                    field: signal.field.clone(),
                    meta,
                    accessor,
                    translator: resolve_translator(ctx, &signal.translator),
                })
            })
            .collect::<Result<Vec<_>, TableCacheError>>()?;

        let end_u64 = waves
            .num_timestamps()
            .and_then(|num| num.to_u64())
            .ok_or(TableCacheError::DataUnavailable)?;
        let range =
            SignalAnalysisTimeRange::new(0, end_u64).ok_or(TableCacheError::DataUnavailable)?;

        let markers = normalize_markers(
            waves.markers.iter().map(|(id, time)| (*id, time.clone())),
            range,
        );
        let intervals = build_intervals(range, &markers);

        let accumulation = accumulate_signal_metrics(
            &trigger_times,
            range,
            &markers,
            analyzed_signals.len(),
            |signal_idx, time| {
                let signal = &analyzed_signals[signal_idx];
                if !signal.field.is_empty() {
                    return None;
                }
                let value = signal.accessor.query_at_time(time)?;
                signal.translator.translate_numeric(&signal.meta, &value)
            },
        );

        let time_formatter = TimeFormatter::new(
            &wave_container.metadata().timescale,
            &ctx.wanted_timeunit,
            &ctx.time_format,
        );

        let mut rows = Vec::new();
        if markers.is_empty() {
            rows.push(build_result_row(
                TableRowId(0),
                range.end,
                GLOBAL_LABEL.to_string(),
                &accumulation.global,
                &time_formatter,
            ));
        } else {
            for (idx, interval) in intervals.iter().enumerate() {
                let metrics = accumulation
                    .per_interval
                    .get(idx)
                    .cloned()
                    .unwrap_or_else(|| vec![None; analyzed_signals.len()]);
                rows.push(build_result_row(
                    TableRowId(idx as u64),
                    interval.end,
                    interval.label.clone(),
                    &metrics,
                    &time_formatter,
                ));
            }
            rows.push(build_result_row(
                TableRowId(rows.len() as u64),
                range.end,
                GLOBAL_LABEL.to_string(),
                &accumulation.global,
                &time_formatter,
            ));
        }

        let row_index = rows
            .iter()
            .enumerate()
            .map(|(idx, row)| (row.row_id, idx))
            .collect();

        let signal_labels = analyzed_signals
            .iter()
            .map(|signal| signal.display_label.clone())
            .collect::<Vec<_>>();

        Ok(Self {
            schema: build_schema(&signal_labels),
            rows,
            row_index,
        })
    }

    fn row_by_id(&self, row: TableRowId) -> Option<&SignalAnalysisResultRow> {
        self.row_index.get(&row).and_then(|idx| self.rows.get(*idx))
    }
}

impl TableModel for SignalAnalysisResultsModel {
    fn schema(&self) -> TableSchema {
        self.schema.clone()
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
            0 => TableCell::Text(row.interval_end_text.clone()),
            1 => TableCell::Text(row.info.clone()),
            col => {
                let metric_idx = col.saturating_sub(2);
                TableCell::Text(
                    row.metric_texts
                        .get(metric_idx)
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
            0 => TableSortKey::Numeric(row.interval_end_u64 as f64),
            1 => TableSortKey::Text(row.info.clone()),
            col => {
                let metric_idx = col.saturating_sub(2);
                row.metric_values
                    .get(metric_idx)
                    .and_then(|value| *value)
                    .map_or(TableSortKey::None, TableSortKey::Numeric)
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
            .map(|row| TableAction::CursorSet(row.interval_end.clone()))
            .unwrap_or(TableAction::None)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct MetricsAccumulator {
    count: u64,
    sum: f64,
    min: Option<f64>,
    max: Option<f64>,
}

impl MetricsAccumulator {
    fn update(&mut self, value: f64) {
        self.count = self.count.saturating_add(1);
        self.sum += value;
        self.min = Some(self.min.map_or(value, |current| current.min(value)));
        self.max = Some(self.max.map_or(value, |current| current.max(value)));
    }

    #[must_use]
    fn into_metrics(self) -> Option<SignalAnalysisMetrics> {
        let min = self.min?;
        let max = self.max?;

        Some(SignalAnalysisMetrics {
            count: self.count,
            average: self.sum / self.count as f64,
            min,
            max,
            sum: self.sum,
        })
    }
}

fn resolve_loaded_signal(
    wave_container: &WaveContainer,
    variable: &VariableRef,
) -> Result<(VariableRef, VariableMeta, SignalAccessor), TableCacheError> {
    let Some(updated_variable) = wave_container.update_variable_ref(variable) else {
        return Err(TableCacheError::ModelNotFound {
            description: format!("Signal not found: {}", variable.full_path_string()),
        });
    };

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

    Ok((updated_variable, meta, accessor))
}

fn resolve_translator(ctx: &TableModelContext<'_>, requested: &str) -> AnyTranslator {
    let has_requested = ctx
        .translators
        .all_translators()
        .iter()
        .any(|translator| translator.name() == requested);

    let translator_name = if has_requested {
        requested
    } else {
        &ctx.translators.default
    };

    ctx.translators.clone_translator(translator_name)
}

fn signal_display_label(variable: &VariableRef, field: &[String]) -> String {
    let full_path = variable.full_path_string();
    if field.is_empty() {
        full_path
    } else {
        format!("{}.{}", full_path, field.join("."))
    }
}

fn build_schema(signal_labels: &[String]) -> TableSchema {
    let mut columns = vec![
        TableColumn {
            key: TableColumnKey::Str(INTERVAL_END_COLUMN_KEY.to_string()),
            label: "Interval End".to_string(),
            default_width: Some(140.0),
            default_visible: true,
            default_resizable: true,
        },
        TableColumn {
            key: TableColumnKey::Str(INFO_COLUMN_KEY.to_string()),
            label: "Info".to_string(),
            default_width: Some(240.0),
            default_visible: true,
            default_resizable: true,
        },
    ];

    for (signal_idx, signal_label) in signal_labels.iter().enumerate() {
        for suffix in METRIC_SUFFIXES {
            columns.push(TableColumn {
                key: TableColumnKey::Str(metric_column_key(signal_idx, suffix)),
                label: format!("{signal_label}.{suffix}"),
                default_width: Some(120.0),
                default_visible: true,
                default_resizable: true,
            });
        }
    }

    TableSchema { columns }
}

fn metric_column_key(signal_idx: usize, suffix: &str) -> String {
    format!("{ANALYSIS_COLUMN_KEY_PREFIX}{signal_idx}:{suffix}")
}

fn build_result_row(
    row_id: TableRowId,
    interval_end_u64: u64,
    info: String,
    metrics: &[Option<SignalAnalysisMetrics>],
    time_formatter: &TimeFormatter,
) -> SignalAnalysisResultRow {
    let interval_end = BigInt::from(interval_end_u64);
    let interval_end_text = time_formatter.format(&interval_end);

    let mut metric_values = Vec::with_capacity(metrics.len() * METRIC_SUFFIXES.len());
    for metric in metrics {
        match metric {
            Some(metric) => {
                metric_values.push(Some(metric.average));
                metric_values.push(Some(metric.min));
                metric_values.push(Some(metric.max));
                metric_values.push(Some(metric.sum));
            }
            None => {
                metric_values.extend([None; METRIC_SUFFIXES.len()]);
            }
        }
    }

    let metric_texts = metric_values
        .iter()
        .map(|value| {
            value
                .map(format_metric)
                .unwrap_or_else(|| EM_DASH.to_string())
        })
        .collect::<Vec<_>>();

    let mut search_parts = Vec::with_capacity(2 + metric_texts.len());
    search_parts.push(interval_end_text.clone());
    search_parts.push(info.clone());
    search_parts.extend(metric_texts.iter().cloned());

    SignalAnalysisResultRow {
        row_id,
        interval_end_u64,
        interval_end,
        interval_end_text,
        info,
        metric_values,
        metric_texts,
        search_text: search_parts.join(" "),
    }
}

fn format_metric(value: f64) -> String {
    let mut text = format!("{value:.6}");

    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" { "0".to_string() } else { text }
}

/// Infer sampling mode from signal metadata using required precedence:
/// `Event` first, then `PosEdge` for 1-bit, otherwise `AnyChange`.
#[must_use]
pub fn infer_sampling_mode(meta: &VariableMeta) -> SignalAnalysisSamplingMode {
    if meta.is_event() {
        SignalAnalysisSamplingMode::Event
    } else if meta.num_bits == Some(1) {
        SignalAnalysisSamplingMode::PosEdge
    } else {
        SignalAnalysisSamplingMode::AnyChange
    }
}

/// Collect trigger timestamps from sampling-signal changes.
#[must_use]
pub fn collect_trigger_times<I>(mode: SignalAnalysisSamplingMode, changes: I) -> Vec<u64>
where
    I: IntoIterator<Item = (u64, VariableValue)>,
{
    match mode {
        SignalAnalysisSamplingMode::Event | SignalAnalysisSamplingMode::AnyChange => {
            changes.into_iter().map(|(time, _)| time).collect()
        }
        SignalAnalysisSamplingMode::PosEdge => collect_posedge_trigger_times(changes),
    }
}

/// Convert BigInt time range into u64 range for table-analysis kernels.
#[must_use]
pub fn normalize_time_range(start: &BigInt, end: &BigInt) -> Option<SignalAnalysisTimeRange> {
    let start = start.to_u64()?;
    let end = end.to_u64()?;
    SignalAnalysisTimeRange::new(start, end)
}

/// Sort, deduplicate, and clip markers to the captured run range.
///
/// Duplicate timestamps are collapsed by keeping the smallest marker id at that time.
#[must_use]
pub fn normalize_markers<I>(markers: I, range: SignalAnalysisTimeRange) -> Vec<SignalAnalysisMarker>
where
    I: IntoIterator<Item = (u8, BigInt)>,
{
    let mut normalized = markers
        .into_iter()
        .filter_map(|(id, time)| {
            let time = time.to_u64()?;
            range
                .contains(time)
                .then_some(SignalAnalysisMarker { id, time })
        })
        .collect::<Vec<_>>();

    normalized.sort_unstable_by(|lhs, rhs| lhs.time.cmp(&rhs.time).then(lhs.id.cmp(&rhs.id)));
    normalized.dedup_by(|lhs, rhs| lhs.time == rhs.time);
    normalized
}

/// Build interval metadata from normalized markers.
///
/// Expected input markers are sorted ascending by time and unique by timestamp.
#[must_use]
pub fn build_intervals(
    range: SignalAnalysisTimeRange,
    markers: &[SignalAnalysisMarker],
) -> Vec<SignalAnalysisInterval> {
    if markers.is_empty() {
        return vec![SignalAnalysisInterval {
            start: range.start,
            end: range.end,
            label: GLOBAL_LABEL.to_string(),
        }];
    }

    let mut intervals = Vec::with_capacity(markers.len() + 1);

    let first = markers[0];
    intervals.push(SignalAnalysisInterval {
        start: range.start,
        end: first.time,
        label: format!("start -> Marker {}", first.id),
    });

    for marker_pair in markers.windows(2) {
        let prev = marker_pair[0];
        let next = marker_pair[1];
        intervals.push(SignalAnalysisInterval {
            start: prev.time,
            end: next.time,
            label: format!("Marker {} -> Marker {}", prev.id, next.id),
        });
    }

    let last = markers[markers.len() - 1];
    intervals.push(SignalAnalysisInterval {
        start: last.time,
        end: range.end,
        label: format!("Marker {} -> end", last.id),
    });

    intervals
}

/// Return the interval index for a trigger time.
///
/// Returns `None` when the time is outside the run range.
#[must_use]
pub fn interval_index_for_time(
    time: u64,
    range: SignalAnalysisTimeRange,
    markers: &[SignalAnalysisMarker],
) -> Option<usize> {
    if !range.contains(time) {
        return None;
    }

    Some(markers.partition_point(|marker| marker.time <= time))
}

/// Accumulate interval and global metrics for all analyzed signals.
///
/// `sample(signal_idx, trigger_time)` should return numeric value for that signal at that time.
/// Non-numeric values (`None`, NaN, +/-inf) are ignored.
#[must_use]
pub fn accumulate_signal_metrics<F>(
    trigger_times: &[u64],
    range: SignalAnalysisTimeRange,
    markers: &[SignalAnalysisMarker],
    signal_count: usize,
    mut sample: F,
) -> SignalAnalysisAccumulation
where
    F: FnMut(usize, u64) -> Option<f64>,
{
    let interval_count = markers.len().saturating_add(1);
    let mut per_interval =
        vec![vec![MetricsAccumulator::default(); signal_count]; interval_count.max(1)];
    let mut global = vec![MetricsAccumulator::default(); signal_count];

    for &trigger_time in trigger_times {
        let Some(interval_idx) = interval_index_for_time(trigger_time, range, markers) else {
            continue;
        };

        for signal_idx in 0..signal_count {
            let Some(value) = sample(signal_idx, trigger_time) else {
                continue;
            };

            if !value.is_finite() {
                continue;
            }

            per_interval[interval_idx][signal_idx].update(value);
            global[signal_idx].update(value);
        }
    }

    let per_interval = per_interval
        .into_iter()
        .map(|signal_accumulators| {
            signal_accumulators
                .into_iter()
                .map(MetricsAccumulator::into_metrics)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    let global = global
        .into_iter()
        .map(MetricsAccumulator::into_metrics)
        .collect::<Vec<_>>();

    SignalAnalysisAccumulation {
        per_interval,
        global,
    }
}

fn collect_posedge_trigger_times<I>(changes: I) -> Vec<u64>
where
    I: IntoIterator<Item = (u64, VariableValue)>,
{
    let mut previous_level = None;

    changes
        .into_iter()
        .filter_map(|(time, value)| {
            let level = logic_level(&value);
            let is_posedge = matches!((previous_level, level), (Some(false), Some(true)));
            previous_level = level;
            is_posedge.then_some(time)
        })
        .collect()
}

fn logic_level(value: &VariableValue) -> Option<bool> {
    match value {
        VariableValue::BigUint(bits) => {
            if bits.is_zero() {
                Some(false)
            } else if bits.is_one() {
                Some(true)
            } else {
                None
            }
        }
        VariableValue::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return None;
            }

            if trimmed.bytes().all(|ch| ch == b'0') {
                return Some(false);
            }
            if trimmed.bytes().all(|ch| ch == b'1') {
                return Some(true);
            }

            let [single] = trimmed.as_bytes() else {
                return None;
            };

            match single.to_ascii_lowercase() {
                b'0' | b'l' => Some(false),
                b'1' | b'h' => Some(true),
                _ => None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SignalAnalysisMarker, SignalAnalysisMetrics, SignalAnalysisTimeRange,
        accumulate_signal_metrics, build_intervals, collect_trigger_times, infer_sampling_mode,
        interval_index_for_time, normalize_markers, normalize_time_range,
    };
    use crate::table::SignalAnalysisSamplingMode;
    use crate::wave_container::{VariableMeta, VariableRef, VariableRefExt};
    use num::BigInt;
    use std::collections::HashMap;
    use surfer_translation_types::{VariableEncoding, VariableValue};

    fn test_meta(num_bits: Option<u32>, encoding: VariableEncoding) -> VariableMeta {
        VariableMeta {
            var: VariableRef::from_hierarchy_string("tb.sig"),
            num_bits,
            variable_type: None,
            variable_type_name: None,
            index: None,
            direction: None,
            enum_map: HashMap::new(),
            encoding,
        }
    }

    fn assert_close(actual: f64, expected: f64) {
        let delta = (actual - expected).abs();
        assert!(
            delta < 1e-9,
            "expected {expected}, got {actual}, delta={delta}"
        );
    }

    fn assert_metrics(
        actual: Option<SignalAnalysisMetrics>,
        count: u64,
        average: f64,
        min: f64,
        max: f64,
        sum: f64,
    ) {
        let metrics = actual.expect("expected numeric metrics");
        assert_eq!(metrics.count, count);
        assert_close(metrics.average, average);
        assert_close(metrics.min, min);
        assert_close(metrics.max, max);
        assert_close(metrics.sum, sum);
    }

    #[test]
    fn infer_sampling_mode_prioritizes_event_over_width() {
        let event_meta = test_meta(Some(1), VariableEncoding::Event);
        let bit_meta = test_meta(Some(1), VariableEncoding::BitVector);
        let bus_meta = test_meta(Some(8), VariableEncoding::BitVector);
        let unknown_width_meta = test_meta(None, VariableEncoding::BitVector);

        assert_eq!(
            infer_sampling_mode(&event_meta),
            SignalAnalysisSamplingMode::Event
        );
        assert_eq!(
            infer_sampling_mode(&bit_meta),
            SignalAnalysisSamplingMode::PosEdge
        );
        assert_eq!(
            infer_sampling_mode(&bus_meta),
            SignalAnalysisSamplingMode::AnyChange
        );
        assert_eq!(
            infer_sampling_mode(&unknown_width_meta),
            SignalAnalysisSamplingMode::AnyChange
        );
    }

    #[test]
    fn collect_trigger_times_event_and_any_change_use_all_changes() {
        let changes = vec![
            (1, VariableValue::String("Event".to_string())),
            (1, VariableValue::String("Event".to_string())),
            (3, VariableValue::BigUint(0u8.into())),
            (5, VariableValue::BigUint(1u8.into())),
        ];

        let event = collect_trigger_times(SignalAnalysisSamplingMode::Event, changes.clone());
        let any_change =
            collect_trigger_times(SignalAnalysisSamplingMode::AnyChange, changes.clone());

        assert_eq!(event, vec![1, 1, 3, 5]);
        assert_eq!(any_change, vec![1, 1, 3, 5]);
    }

    #[test]
    fn collect_trigger_times_posedge_requires_explicit_zero_to_one_transition() {
        let changes = vec![
            (0, VariableValue::BigUint(0u8.into())),
            (5, VariableValue::BigUint(1u8.into())),
            (6, VariableValue::BigUint(1u8.into())),
            (7, VariableValue::BigUint(0u8.into())),
            (8, VariableValue::BigUint(1u8.into())),
            (9, VariableValue::String("x".to_string())),
            (10, VariableValue::BigUint(1u8.into())),
            (11, VariableValue::String("L".to_string())),
            (12, VariableValue::String("H".to_string())),
            (13, VariableValue::String("Event".to_string())),
            (14, VariableValue::BigUint(1u8.into())),
        ];

        let triggers = collect_trigger_times(SignalAnalysisSamplingMode::PosEdge, changes);
        assert_eq!(triggers, vec![5, 8, 12]);
    }

    #[test]
    fn normalize_time_range_rejects_invalid_or_non_u64_bounds() {
        assert_eq!(
            normalize_time_range(&BigInt::from(10), &BigInt::from(9)),
            None
        );
        assert_eq!(
            normalize_time_range(&BigInt::from(-1), &BigInt::from(10)),
            None
        );

        let range = normalize_time_range(&BigInt::from(2), &BigInt::from(5)).expect("range");
        assert_eq!(range, SignalAnalysisTimeRange { start: 2, end: 5 });
    }

    #[test]
    fn normalize_markers_sorts_clips_and_deduplicates_by_timestamp() {
        let range = SignalAnalysisTimeRange::new(10, 30).expect("range");
        let markers = normalize_markers(
            vec![
                (2, BigInt::from(20)),
                (1, BigInt::from(5)),
                (3, BigInt::from(20)),
                (4, BigInt::from(31)),
                (5, BigInt::from(10)),
                (6, BigInt::from(30)),
                (7, BigInt::from(-10)),
            ],
            range,
        );

        assert_eq!(
            markers,
            vec![
                SignalAnalysisMarker { id: 5, time: 10 },
                SignalAnalysisMarker { id: 2, time: 20 },
                SignalAnalysisMarker { id: 6, time: 30 },
            ]
        );
    }

    #[test]
    fn build_intervals_creates_global_only_when_markers_are_empty() {
        let range = SignalAnalysisTimeRange::new(10, 30).expect("range");
        let intervals = build_intervals(range, &[]);

        assert_eq!(intervals.len(), 1);
        assert_eq!(intervals[0].start, 10);
        assert_eq!(intervals[0].end, 30);
        assert_eq!(intervals[0].label, "GLOBAL");
    }

    #[test]
    fn build_intervals_creates_n_plus_one_marker_intervals() {
        let range = SignalAnalysisTimeRange::new(10, 30).expect("range");
        let markers = vec![
            SignalAnalysisMarker { id: 5, time: 10 },
            SignalAnalysisMarker { id: 2, time: 20 },
            SignalAnalysisMarker { id: 6, time: 30 },
        ];

        let intervals = build_intervals(range, &markers);

        assert_eq!(intervals.len(), 4);
        assert_eq!(intervals[0].label, "start -> Marker 5");
        assert_eq!(intervals[0].start, 10);
        assert_eq!(intervals[0].end, 10);

        assert_eq!(intervals[1].label, "Marker 5 -> Marker 2");
        assert_eq!(intervals[1].start, 10);
        assert_eq!(intervals[1].end, 20);

        assert_eq!(intervals[2].label, "Marker 2 -> Marker 6");
        assert_eq!(intervals[2].start, 20);
        assert_eq!(intervals[2].end, 30);

        assert_eq!(intervals[3].label, "Marker 6 -> end");
        assert_eq!(intervals[3].start, 30);
        assert_eq!(intervals[3].end, 30);
    }

    #[test]
    fn interval_index_for_time_applies_half_open_boundaries_with_final_inclusive_end() {
        let range = SignalAnalysisTimeRange::new(0, 10).expect("range");
        let markers = vec![
            SignalAnalysisMarker { id: 1, time: 3 },
            SignalAnalysisMarker { id: 2, time: 7 },
        ];

        assert_eq!(interval_index_for_time(0, range, &markers), Some(0));
        assert_eq!(interval_index_for_time(2, range, &markers), Some(0));
        assert_eq!(interval_index_for_time(3, range, &markers), Some(1));
        assert_eq!(interval_index_for_time(6, range, &markers), Some(1));
        assert_eq!(interval_index_for_time(7, range, &markers), Some(2));
        assert_eq!(interval_index_for_time(10, range, &markers), Some(2));
        assert_eq!(interval_index_for_time(11, range, &markers), None);
    }

    #[test]
    fn accumulate_signal_metrics_handles_nan_non_numeric_and_empty_intervals() {
        let range = SignalAnalysisTimeRange::new(0, 10).expect("range");
        let markers = vec![SignalAnalysisMarker { id: 1, time: 5 }];
        let trigger_times = vec![1, 2, 5, 8, 10];

        let accumulation =
            accumulate_signal_metrics(&trigger_times, range, &markers, 2, |signal_idx, time| {
                match signal_idx {
                    0 => Some(time as f64),
                    1 => match time {
                        5 => Some(-5.0),
                        8 => Some(f64::NAN),
                        10 => Some(-10.0),
                        _ => None,
                    },
                    _ => None,
                }
            });

        assert_eq!(accumulation.per_interval.len(), 2);
        assert_eq!(accumulation.per_interval[0].len(), 2);
        assert_eq!(accumulation.global.len(), 2);

        assert_metrics(accumulation.per_interval[0][0], 2, 1.5, 1.0, 2.0, 3.0);
        assert_metrics(
            accumulation.per_interval[1][0],
            3,
            23.0 / 3.0,
            5.0,
            10.0,
            23.0,
        );
        assert_metrics(accumulation.global[0], 5, 5.2, 1.0, 10.0, 26.0);

        assert_eq!(accumulation.per_interval[0][1], None);
        assert_metrics(accumulation.per_interval[1][1], 2, -7.5, -10.0, -5.0, -15.0);
        assert_metrics(accumulation.global[1], 2, -7.5, -10.0, -5.0, -15.0);
    }

    #[test]
    fn accumulate_signal_metrics_ignores_out_of_range_triggers() {
        let range = SignalAnalysisTimeRange::new(0, 10).expect("range");
        let markers = vec![
            SignalAnalysisMarker { id: 1, time: 3 },
            SignalAnalysisMarker { id: 2, time: 6 },
        ];
        let trigger_times = vec![0, 2, 8, 11];

        let accumulation =
            accumulate_signal_metrics(&trigger_times, range, &markers, 1, |_signal_idx, _time| {
                Some(1.0)
            });

        assert_eq!(accumulation.per_interval.len(), 3);
        assert_metrics(accumulation.per_interval[0][0], 2, 1.0, 1.0, 1.0, 2.0);
        assert_eq!(accumulation.per_interval[1][0], None);
        assert_metrics(accumulation.per_interval[2][0], 1, 1.0, 1.0, 1.0, 1.0);
        assert_metrics(accumulation.global[0], 3, 1.0, 1.0, 1.0, 3.0);
    }
}
