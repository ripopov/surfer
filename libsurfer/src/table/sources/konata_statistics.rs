use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::{
    konata::{FlushState, KonataInstructionClassifier, KonataModel, KonataModelSpec, KonataScalar},
    table::{
        TableAction, TableCacheError, TableCell, TableColumn, TableColumnKey, TableModel,
        TableRowId, TableSchema, TableSortKey,
    },
};

#[derive(Clone)]
pub struct KonataStatisticsInput {
    pub spec: KonataModelSpec,
    pub model: Arc<KonataModel>,
    pub clock_period_ticks: Option<u64>,
    pub clock_origin_tick: i64,
    pub range: Option<(u64, u64)>,
    pub stall_stages: String,
    pub stall_case_sensitive: bool,
    pub instruction_classifier: KonataInstructionClassifier,
    pub include_estimated_flush_rates: bool,
    pub cancel: Option<Arc<AtomicBool>>,
}

#[derive(Debug, Clone)]
struct StatisticRow {
    category: String,
    name: String,
    thread: String,
    value: Option<f64>,
    unit: String,
    count: Option<u64>,
    average: Option<f64>,
    maximum: Option<u64>,
    total: Option<u64>,
    status: String,
}

pub struct KonataStatisticsTableModel {
    spec: KonataModelSpec,
    rows: Vec<StatisticRow>,
}

#[derive(Default)]
struct StageAggregate {
    count: u64,
    total: u64,
    maximum: u64,
}

#[derive(Default)]
struct ThreadAggregate {
    fetched: u64,
    committed: u64,
    flushed: u64,
    first_fetch: Option<u64>,
    last_retire: Option<u64>,
}

impl KonataStatisticsTableModel {
    pub fn try_new(input: KonataStatisticsInput) -> Result<Self, TableCacheError> {
        let rows = futures::executor::block_on(build_statistics(&input, false))?;
        Ok(Self {
            spec: input.spec,
            rows,
        })
    }

    pub async fn try_new_cooperative(
        input: KonataStatisticsInput,
    ) -> Result<Self, TableCacheError> {
        let rows = build_statistics(&input, true).await?;
        Ok(Self {
            spec: input.spec,
            rows,
        })
    }

    fn text(value: impl ToString) -> TableCell {
        TableCell::Text(value.to_string())
    }
}

async fn build_statistics(
    input: &KonataStatisticsInput,
    cooperative: bool,
) -> Result<Vec<StatisticRow>, TableCacheError> {
    let model = &input.model;
    let in_range = |tick: u64| {
        input
            .range
            .is_none_or(|(start, end)| tick >= start && tick < end)
    };
    let mut fetched = 0u64;
    let mut committed = 0u64;
    let mut flushed = 0u64;
    let mut unknown_retirement = 0u64;
    let mut flush_episodes = 0u64;
    let mut in_flush = false;
    let mut first_fetch = None;
    let mut last_retire = None;
    let mut threads = BTreeMap::<String, ThreadAggregate>::new();
    let mut exact_causes = BTreeMap::<(String, String), u64>::new();
    let mut estimated_causes = BTreeMap::<(String, String), u64>::new();
    let mut committed_classes = BTreeMap::<String, u64>::new();
    let mut previous_committed_row = None;

    for row in 0..model.row_count() {
        if row % 4096 == 0 && is_cancelled(input) {
            return Err(TableCacheError::Cancelled);
        }
        if cooperative && row % 4096 == 0 {
            crate::async_util::sleep_ms(0).await;
        }
        let row_fetched = in_range(model.rows.begin[row]);
        let row_committed = model.rows.flushed[row] == FlushState::False
            && model.rows.rid(row).is_some()
            && in_range(model.rows.end[row]);
        let row_flushed = model.rows.flushed[row] == FlushState::True && row_fetched;
        fetched += u64::from(row_fetched);
        committed += u64::from(row_committed);
        flushed += u64::from(row_flushed);
        unknown_retirement += u64::from(
            row_fetched
                && model.rows.flushed[row] != FlushState::True
                && model.rows.rid(row).is_none(),
        );
        if row_flushed && !in_flush {
            flush_episodes += 1;
            if let Some((cause, provenance)) = recorded_flush_cause(input, row) {
                *exact_causes.entry((cause, provenance)).or_default() += 1;
            } else {
                let (cause, provenance) = previous_committed_row.map_or_else(
                    || {
                        (
                            "Unclassified".to_string(),
                            "no preceding committed op".to_string(),
                        )
                    },
                    |cause_row| classify_instruction(input, cause_row),
                );
                *estimated_causes.entry((cause, provenance)).or_default() += 1;
            }
        }
        if row_flushed {
            in_flush = true;
        } else if row_committed {
            in_flush = false;
        }
        if row_fetched {
            first_fetch = Some(first_fetch.map_or(model.rows.begin[row], |tick: u64| {
                tick.min(model.rows.begin[row])
            }));
        }
        if row_committed {
            last_retire = Some(last_retire.map_or(model.rows.end[row], |tick: u64| {
                tick.max(model.rows.end[row])
            }));
            previous_committed_row = Some(row);
            let (class, _) = classify_instruction(input, row);
            *committed_classes.entry(class).or_default() += 1;
        }

        let thread = model
            .rows
            .tid(row)
            .map_or("—", |tid| model.thread_name(tid))
            .to_string();
        let aggregate = threads.entry(thread).or_default();
        aggregate.fetched += u64::from(row_fetched);
        aggregate.committed += u64::from(row_committed);
        aggregate.flushed += u64::from(row_flushed);
        if row_fetched {
            aggregate.first_fetch =
                Some(aggregate.first_fetch.map_or(model.rows.begin[row], |tick| {
                    tick.min(model.rows.begin[row])
                }));
        }
        if row_committed {
            aggregate.last_retire = Some(
                aggregate
                    .last_retire
                    .map_or(model.rows.end[row], |tick| tick.max(model.rows.end[row])),
            );
        }
    }

    let elapsed_bounds = input
        .range
        .map_or((first_fetch, last_retire), |(start, end)| {
            (Some(start), Some(end))
        });
    let elapsed = elapsed_cycles(
        elapsed_bounds.0,
        elapsed_bounds.1,
        input.clock_period_ticks,
        input.clock_origin_tick,
    );
    let mut rows = vec![
        summary("Fetched ops", Some(fetched as f64), "ops", "exact"),
        summary("Committed ops", Some(committed as f64), "ops", "exact"),
        summary("Flushed ops", Some(flushed as f64), "ops", "exact"),
        summary(
            "Flush episodes",
            Some(flush_episodes as f64),
            "flushes",
            "exact",
        ),
        summary(
            "Unknown retirement",
            Some(unknown_retirement as f64),
            "ops",
            if unknown_retirement == 0 {
                "exact"
            } else {
                "incomplete metadata"
            },
        ),
        summary(
            "Elapsed cycles",
            elapsed.map(|cycles| cycles as f64),
            "cycles",
            if elapsed.is_some() {
                "exact"
            } else {
                "pipeline clock required"
            },
        ),
        summary(
            "IPC",
            elapsed
                .filter(|cycles| *cycles > 0)
                .map(|cycles| committed as f64 / cycles as f64),
            "ops/cycle",
            if elapsed.is_some() {
                "exact"
            } else {
                "pipeline clock required"
            },
        ),
    ];
    rows.extend(flush_cause_rows(
        exact_causes,
        &committed_classes,
        committed,
        false,
        true,
    ));
    rows.extend(flush_cause_rows(
        estimated_causes,
        &committed_classes,
        committed,
        true,
        input.include_estimated_flush_rates,
    ));

    let mut stages = BTreeMap::<String, StageAggregate>::new();
    for row in 0..model.row_count() {
        if row % 4096 == 0 && is_cancelled(input) {
            return Err(TableCacheError::Cancelled);
        }
        if cooperative && row % 4096 == 0 {
            crate::async_util::sleep_ms(0).await;
        }
        for stage in model.stages_for_row_blocking(row).iter() {
            let start = input
                .range
                .map_or(stage.start, |(start, _)| stage.start.max(start));
            let end = input.range.map_or(stage.end, |(_, end)| stage.end.min(end));
            if end <= start {
                continue;
            }
            let duration = end - start;
            let aggregate = stages
                .entry(model.stage_name(stage).to_string())
                .or_default();
            aggregate.count += 1;
            aggregate.total = aggregate.total.saturating_add(duration);
            aggregate.maximum = aggregate.maximum.max(duration);
        }
    }
    rows.extend(stages.into_iter().map(|(name, aggregate)| {
        let stall = input
            .stall_stages
            .split(',')
            .map(str::trim)
            .any(|candidate| {
                if input.stall_case_sensitive {
                    candidate == name
                } else {
                    candidate.eq_ignore_ascii_case(&name)
                }
            });
        StatisticRow {
            category: "Stage".to_string(),
            name,
            thread: String::new(),
            value: None,
            unit: "ticks".to_string(),
            count: Some(aggregate.count),
            average: (aggregate.count > 0).then(|| aggregate.total as f64 / aggregate.count as f64),
            maximum: Some(aggregate.maximum),
            total: Some(aggregate.total),
            status: if stall {
                "stall cycles".to_string()
            } else {
                "recorded durations".to_string()
            },
        }
    }));

    for (thread, aggregate) in threads {
        let thread_bounds = input.range.map_or(
            (aggregate.first_fetch, aggregate.last_retire),
            |(start, end)| (Some(start), Some(end)),
        );
        let cycles = elapsed_cycles(
            thread_bounds.0,
            thread_bounds.1,
            input.clock_period_ticks,
            input.clock_origin_tick,
        );
        rows.push(StatisticRow {
            category: "Thread".to_string(),
            name: "Throughput".to_string(),
            thread,
            value: cycles
                .filter(|cycles| *cycles > 0)
                .map(|cycles| aggregate.committed as f64 / cycles as f64),
            unit: "ops/cycle".to_string(),
            count: Some(aggregate.fetched),
            average: None,
            maximum: None,
            total: Some(aggregate.committed),
            status: format!("{} flushed", aggregate.flushed),
        });
    }
    Ok(rows)
}

fn flush_cause_rows(
    causes: BTreeMap<(String, String), u64>,
    committed_classes: &BTreeMap<String, u64>,
    committed: u64,
    estimated: bool,
    include_in_rates: bool,
) -> Vec<StatisticRow> {
    causes
        .into_iter()
        .flat_map(|((cause, provenance), count)| {
            let status = if estimated && include_in_rates {
                format!("estimated by {provenance}; included by user")
            } else if estimated {
                format!("estimated by {provenance}; excluded from exact rates")
            } else {
                format!("exact: {provenance}")
            };
            let mpki = (include_in_rates && committed > 0)
                .then(|| count as f64 * 1000.0 / committed as f64);
            let classified = committed_classes.get(&cause).copied().unwrap_or_default();
            let rate = (include_in_rates && classified > 0)
                .then(|| count as f64 * 100.0 / classified as f64);
            [
                StatisticRow {
                    category: "Flush cause".to_string(),
                    name: cause.clone(),
                    thread: String::new(),
                    value: mpki,
                    unit: if estimated {
                        "estimated MPKI".to_string()
                    } else {
                        "MPKI".to_string()
                    },
                    count: Some(count),
                    average: None,
                    maximum: None,
                    total: None,
                    status: status.clone(),
                },
                StatisticRow {
                    category: "Flush rate".to_string(),
                    name: format!("{cause} miss rate"),
                    thread: String::new(),
                    value: rate,
                    unit: "%".to_string(),
                    count: Some(count),
                    average: None,
                    maximum: None,
                    total: Some(classified),
                    status,
                },
            ]
        })
        .collect()
}

fn recorded_flush_cause(
    input: &KonataStatisticsInput,
    flushed_row: usize,
) -> Option<(String, String)> {
    let model = &input.model;
    if let Some(annotation) = model.row_annotation(
        flushed_row,
        &["flush_cause", "mispredict_cause", "squash_cause"],
    ) {
        return Some((
            normalize_instruction_class(&model.scalar_text(&annotation.value)),
            format!("recorded {} attribute", model.string(annotation.name)),
        ));
    }
    if let Some(annotation) = model.row_annotation(
        flushed_row,
        &["flush_cause_tx", "cause_transaction", "cause_tx_id"],
    ) && let Some(tx_id) = scalar_u64(&annotation.value)
        && let Some(cause_row) = model.row_for_transaction(tx_id)
    {
        let (cause, _) = classify_instruction(input, cause_row);
        return Some((
            cause,
            format!(
                "recorded {} transaction reference",
                model.string(annotation.name)
            ),
        ));
    }
    model
        .incoming_dependencies(flushed_row)
        .find(|dependency| {
            let name = model.dependency_name(dependency).to_ascii_lowercase();
            ["flush", "mispredict", "squash", "redirect", "cause"]
                .iter()
                .any(|candidate| name.contains(candidate))
        })
        .map(|dependency| {
            let (cause, _) = classify_instruction(input, dependency.producer_row as usize);
            (
                cause,
                format!("recorded {} relation", model.dependency_name(dependency)),
            )
        })
}

fn scalar_u64(value: &KonataScalar) -> Option<u64> {
    match value {
        KonataScalar::Unsigned(value) | KonataScalar::Time(value) => Some(*value),
        KonataScalar::Signed(value) => u64::try_from(*value).ok(),
        _ => None,
    }
}

fn classify_instruction(input: &KonataStatisticsInput, row: usize) -> (String, String) {
    if let Some(annotation) = input.model.row_annotation(
        row,
        &["instruction_class", "op_class", "instruction_type", "class"],
    ) {
        return (
            normalize_instruction_class(&input.model.scalar_text(&annotation.value)),
            format!("recorded {} attribute", input.model.string(annotation.name)),
        );
    }
    let label = input
        .model
        .rows
        .label(row)
        .map_or("", |label| input.model.string(label));
    let class = match input.instruction_classifier {
        KonataInstructionClassifier::Generic => classify_instruction_label(label),
        KonataInstructionClassifier::X86Gem5 => classify_x86_gem5_label(label),
    };
    (
        class.to_string(),
        match input.instruction_classifier {
            KonataInstructionClassifier::Generic => "generic label classifier",
            KonataInstructionClassifier::X86Gem5 => "x86/gem5 label classifier",
        }
        .to_string(),
    )
}

fn normalize_instruction_class(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    if lower.contains("branch") || lower.contains("cond") {
        "Branch".to_string()
    } else if lower.contains("jump")
        || lower.contains("call")
        || lower.contains("return")
        || lower == "ret"
    {
        "Jump".to_string()
    } else if lower.contains("mem")
        || lower.contains("load")
        || lower.contains("store")
        || lower.contains("atomic")
    {
        "Memory speculation".to_string()
    } else if value.trim().is_empty() {
        "Unclassified".to_string()
    } else {
        value.trim().to_string()
    }
}

fn classify_instruction_label(label: &str) -> &'static str {
    let mnemonic = label
        .split_once(':')
        .map_or(label, |(_, instruction)| instruction)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(mnemonic.as_str(), "jal" | "jalr" | "jmp" | "call" | "ret") {
        "Jump"
    } else if mnemonic.starts_with('b') || mnemonic.contains("branch") {
        "Branch"
    } else if matches!(
        mnemonic.as_str(),
        "lb" | "lbu" | "lh" | "lhu" | "lw" | "lwu" | "ld" | "sb" | "sh" | "sw" | "sd"
    ) || mnemonic.contains("load")
        || mnemonic.contains("store")
    {
        "Memory speculation"
    } else {
        "Unclassified"
    }
}

fn classify_x86_gem5_label(label: &str) -> &'static str {
    let instruction = label
        .split_once(':')
        .map_or(label, |(_, instruction)| instruction);
    let mnemonic = instruction
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_end_matches(',')
        .to_ascii_lowercase();
    if matches!(
        mnemonic.as_str(),
        "jmp" | "call" | "callq" | "ret" | "retq" | "sysret" | "iret" | "iretq"
    ) {
        "Jump"
    } else if (mnemonic.starts_with('j') && mnemonic != "jmp") || mnemonic.starts_with("loop") {
        "Branch"
    } else if instruction.contains('[')
        || instruction.contains("(%")
        || instruction.to_ascii_lowercase().contains("load")
        || instruction.to_ascii_lowercase().contains("store")
    {
        "Memory speculation"
    } else {
        "Unclassified"
    }
}

fn is_cancelled(input: &KonataStatisticsInput) -> bool {
    input
        .cancel
        .as_ref()
        .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
}

fn summary(name: &str, value: Option<f64>, unit: &str, status: &str) -> StatisticRow {
    StatisticRow {
        category: "Summary".to_string(),
        name: name.to_string(),
        thread: String::new(),
        value,
        unit: unit.to_string(),
        count: None,
        average: None,
        maximum: None,
        total: None,
        status: status.to_string(),
    }
}

fn elapsed_cycles(
    start: Option<u64>,
    end: Option<u64>,
    period: Option<u64>,
    origin: i64,
) -> Option<u64> {
    let (start, end, period) = (start?, end?, period.filter(|period| *period > 0)?);
    let period = i128::from(period);
    let start = i128::from(start) - i128::from(origin);
    let end = i128::from(end) - i128::from(origin);
    let first_cycle = start.div_euclid(period);
    let end_cycle = -(-end).div_euclid(period);
    u64::try_from((end_cycle - first_cycle).max(1)).ok()
}

impl TableModel for KonataStatisticsTableModel {
    fn schema(&self) -> TableSchema {
        let column = |key: &str, label: &str, width| TableColumn {
            key: TableColumnKey::Str(key.to_string()),
            label: label.to_string(),
            default_width: Some(width),
            default_visible: true,
            default_resizable: true,
        };
        TableSchema {
            columns: vec![
                column("category", "Category", 90.0),
                column("name", "Metric", 170.0),
                column("thread", "Thread", 80.0),
                column("value", "Value", 100.0),
                column("unit", "Unit", 90.0),
                column("count", "Count", 90.0),
                column("average", "Average", 100.0),
                column("maximum", "Maximum", 100.0),
                column("total", "Total", 100.0),
                column("status", "Status", 190.0),
            ],
        }
    }

    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn row_id_at(&self, index: usize) -> Option<TableRowId> {
        (index < self.rows.len()).then_some(TableRowId(index as u64))
    }

    fn cell(&self, row: TableRowId, col: usize) -> TableCell {
        let Some(row) = self.rows.get(row.0 as usize) else {
            return Self::text("");
        };
        match col {
            0 => Self::text(&row.category),
            1 => Self::text(&row.name),
            2 => Self::text(&row.thread),
            3 => Self::text(
                row.value
                    .map_or_else(String::new, |value| format!("{value:.4}")),
            ),
            4 => Self::text(&row.unit),
            5 => Self::text(
                row.count
                    .map_or_else(String::new, |value| value.to_string()),
            ),
            6 => Self::text(
                row.average
                    .map_or_else(String::new, |value| format!("{value:.3}")),
            ),
            7 => Self::text(
                row.maximum
                    .map_or_else(String::new, |value| value.to_string()),
            ),
            8 => Self::text(
                row.total
                    .map_or_else(String::new, |value| value.to_string()),
            ),
            9 => Self::text(&row.status),
            _ => Self::text(""),
        }
    }

    fn sort_key(&self, row: TableRowId, col: usize) -> TableSortKey {
        let Some(row) = self.rows.get(row.0 as usize) else {
            return TableSortKey::None;
        };
        match col {
            3 => row.value.map_or(TableSortKey::None, TableSortKey::Numeric),
            5 => row.count.map_or(TableSortKey::None, |value| {
                TableSortKey::Numeric(value as f64)
            }),
            6 => row
                .average
                .map_or(TableSortKey::None, TableSortKey::Numeric),
            7 => row.maximum.map_or(TableSortKey::None, |value| {
                TableSortKey::Numeric(value as f64)
            }),
            8 => row.total.map_or(TableSortKey::None, |value| {
                TableSortKey::Numeric(value as f64)
            }),
            0 => TableSortKey::Text(row.category.clone()),
            1 => TableSortKey::Text(row.name.clone()),
            2 => TableSortKey::Text(row.thread.clone()),
            4 => TableSortKey::Text(row.unit.clone()),
            9 => TableSortKey::Text(row.status.clone()),
            _ => TableSortKey::None,
        }
    }

    fn search_text(&self, row: TableRowId) -> String {
        self.rows
            .get(row.0 as usize)
            .map_or_else(String::new, |row| {
                format!(
                    "{} {} {} {} {}",
                    row.category, row.name, row.thread, row.unit, row.status
                )
            })
    }

    fn on_activate(&self, _row: TableRowId) -> TableAction {
        let _ = &self.spec;
        TableAction::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        konata::KonataBuildInput,
        source::SourceId,
        transaction_container::TransactionStreamRef,
        transaction_events::test_util::{ftr_from_json, generator, stream, tx},
    };
    use ftr_parser::types::{GeneratorId, StreamId};
    use serde_json::json;

    #[test]
    fn elapsed_cycles_uses_half_open_coverage() {
        assert_eq!(elapsed_cycles(Some(10), Some(20), Some(4), 0), Some(3));
        assert_eq!(elapsed_cycles(Some(10), Some(18), Some(4), 0), Some(3));
        assert_eq!(elapsed_cycles(Some(10), Some(18), Some(4), 2), Some(2));
        assert_eq!(elapsed_cycles(Some(10), Some(18), None, 0), None);
    }

    #[test]
    fn generic_classifier_separates_branch_jump_and_memory() {
        assert_eq!(classify_instruction_label("1000: bne a0, a1, 8"), "Branch");
        assert_eq!(classify_instruction_label("1004: jal ra, 20"), "Jump");
        assert_eq!(
            classify_instruction_label("1008: lw a0, 0(sp)"),
            "Memory speculation"
        );
        assert_eq!(
            classify_instruction_label("100c: add a0, a1, a2"),
            "Unclassified"
        );
    }

    #[test]
    fn x86_classifier_separates_control_flow_and_memory() {
        assert_eq!(classify_x86_gem5_label("4000: jne 4010"), "Branch");
        assert_eq!(classify_x86_gem5_label("4004: callq 5000"), "Jump");
        assert_eq!(
            classify_x86_gem5_label("4008: mov (%rax), %rbx"),
            "Memory speculation"
        );
    }

    #[test]
    fn recorded_flush_cause_precedes_estimation() {
        let mut committed = tx(1, 10, 0, 10, None, &[]);
        committed["attributes"] = json!([
            {"kind":"RECORD", "name":"label", "data_type":{"String":"0: bne a0, a1, 8"}},
            {"kind":"RECORD", "name":"flushed", "data_type":{"Boolean":false}},
            {"kind":"RECORD", "name":"retire_id", "data_type":{"Unsigned":0}},
            {"kind":"RECORD", "name":"instruction_class", "data_type":{"String":"conditional branch"}}
        ]);
        let mut flushed = tx(2, 10, 10, 20, None, &[]);
        flushed["attributes"] = json!([
            {"kind":"RECORD", "name":"flushed", "data_type":{"Boolean":true}},
            {"kind":"RECORD", "name":"flush_cause", "data_type":{"String":"branch"}}
        ]);
        let ftr = ftr_from_json(
            json!({"1": stream(1, "cpu", &[10, 11])}),
            json!({
                "10": generator(10, 1, "instruction", json!([committed, flushed])),
                "11": generator(11, 1, "instruction.events", json!([])),
            }),
        );
        let model = Arc::new(KonataModel::build(KonataBuildInput {
            parent_generator: GeneratorId(10),
            event_generator: GeneratorId(11),
            stream: StreamId(1),
            parents: ftr
                .get_generator(GeneratorId(10))
                .unwrap()
                .transactions
                .clone(),
            events: ftr
                .get_generator(GeneratorId(11))
                .unwrap()
                .transactions
                .clone(),
            relations: ftr.tx_relations.clone(),
        }));
        let rows = futures::executor::block_on(build_statistics(
            &KonataStatisticsInput {
                spec: KonataModelSpec {
                    source: SourceId::default(),
                    generator: TransactionStreamRef::new_gen(
                        StreamId(1),
                        GeneratorId(10),
                        "instruction".to_string(),
                    ),
                },
                model,
                clock_period_ticks: Some(1),
                clock_origin_tick: 0,
                range: None,
                stall_stages: "f,stl".to_string(),
                stall_case_sensitive: true,
                instruction_classifier: KonataInstructionClassifier::Generic,
                include_estimated_flush_rates: false,
                cancel: None,
            },
            false,
        ))
        .unwrap();
        let cause = rows
            .iter()
            .find(|row| row.category == "Flush cause" && row.name == "Branch")
            .expect("recorded branch cause");
        assert_eq!(cause.count, Some(1));
        assert!(cause.value.is_some());
        assert!(
            cause
                .status
                .contains("exact: recorded flush_cause attribute")
        );
        assert!(!cause.status.contains("estimated"));
    }
}
