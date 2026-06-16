use super::support::test_runtime;
use super::*;
use crate::table::{TableAction, TableCell, TableColumnKey, TableSortKey};
use crate::transaction_container::TransactionStreamRef;
use ftr_parser::types::{GeneratorId, StreamId, TransactionId};

// ========================
// FTR Event Table Model
// ========================

/// Loads examples/ftr_events.ftr: stream 1 "i_test.CPU_Core" holds
/// generators 3 "instruction" / 4 "instruction.events", stream 2
/// "i_test.Memory" holds 5 "bus_transaction" / 6 "bus_transaction.events".
fn load_events_state() -> SystemState {
    let mut state = SystemState::new_default_config()
        .expect("state")
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .expect("project root")
                    .join("examples/ftr_events.ftr")
                    .try_into()
                    .expect("path"),
            )),
            ..Default::default()
        });
    wait_for_waves_fully_loaded(&mut state, 10);

    // Trigger lazy-loading of both streams
    state.update(Message::AddStreamOrGenerator(
        TransactionStreamRef::new_stream(StreamId(1), "i_test.CPU_Core".to_string()),
    ));
    state.update(Message::AddStreamOrGenerator(
        TransactionStreamRef::new_stream(StreamId(2), "i_test.Memory".to_string()),
    ));
    wait_for_waves_fully_loaded(&mut state, 10);

    state
}

fn instruction_generator_ref() -> TransactionStreamRef {
    TransactionStreamRef::new_gen(StreamId(1), GeneratorId(3), "instruction".to_string())
}

fn cell_text(cell: &TableCell) -> String {
    match cell {
        TableCell::Text(text) => text.clone(),
        TableCell::RichText(text) => text.text().to_string(),
    }
}

#[test]
fn event_table_model_spec_creates_model() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let state = load_events_state();

    let spec = TableModelSpec::EventTable {
        source: crate::source::SourceId::default(),
        generator: instruction_generator_ref(),
    };
    let ctx = state.table_model_context();
    let model = spec.create_model(&ctx);
    assert!(model.is_ok(), "EventTable model should be created");
}

#[test]
fn event_table_rejects_generator_without_events() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let state = load_events_state();

    // The events generator itself has no events generator of its own
    let spec = TableModelSpec::EventTable {
        source: crate::source::SourceId::default(),
        generator: TransactionStreamRef::new_gen(
            StreamId(1),
            GeneratorId(4),
            "instruction.events".to_string(),
        ),
    };
    let ctx = state.table_model_context();
    assert!(spec.create_model(&ctx).is_err());
}

#[test]
fn event_table_has_promoted_columns() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let state = load_events_state();

    let spec = TableModelSpec::EventTable {
        source: crate::source::SourceId::default(),
        generator: instruction_generator_ref(),
    };
    let ctx = state.table_model_context();
    let model = spec.create_model(&ctx).expect("model");
    let schema = model.schema();

    let keys: Vec<String> = schema
        .columns
        .iter()
        .map(|column| match &column.key {
            TableColumnKey::Str(key) => key.clone(),
            TableColumnKey::Id(id) => id.to_string(),
        })
        .collect();
    assert_eq!(keys[0], "time");
    assert_eq!(keys[1], "duration");
    assert_eq!(keys[2], "name");
    assert_eq!(keys[3], "parent");
    // Attribute columns discovered from the events, excluding the promoted
    // BEGIN name attribute
    assert!(keys.contains(&"attr_stage_id".to_string()));
    assert!(keys.contains(&"attr_pc".to_string()));
    assert!(!keys.iter().any(|key| key == "attr_name"));
}

#[test]
fn event_table_rows_have_names_and_parents() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let state = load_events_state();

    let spec = TableModelSpec::EventTable {
        source: crate::source::SourceId::default(),
        generator: instruction_generator_ref(),
    };
    let ctx = state.table_model_context();
    let model = spec.create_model(&ctx).expect("model");

    // instruction.events has 50 events
    assert_eq!(model.row_count(), 50);

    // First row (sorted by time): event tx#2 "IF" of instruction tx#1
    let first = model.row_id_at(0).expect("row");
    assert_eq!(cell_text(&model.cell(first, 2)), "IF");
    assert_eq!(cell_text(&model.cell(first, 3)), "instruction #1");
    // Zero duration
    assert_eq!(model.sort_key(first, 1), TableSortKey::Numeric(0.0));
}

#[test]
fn event_table_activation_focuses_event() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let state = load_events_state();

    let spec = TableModelSpec::EventTable {
        source: crate::source::SourceId::default(),
        generator: instruction_generator_ref(),
    };
    let ctx = state.table_model_context();
    let model = spec.create_model(&ctx).expect("model");

    let first = model.row_id_at(0).expect("row");
    match model.on_activate(first) {
        TableAction::FocusTransaction(source, tx_ref) => {
            assert_eq!(source, crate::source::SourceId::default());
            assert_eq!(tx_ref.id, TransactionId(2));
        }
        other => panic!("expected FocusTransaction, got {other:?}"),
    }
}

#[test]
fn event_table_search_text_includes_name_and_parent() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let state = load_events_state();

    let spec = TableModelSpec::EventTable {
        source: crate::source::SourceId::default(),
        generator: instruction_generator_ref(),
    };
    let ctx = state.table_model_context();
    let model = spec.create_model(&ctx).expect("model");

    let first = model.row_id_at(0).expect("row");
    let text = model.search_text(first);
    assert!(text.contains("IF"), "search text was: {text}");
    assert!(text.contains("instruction #1"), "search text was: {text}");
}

#[test]
fn transaction_trace_gains_events_count_column() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let state = load_events_state();

    let spec = TableModelSpec::TransactionTrace {
        source: crate::source::SourceId::default(),
        generator: instruction_generator_ref(),
    };
    let ctx = state.table_model_context();
    let model = spec.create_model(&ctx).expect("model");
    let schema = model.schema();

    let keys: Vec<String> = schema
        .columns
        .iter()
        .map(|column| match &column.key {
            TableColumnKey::Str(key) => key.clone(),
            TableColumnKey::Id(id) => id.to_string(),
        })
        .collect();
    assert_eq!(
        keys[4], "events",
        "events count column should follow the fixed columns"
    );

    // instruction tx#1 has 5 events (IF/ID/EX/MEM/WB)
    let first = model.row_id_at(0).expect("row");
    assert_eq!(cell_text(&model.cell(first, 4)), "5");
    assert_eq!(model.sort_key(first, 4), TableSortKey::Numeric(5.0));
}

#[test]
fn transaction_trace_without_events_has_no_events_column() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let state = load_events_state();

    // The raw events generator has no events generator of its own
    let spec = TableModelSpec::TransactionTrace {
        source: crate::source::SourceId::default(),
        generator: TransactionStreamRef::new_gen(
            StreamId(1),
            GeneratorId(4),
            "instruction.events".to_string(),
        ),
    };
    let ctx = state.table_model_context();
    let model = spec.create_model(&ctx).expect("model");
    let schema = model.schema();

    assert!(!schema.columns.iter().any(|column| matches!(
        &column.key,
        TableColumnKey::Str(key) if key == "events"
    )));
}
