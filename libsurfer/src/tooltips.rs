use egui::{Response, Ui};
use egui_extras::{Column, TableBuilder};
use ftr_parser::types::Transaction;
use itertools::Itertools;
use num::BigUint;

use crate::{
    source::SourceId,
    transaction_container::{TransactionContainer, TransactionRef, TransactionStreamRef},
    wave_container::{ScopeRef, VariableMeta, VariableRef, VariableRefExt, WaveContainer},
    wave_data::WaveData,
};

// Try to locate a transaction for the tooltip without panicking
fn find_transaction<'a>(
    waves: &'a WaveData,
    source: SourceId,
    gen_ref: &TransactionStreamRef,
    tx_ref: &TransactionRef,
) -> Option<&'a Transaction> {
    let txs = waves.transactions_for_source(source)?;
    let gen_id = gen_ref.gen_id?;
    let generator = txs.get_generator(gen_id)?;
    generator
        .transactions
        .iter()
        .find(|transaction| transaction.get_tx_id() == tx_ref.id)
}

#[must_use]
pub(crate) fn variable_tooltip_text(
    meta: Option<&VariableMeta>,
    variable: &VariableRef,
    source_label: Option<&str>,
) -> String {
    let source_prefix = source_label.map_or_else(String::new, |label| format!("Source: {label}\n"));
    if let Some(meta) = meta {
        format!(
            "{}{}\nNum bits: {}\nType: {}\nDirection: {}",
            source_prefix,
            variable.full_path_string(),
            meta.num_bits
                .map_or_else(|| "unknown".to_string(), |bits| bits.to_string()),
            meta.variable_type_name
                .clone()
                .or_else(|| meta.variable_type.map(|t| t.to_string()))
                .unwrap_or_else(|| "unknown".to_string()),
            meta.direction
                .map_or_else(|| "unknown".to_string(), |direction| format!("{direction}"))
        )
    } else {
        format!("{}{}", source_prefix, variable.full_path_string())
    }
}

#[must_use]
pub(crate) fn scope_tooltip_text_for_container(
    wave_container: &WaveContainer,
    scope: &ScopeRef,
    include_parameters: bool,
) -> String {
    let mut parts = vec![format!("{scope}")];
    if include_parameters {
        for param in &wave_container.parameters_in_scope(scope) {
            let value = wave_container
                .query_variable(param, &BigUint::ZERO)
                .ok()
                .and_then(|o| o.and_then(|q| q.current.map(|v| format!("{}", v.1))))
                .unwrap_or_else(|| "Undefined".to_string());
            parts.push(format!("{}: {}", param.name, value));
        }
    }
    let other = wave_container.get_scope_tooltip_data(scope);
    if !other.is_empty() {
        parts.push(other);
    }
    parts.join("\n")
}

#[must_use]
pub(crate) fn handle_transaction_tooltip_for_source(
    response: Response,
    waves: &WaveData,
    source: SourceId,
    gen_ref: &TransactionStreamRef,
    tx_ref: &TransactionRef,
) -> Response {
    response
        .on_hover_ui(|ui| {
            if let Some(tx) = find_transaction(waves, source, gen_ref, tx_ref) {
                ui.set_max_width(ui.spacing().tooltip_width);
                ui.add(egui::Label::new(transaction_tooltip_text(
                    waves, source, tx,
                )));
            } else {
                ui.label("Transaction unavailable");
            }
        })
        .on_hover_ui(|ui| {
            // Seemingly a bit redundant to determine tx twice, but since the
            // alternative is to do it every frame for every transaction, this
            // is most likely still a better approach.
            // Feel free to use some Rust magic to only do it once though...
            if let Some(tx) = find_transaction(waves, source, gen_ref, tx_ref) {
                transaction_tooltip_table(ui, tx);
            } else {
                ui.label("Transaction details unavailable");
            }
        })
}

/// Tooltip for an aggregated event cluster marker: per-name counts and the
/// covered time span.
#[must_use]
pub(crate) fn handle_event_cluster_tooltip(
    response: Response,
    count: usize,
    names: &[(String, usize)],
    time_span: &(num::BigInt, num::BigInt),
    time_scale: &str,
    source_label: Option<&str>,
) -> Response {
    response.on_hover_ui(|ui| {
        ui.set_max_width(ui.spacing().tooltip_width);
        if let Some(source_label) = source_label {
            ui.label(format!("Source: {source_label}"));
        }
        let breakdown = names
            .iter()
            .map(|(name, n)| {
                let label = if name.is_empty() { "<unnamed>" } else { name };
                format!("{label} ×{n}")
            })
            .join(", ");
        ui.add(egui::Label::new(format!(
            "{count} events: {breakdown}\n{}{time_scale} - {}{time_scale}\nClick to zoom in",
            time_span.0, time_span.1,
        )));
    })
}

fn transaction_tooltip_text(waves: &WaveData, source: SourceId, tx: &Transaction) -> String {
    let Some(transactions) = waves.transactions_for_source(source) else {
        return format!("tx#{}: Transaction source unavailable", tx.event.tx_id);
    };
    let time_scale = transactions.inner.time_scale.to_string();
    let source_prefix = (waves.source_count() > 1)
        .then(|| waves.source_label_for(source))
        .flatten()
        .map_or_else(String::new, |label| format!("Source: {label}\n"));

    if let Some(text) = event_tooltip_text(transactions, tx, &time_scale) {
        return format!("{source_prefix}{text}");
    }

    let mut text = format!(
        "{source_prefix}tx#{}: {}{} - {}{}\nType: {}",
        tx.event.tx_id,
        tx.event.start_time,
        time_scale,
        tx.event.end_time,
        time_scale,
        transactions
            .get_generator(tx.get_gen_id())
            .map_or_else(|| "unknown".to_string(), |g| g.name.clone()),
    );

    // One summary line for the transaction's events
    let events = transactions.events_of_parent(tx.get_tx_id());
    if !events.is_empty() {
        let names = events
            .iter()
            .filter_map(|event_id| {
                transactions
                    .get_transaction(&TransactionRef { id: *event_id })
                    .and_then(crate::transaction_events::event_name)
            })
            .collect::<Vec<_>>();
        let mut summary = names.iter().take(3).join(", ");
        if names.len() > 3 {
            summary.push_str(&format!(" +{} more", names.len() - 3));
        }
        text.push_str(&format!("\nevents: {} ({summary})", events.len()));
    }
    text
}

/// Event-specific tooltip: the event name leads, followed by time, duration,
/// a parent summary, and any convention violations.
fn event_tooltip_text(
    transactions: &TransactionContainer,
    tx: &Transaction,
    time_scale: &str,
) -> Option<String> {
    let index = transactions.event_index();
    let tx_id = tx.get_tx_id();
    if !index.is_events_generator(tx.get_gen_id()) {
        return None;
    }

    let name =
        crate::transaction_events::event_name(tx).unwrap_or_else(|| format!("event tx#{tx_id}"));
    let mut lines = vec![name];

    let duration = tx.event.end_time - tx.event.start_time;
    if duration == 0 {
        lines.push(format!("Time: {}{time_scale}", tx.event.start_time));
    } else {
        lines.push(format!(
            "Time: {}{time_scale}   Duration: {duration}{time_scale}",
            tx.event.start_time
        ));
    }

    if let Some(info) = index.event_info(tx_id) {
        if let Some(parent) = transactions.get_transaction(&TransactionRef { id: info.parent_tx }) {
            let parent_gen_name = transactions
                .get_generator(info.parent_gen)
                .map_or_else(|| "unknown".to_string(), |g| g.name.clone());
            lines.push(format!(
                "in {parent_gen_name} tx#{}, {}{time_scale} - {}{time_scale}",
                info.parent_tx, parent.event.start_time, parent.event.end_time
            ));
        }
        if info.out_of_range {
            lines.push("⚠ outside parent transaction range".to_string());
        }
        if info.multiple_parents {
            lines.push("⚠ multiple parents recorded; showing first".to_string());
        }
    } else {
        lines.push("orphan event (no parent_of relation)".to_string());
    }

    Some(lines.join("\n"))
}

fn transaction_tooltip_table(ui: &mut Ui, tx: &Transaction) {
    TableBuilder::new(ui)
        .column(Column::exact(80.))
        .column(Column::exact(80.))
        .header(20.0, |mut header| {
            header.col(|ui| {
                ui.heading("Attribute");
            });
            header.col(|ui| {
                ui.heading("Value");
            });
        })
        .body(|body| {
            let total_rows = tx.attributes.len();
            let attributes = &tx.attributes;
            body.rows(15., total_rows, |mut row| {
                if let Some(attribute) = attributes.get(row.index()) {
                    row.col(|ui| {
                        ui.label(attribute.name.to_string());
                    });
                    row.col(|ui| {
                        ui.label(attribute.value());
                    });
                }
            });
        });
}

#[cfg(test)]
mod tests {
    use ftr_parser::types::{GeneratorId, StreamId};
    use project_root::get_project_root;

    use super::*;
    use crate::{
        Message, StartupParams, SystemState,
        transaction_container::TransactionStreamRef,
        wave_source::{LoadIntent, WaveFormat},
    };

    fn fixture(path: &str) -> camino::Utf8PathBuf {
        get_project_root().unwrap().join(path).try_into().unwrap()
    }

    #[test]
    fn variable_tooltip_includes_source_label_when_provided() {
        let variable = VariableRef::from_hierarchy_string("tb.clk");
        let tooltip = variable_tooltip_text(None, &variable, Some("waves.vcd"));

        assert!(tooltip.starts_with("Source: waves.vcd\n"));
        assert!(tooltip.contains("tb.clk"));
    }

    #[test]
    fn additive_ftr_transaction_tooltip_uses_its_source_container() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();

        let mut state = SystemState::new_default_config()
            .unwrap()
            .with_params(StartupParams::default());
        state.update(Message::LoadFilesWithIntents(vec![
            (
                fixture("examples/fused_ftr_wave.vcd"),
                LoadIntent::ReplaceSession,
            ),
            (fixture("examples/my_db.ftr"), LoadIntent::AddSource),
        ]));
        crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);
        state.update(Message::AddStreamOrGeneratorFromSource(
            SourceId(1),
            TransactionStreamRef::new_gen(
                StreamId(1),
                GeneratorId(4),
                "pipelined_stream.read".to_string(),
            ),
        ));
        crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

        let waves = state.user.waves.as_ref().expect("waves loaded");
        assert_eq!(waves.format, WaveFormat::Vcd);
        let transactions = waves
            .transactions_for_source(SourceId(1))
            .expect("additive FTR source");
        let tx = transactions
            .get_generator(GeneratorId(4))
            .expect("read generator")
            .transactions
            .first()
            .expect("transaction");

        let tooltip = transaction_tooltip_text(waves, SourceId(1), tx);
        assert!(
            tooltip.starts_with("Source: my_db.ftr\n"),
            "expected additive source label in tooltip, got: {tooltip}"
        );
        assert!(
            tooltip.contains("Type: read"),
            "expected additive-source generator name in tooltip, got: {tooltip}"
        );
        assert!(
            !tooltip.contains("Type: unknown"),
            "tooltip must not fall back to the primary VCD container: {tooltip}"
        );
        assert!(
            tooltip.to_ascii_lowercase().contains("ns"),
            "expected additive FTR time scale in tooltip, got: {tooltip}"
        );
    }
}
