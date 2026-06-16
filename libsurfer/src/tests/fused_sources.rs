use ftr_parser::types::{GeneratorId, StreamId, TransactionId};
use num::BigInt;
use project_root::get_project_root;
use tempfile::tempdir;

use crate::{
    Message, StartupParams, SystemState, WaveSource,
    data_container::DataContainer,
    displayed_item::DisplayedItem,
    source::{LoadRequestId, SourceId, SourceLoadState, SourceTransactionRef},
    table::sources::decode_signal_column_key,
    table::{
        AnalysisKind, AnalysisParams, MultiSignalEntry, SignalAnalysisConfig,
        SignalAnalysisSamplingConfig, SignalAnalysisSignal, TableAction, TableColumnKey,
        TableModelSpec,
    },
    transaction_container::{TransactionRef, TransactionStreamRef},
    wave_container::{ScopeRef, ScopeRefExt, VariableRef, VariableRefExt},
    wave_data::ScopeType,
    wave_source::{LoadIntent, WaveFormat},
};

fn fixture(path: &str) -> camino::Utf8PathBuf {
    get_project_root().unwrap().join(path).try_into().unwrap()
}

fn wait_for_source_count(state: &mut SystemState, expected_count: usize) {
    let load_start = std::time::Instant::now();
    while state
        .user
        .waves
        .as_ref()
        .is_none_or(|waves| waves.source_count() != expected_count)
    {
        state.handle_async_messages();
        state.handle_batch_commands();
        if load_start.elapsed().as_secs() > 10 {
            panic!("Timeout waiting for {expected_count} sources");
        }
    }
}

fn wait_until(
    state: &mut SystemState,
    label: &str,
    mut condition: impl FnMut(&SystemState) -> bool,
) {
    let load_start = std::time::Instant::now();
    while !condition(state) {
        state.handle_async_messages();
        state.handle_batch_commands();
        if load_start.elapsed().as_secs() > 10 {
            panic!("Timeout waiting for {label}");
        }
    }
}

#[test]
fn additive_ftr_source_can_add_generator_row() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _guard = runtime.enter();

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(fixture("examples/my_db.ftr"))),
            startup_commands: vec![],
            ..Default::default()
        });

    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);
    state.update(Message::LoadFileWithIntent(
        fixture("examples/my_db.ftr"),
        LoadIntent::AddSource,
    ));
    state.handle_async_messages();
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

    let waves = state.user.waves.as_ref().expect("waves loaded");
    assert_eq!(waves.source_count(), 2);
    let transactions = waves
        .transactions_for_source(SourceId(1))
        .expect("added FTR source should be a transaction container");
    assert!(
        transactions.get_generator(GeneratorId(4)).is_some(),
        "expected generator 4 in added FTR source; available generators: {:?}",
        transactions
            .get_generators()
            .iter()
            .map(|generator| (generator.id, generator.name.as_str()))
            .collect::<Vec<_>>()
    );

    state.update(Message::AddStreamOrGeneratorFromSource(
        SourceId(1),
        TransactionStreamRef::new_gen(
            StreamId(1),
            GeneratorId(4),
            "pipelined_stream.read".to_string(),
        ),
    ));

    let waves = state.user.waves.as_ref().expect("waves loaded");
    assert!(
        waves.displayed_items.values().any(
            |item| matches!(item, DisplayedItem::Stream(stream) if stream.source == SourceId(1))
        ),
        "expected a displayed stream from the added FTR source"
    );
}

#[test]
fn startup_additional_ftr_source_is_loaded_additively() {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(fixture("examples/my_db.ftr"))),
            additional_waves: vec![WaveSource::File(fixture("examples/my_db.ftr"))],
            startup_commands: vec![],
            ..Default::default()
        });

    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

    let waves = state.user.waves.as_ref().expect("waves loaded");
    assert_eq!(waves.source_count(), 2);
    assert!(waves.transactions_for_source(SourceId(1)).is_some());
}

#[test]
fn ordered_multi_file_load_keeps_first_file_as_primary_source() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let _guard = runtime.enter();

    let mut state = SystemState::new_default_config().unwrap();
    let primary = fixture("examples/fused_ftr_wave.vcd");
    state.update(Message::LoadFilesWithIntents(vec![
        (primary.clone(), LoadIntent::ReplaceSession),
        (fixture("examples/my_db.ftr"), LoadIntent::AddSource),
    ]));
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

    let waves = state.user.waves.as_ref().expect("waves loaded");
    assert_eq!(waves.source_count(), 2);
    assert_eq!(waves.source, WaveSource::File(primary));
    assert!(waves.waves_for_source(SourceId::default()).is_some());
    assert!(waves.transactions_for_source(SourceId(1)).is_some());
}

#[test]
fn matching_timescale_sources_can_have_different_spans() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let _guard = runtime.enter();

    let mut state = SystemState::new_default_config().unwrap();
    state.update(Message::LoadFilesWithIntents(vec![
        (
            fixture("examples/fused_ftr_wave_long.vcd"),
            LoadIntent::ReplaceSession,
        ),
        (fixture("examples/my_db.ftr"), LoadIntent::AddSource),
    ]));
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

    let waves = state.user.waves.as_ref().expect("waves loaded");
    assert_eq!(waves.source_count(), 2);
    assert!(waves.waves_for_source(SourceId::default()).is_some());
    assert!(waves.transactions_for_source(SourceId(1)).is_some());
    assert_eq!(waves.safe_num_timestamps(), BigInt::from(6_000_000));
    let ftr_domain = waves.time_domain_for_source(SourceId(1)).unwrap();
    let session_domain = waves.sources.common_time_domain().unwrap();
    assert!(ftr_domain.max_timestamp < session_domain.max_timestamp.clone());
}

#[test]
fn source_rename_updates_primary_and_additive_labels() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let _guard = runtime.enter();

    let mut state = SystemState::new_default_config().unwrap();
    state.update(Message::LoadFilesWithIntents(vec![
        (
            fixture("examples/fused_ftr_wave.vcd"),
            LoadIntent::ReplaceSession,
        ),
        (fixture("examples/my_db.ftr"), LoadIntent::AddSource),
    ]));
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

    state.update(Message::RenameSource(
        SourceId::default(),
        "waves".to_string(),
    ));
    state.update(Message::RenameSource(
        SourceId(1),
        "transactions".to_string(),
    ));

    let waves = state.user.waves.as_ref().expect("waves loaded");
    assert_eq!(
        waves.source_label_for(SourceId::default()).as_deref(),
        Some("waves")
    );
    assert_eq!(
        waves.source_label_for(SourceId(1)).as_deref(),
        Some("transactions")
    );
}

#[test]
fn canvas_zoom_fit_ignores_loaded_sources_without_displayed_rows() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let _guard = runtime.enter();

    let mut state = SystemState::new_default_config().unwrap();
    state.update(Message::LoadFilesWithIntents(vec![
        (
            fixture("examples/fused_ftr_wave.vcd"),
            LoadIntent::ReplaceSession,
        ),
        (fixture("examples/my_db.ftr"), LoadIntent::AddSource),
        (
            fixture("examples/fused_ftr_wave_long.vcd"),
            LoadIntent::AddSource,
        ),
    ]));
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
    ]));
    state.update(Message::AddStreamOrGeneratorFromSource(
        SourceId(1),
        TransactionStreamRef::new_gen(
            StreamId(1),
            GeneratorId(4),
            "pipelined_stream.read".to_string(),
        ),
    ));
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

    {
        let waves = state.user.waves.as_ref().expect("waves loaded");
        assert_eq!(waves.safe_num_timestamps(), BigInt::from(6_000_000));
        assert_eq!(waves.safe_canvas_num_timestamps(), BigInt::from(3_400_000));
    }

    state.update(Message::ZoomToFit { viewport_idx: 0 });
    {
        let waves = state.user.waves.as_ref().expect("waves loaded");
        assert_eq!(
            waves.viewports[0].right_edge_time(&waves.safe_canvas_num_timestamps()),
            BigInt::from(3_400_000)
        );
    }

    state.update(Message::AddVariablesFromSource(
        SourceId(2),
        vec![VariableRef::from_hierarchy_string("tb.counter")],
    ));
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

    let waves = state.user.waves.as_ref().expect("waves loaded");
    assert_eq!(waves.safe_canvas_num_timestamps(), BigInt::from(6_000_000));
}

#[test]
fn source_load_request_tokens_reject_stale_additive_source_responses() {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(fixture("examples/my_db.ftr"))),
            startup_commands: vec![],
            ..Default::default()
        });

    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

    let old_request = LoadRequestId(10);
    let new_request = LoadRequestId(11);
    let source_id = state
        .user
        .waves
        .as_mut()
        .expect("waves loaded")
        .add_pending_source(
            WaveSource::File(fixture("examples/fused_ftr_wave.vcd")),
            WaveFormat::Vcd,
            DataContainer::Empty,
            old_request,
        );

    assert!(state.source_load_request_is_current(source_id, old_request));
    let source = state
        .user
        .waves
        .as_ref()
        .and_then(|waves| waves.sources.source(source_id))
        .expect("pending source");
    assert_eq!(source.active_load_request, Some(old_request));
    assert!(matches!(source.load_state, SourceLoadState::Pending));

    state
        .user
        .waves
        .as_mut()
        .expect("waves loaded")
        .mark_source_load_request(source_id, new_request);

    assert!(!state.source_load_request_is_current(source_id, old_request));
    assert!(state.source_load_request_is_current(source_id, new_request));
}

#[test]
fn failed_additive_wave_header_removes_pending_source() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let _guard = runtime.enter();

    let tempdir = tempdir().expect("tempdir");
    let invalid_wave = camino::Utf8PathBuf::from_path_buf(tempdir.path().join("invalid.vcd"))
        .expect("utf8 temp path");
    std::fs::write(invalid_wave.as_std_path(), "not a waveform").expect("write invalid wave");

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(fixture("examples/my_db.ftr"))),
            startup_commands: vec![],
            ..Default::default()
        });

    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);
    state.update(Message::LoadFileWithIntent(
        invalid_wave,
        LoadIntent::AddSource,
    ));

    wait_until(&mut state, "failed additive wave header", |state| {
        state.user.show_logs
            && state
                .user
                .waves
                .as_ref()
                .is_some_and(|waves| waves.source_count() == 1)
    });

    let waves = state.user.waves.as_ref().expect("waves loaded");
    assert_eq!(waves.source_count(), 1);
    assert!(waves.transactions_for_source(SourceId::default()).is_some());
}

#[test]
fn additive_wave_source_can_add_variable_row() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _guard = runtime.enter();
    std::thread::spawn(move || {
        runtime.block_on(async {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
            }
        });
    });

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(fixture("examples/fused_ftr_wave.vcd"))),
            startup_commands: vec![],
            ..Default::default()
        });

    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);
    state.update(Message::LoadFileWithIntent(
        fixture("examples/fused_ftr_wave.vcd"),
        LoadIntent::AddSource,
    ));
    wait_for_source_count(&mut state, 2);
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

    let waves = state.user.waves.as_ref().expect("waves loaded");
    assert_eq!(waves.source_count(), 2);
    assert!(
        waves.waves_for_source(SourceId(1)).is_some(),
        "added VCD source should be a waveform container"
    );

    state.update(Message::SetActiveScopeFromSource(
        SourceId(1),
        Some(ScopeType::WaveScope(ScopeRef::from_strs(&["tb"]))),
    ));
    let waves = state.user.waves.as_ref().expect("waves loaded");
    assert_eq!(waves.active_scope_source, SourceId(1));

    state.update(Message::AddVariablesFromSource(
        SourceId(1),
        vec![VariableRef::from_hierarchy_string("tb.clk")],
    ));
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

    let waves = state.user.waves.as_ref().expect("waves loaded");
    assert!(
        waves.displayed_items.values().any(
            |item| matches!(item, DisplayedItem::Variable(variable) if variable.source == SourceId(1))
        ),
        "expected a displayed variable from the added waveform source"
    );
}

#[test]
fn additive_ftr_source_can_open_transaction_table() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _guard = runtime.enter();

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(fixture("examples/my_db.ftr"))),
            startup_commands: vec![],
            ..Default::default()
        });

    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);
    state.update(Message::LoadFileWithIntent(
        fixture("examples/my_db.ftr"),
        LoadIntent::AddSource,
    ));
    state.handle_async_messages();
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

    let generator = TransactionStreamRef::new_gen(
        StreamId(1),
        GeneratorId(4),
        "pipelined_stream.read".to_string(),
    );
    state.update(Message::OpenTransactionTable {
        source: SourceId(1),
        generator,
    });

    let spec = state
        .user
        .table_tiles
        .values()
        .next()
        .expect("transaction table tile")
        .spec
        .clone();
    let TableModelSpec::TransactionTrace { source, generator } = &spec else {
        panic!("expected transaction trace table");
    };
    assert_eq!(*source, SourceId(1));
    assert_eq!(generator.gen_id, Some(GeneratorId(4)));

    let ctx = state.table_model_context();
    let model = spec.create_model(&ctx).expect("table model");
    assert!(model.row_count() > 0);
    let row = model.row_id_at(0).expect("first row");
    let TableAction::FocusTransaction(source, tx_ref) = model.on_activate(row) else {
        panic!("expected focus transaction action");
    };
    assert_eq!(source, SourceId(1));
    assert!(tx_ref.id.0 < usize::MAX);
}

#[test]
fn close_additive_source_removes_rows_tables_and_focus() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let _guard = runtime.enter();

    let mut state = SystemState::new_default_config().unwrap();
    state.update(Message::LoadFilesWithIntents(vec![
        (
            fixture("examples/fused_ftr_wave.vcd"),
            LoadIntent::ReplaceSession,
        ),
        (fixture("examples/my_db.ftr"), LoadIntent::AddSource),
    ]));
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

    let generator = TransactionStreamRef::new_gen(
        StreamId(1),
        GeneratorId(4),
        "pipelined_stream.read".to_string(),
    );
    state.update(Message::AddStreamOrGeneratorFromSource(
        SourceId(1),
        generator.clone(),
    ));
    state.update(Message::OpenTransactionTable {
        source: SourceId(1),
        generator,
    });
    state.update(Message::SetActiveScopeFromSource(SourceId(1), None));
    state.update(Message::FocusTransactionFromSource(
        Some(SourceTransactionRef::new(
            SourceId(1),
            TransactionRef {
                id: TransactionId(4),
            },
        )),
        None,
    ));

    assert_eq!(state.user.table_tiles.len(), 1);
    assert!(
        state.user.tile_tree.tree.tiles.iter().any(|(_, tile)| {
            matches!(
                tile,
                egui_tiles::Tile::Pane(crate::tiles::SurferPane::Table(_))
            )
        }),
        "expected a table pane before closing the source"
    );

    state.update(Message::CloseSource(SourceId(1)));

    let waves = state.user.waves.as_ref().expect("waves loaded");
    assert_eq!(waves.source_count(), 1);
    assert_eq!(waves.active_scope_source, SourceId::default());
    assert!(waves.focused_transaction.0.is_none());
    assert!(
        waves.displayed_items.values().all(
            |item| !matches!(item, DisplayedItem::Stream(stream) if stream.source == SourceId(1))
        ),
        "closing a source should remove its displayed rows"
    );
    assert!(state.user.table_tiles.is_empty());
    assert!(
        !state.user.tile_tree.tree.tiles.iter().any(|(_, tile)| {
            matches!(
                tile,
                egui_tiles::Tile::Pane(crate::tiles::SurferPane::Table(_))
            )
        }),
        "closing a source should remove its table panes"
    );
}

#[test]
fn reload_additive_ftr_source_preserves_other_sources_and_rows() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let _guard = runtime.enter();

    let mut state = SystemState::new_default_config().unwrap();
    state.update(Message::LoadFilesWithIntents(vec![
        (
            fixture("examples/fused_ftr_wave.vcd"),
            LoadIntent::ReplaceSession,
        ),
        (fixture("examples/my_db.ftr"), LoadIntent::AddSource),
    ]));
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

    state.update(Message::AddVariablesFromSource(
        SourceId::default(),
        vec![VariableRef::from_hierarchy_string("tb.clk")],
    ));
    let generator = TransactionStreamRef::new_gen(
        StreamId(1),
        GeneratorId(4),
        "pipelined_stream.read".to_string(),
    );
    state.update(Message::AddStreamOrGeneratorFromSource(
        SourceId(1),
        generator.clone(),
    ));
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

    let primary_signal_spec = TableModelSpec::SignalChangeList {
        source: SourceId::default(),
        variable: VariableRef::from_hierarchy_string("tb.clk"),
        field: vec![],
    };
    let ftr_table_spec = TableModelSpec::TransactionTrace {
        source: SourceId(1),
        generator,
    };
    let primary_generation_before =
        primary_signal_spec.cache_generation(&state.table_model_context());
    let ftr_generation_before = ftr_table_spec.cache_generation(&state.table_model_context());
    let old_generation = state
        .user
        .waves
        .as_ref()
        .and_then(|waves| waves.sources.source(SourceId(1)))
        .map(|source| source.cache_generation)
        .expect("additive source generation");

    state.update(Message::ReloadSource(SourceId(1), true));
    wait_until(&mut state, "additive FTR reload", |state| {
        state
            .user
            .waves
            .as_ref()
            .and_then(|waves| waves.sources.source(SourceId(1)))
            .is_some_and(|source| source.cache_generation > old_generation)
    });

    let waves = state.user.waves.as_ref().expect("waves loaded");
    assert_eq!(
        primary_signal_spec.cache_generation(&state.table_model_context()),
        primary_generation_before,
        "reloading an additive FTR source should not invalidate primary-source signal tables"
    );
    assert!(
        ftr_table_spec.cache_generation(&state.table_model_context()) > ftr_generation_before,
        "reloading an additive FTR source should invalidate tables tied to that source"
    );
    assert_eq!(waves.source_count(), 2);
    assert!(waves.waves_for_source(SourceId::default()).is_some());
    assert!(waves.transactions_for_source(SourceId(1)).is_some());
    assert!(
        waves.displayed_items.values().any(
            |item| matches!(item, DisplayedItem::Variable(variable) if variable.source == SourceId::default())
        ),
        "primary waveform row should survive additive FTR reload"
    );
    assert!(
        waves.displayed_items.values().any(
            |item| matches!(item, DisplayedItem::Stream(stream) if stream.source == SourceId(1))
        ),
        "FTR stream row should stay attached to the reloaded source"
    );
}

#[test]
fn reload_additive_wave_source_rejects_mismatched_time_domain_without_replacing_old_source() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let _guard = runtime.enter();

    let tempdir = tempdir().expect("tempdir");
    let reload_path = camino::Utf8PathBuf::from_path_buf(tempdir.path().join("reload.vcd"))
        .expect("utf8 temp path");
    std::fs::copy(
        fixture("examples/fused_ftr_wave.vcd").as_std_path(),
        reload_path.as_std_path(),
    )
    .expect("seed reload fixture");

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(fixture("examples/my_db.ftr"))),
            startup_commands: vec![],
            ..Default::default()
        });

    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);
    state.update(Message::LoadFileWithIntent(
        reload_path.clone(),
        LoadIntent::AddSource,
    ));
    wait_for_source_count(&mut state, 2);
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);
    state.update(Message::AddVariablesFromSource(
        SourceId(1),
        vec![VariableRef::from_hierarchy_string("tb.clk")],
    ));
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

    let (old_generation, old_domain) = {
        let waves = state.user.waves.as_ref().expect("waves loaded");
        let source = waves.sources.source(SourceId(1)).expect("additive source");
        (source.cache_generation, source.time_domain.clone())
    };

    std::fs::copy(
        fixture("examples/counter.vcd").as_std_path(),
        reload_path.as_std_path(),
    )
    .expect("replace reload fixture with mismatched domain");

    state.update(Message::ReloadSource(SourceId(1), true));
    wait_until(&mut state, "reload mismatch error", |state| {
        state.user.show_logs
    });

    let waves = state.user.waves.as_ref().expect("waves loaded");
    let source = waves.sources.source(SourceId(1)).expect("additive source");
    assert_eq!(source.cache_generation, old_generation);
    assert_eq!(source.time_domain, old_domain);
    assert_eq!(waves.source_count(), 2);
    assert!(
        waves.displayed_items.values().any(
            |item| matches!(item, DisplayedItem::Variable(variable) if variable.source == SourceId(1))
        ),
        "rejected reload should keep rows from the old source"
    );
}

#[test]
fn additive_wave_source_can_open_source_qualified_signal_tables() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let _guard = runtime.enter();

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(fixture("examples/my_db.ftr"))),
            startup_commands: vec![],
            ..Default::default()
        });

    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);
    state.update(Message::LoadFileWithIntent(
        fixture("examples/fused_ftr_wave.vcd"),
        LoadIntent::AddSource,
    ));
    wait_for_source_count(&mut state, 2);
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

    state.update(Message::AddVariablesFromSource(
        SourceId(1),
        vec![
            VariableRef::from_hierarchy_string("tb.clk"),
            VariableRef::from_hierarchy_string("tb.counter"),
        ],
    ));
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

    let waves = state.user.waves.as_ref().expect("waves loaded");
    assert!(waves.transactions_for_source(SourceId::default()).is_some());
    assert!(waves.waves_for_source(SourceId(1)).is_some());

    let ctx = state.table_model_context();
    let signal_spec = TableModelSpec::SignalChangeList {
        source: SourceId(1),
        variable: VariableRef::from_hierarchy_string("tb.clk"),
        field: vec![],
    };
    let signal_model = signal_spec.create_model(&ctx).expect("signal table model");
    assert!(signal_model.row_count() > 0);

    let primary_signal_spec = TableModelSpec::SignalChangeList {
        source: SourceId::default(),
        variable: VariableRef::from_hierarchy_string("tb.clk"),
        field: vec![],
    };
    assert!(
        primary_signal_spec.create_model(&ctx).is_err(),
        "primary source is an FTR trace, so signal tables must not fall back to it"
    );

    let multi_spec = TableModelSpec::MultiSignalChangeList {
        variables: vec![
            MultiSignalEntry {
                source: SourceId(1),
                variable: VariableRef::from_hierarchy_string("tb.clk"),
                field: vec![],
            },
            MultiSignalEntry {
                source: SourceId(1),
                variable: VariableRef::from_hierarchy_string("tb.counter"),
                field: vec![],
            },
        ],
    };
    let multi_model = multi_spec.create_model(&ctx).expect("multi-signal model");
    assert!(multi_model.row_count() > 0);

    let schema = multi_model.schema();
    assert_eq!(schema.columns.len(), 3);
    for column in &schema.columns[1..] {
        let TableColumnKey::Str(key) = &column.key else {
            panic!("signal column key should be string-backed");
        };
        let (source, _, _) = decode_signal_column_key(key).expect("signal column key");
        assert_eq!(source, SourceId(1));
    }
}

#[test]
fn additive_wave_source_can_run_source_qualified_signal_analysis() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let _guard = runtime.enter();

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(fixture("examples/my_db.ftr"))),
            startup_commands: vec![],
            ..Default::default()
        });

    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);
    state.update(Message::LoadFileWithIntent(
        fixture("examples/fused_ftr_wave.vcd"),
        LoadIntent::AddSource,
    ));
    wait_for_source_count(&mut state, 2);
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

    state.update(Message::AddVariablesFromSource(
        SourceId(1),
        vec![
            VariableRef::from_hierarchy_string("tb.clk"),
            VariableRef::from_hierarchy_string("tb.counter"),
        ],
    ));
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

    let spec = TableModelSpec::AnalysisResults {
        kind: AnalysisKind::SignalAnalysisV1,
        params: AnalysisParams::SignalAnalysisV1 {
            config: SignalAnalysisConfig {
                sampling: SignalAnalysisSamplingConfig {
                    source: SourceId(1),
                    signal: VariableRef::from_hierarchy_string("tb.clk"),
                },
                signals: vec![SignalAnalysisSignal {
                    source: SourceId(1),
                    variable: VariableRef::from_hierarchy_string("tb.counter"),
                    field: vec![],
                    translator: "Unsigned".to_string(),
                }],
                run_revision: 0,
            },
        },
    };

    let ctx = state.table_model_context();
    let model = spec
        .create_model(&ctx)
        .expect("source-qualified analysis model");
    assert!(model.row_count() > 0);
}

#[test]
fn mixed_source_state_file_reloads_sources_and_restores_rows_and_tables() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let _guard = runtime.enter();

    let mut state = SystemState::new_default_config().unwrap();
    state.update(Message::LoadFilesWithIntents(vec![
        (
            fixture("examples/fused_ftr_wave.vcd"),
            LoadIntent::ReplaceSession,
        ),
        (fixture("examples/my_db.ftr"), LoadIntent::AddSource),
    ]));
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

    state.update(Message::AddVariablesFromSource(
        SourceId::default(),
        vec![VariableRef::from_hierarchy_string("tb.clk")],
    ));
    let generator = TransactionStreamRef::new_gen(
        StreamId(1),
        GeneratorId(4),
        "pipelined_stream.read".to_string(),
    );
    state.update(Message::AddStreamOrGeneratorFromSource(
        SourceId(1),
        generator.clone(),
    ));
    state.update(Message::OpenTransactionTable {
        source: SourceId(1),
        generator,
    });
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

    let encoded = state.encode_state().expect("encoded state");
    let loaded_state = ron::from_str(&encoded).expect("decoded state");
    let mut restored = SystemState::new_default_config().unwrap();
    restored.update(Message::LoadState(Box::new(loaded_state), None));
    wait_until(&mut restored, "mixed source state restore", |state| {
        state.user.pending_state_restore.is_none()
            && state
                .user
                .waves
                .as_ref()
                .is_some_and(|waves| waves.source_count() == 2)
    });
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut restored, 10);

    let waves = restored.user.waves.as_ref().expect("restored waves");
    assert!(waves.waves_for_source(SourceId::default()).is_some());
    assert!(waves.transactions_for_source(SourceId(1)).is_some());
    assert!(
        waves.displayed_items.values().any(
            |item| matches!(item, DisplayedItem::Variable(variable) if variable.source == SourceId::default())
        ),
        "restored state should keep the waveform row"
    );
    assert!(
        waves.displayed_items.values().any(
            |item| matches!(item, DisplayedItem::Stream(stream) if stream.source == SourceId(1))
        ),
        "restored state should keep the FTR row"
    );

    let spec = restored
        .user
        .table_tiles
        .values()
        .next()
        .expect("restored transaction table")
        .spec
        .clone();
    let TableModelSpec::TransactionTrace { source, .. } = spec else {
        panic!("expected restored transaction table");
    };
    assert_eq!(source, SourceId(1));

    let ctx = restored.table_model_context();
    let model = restored
        .user
        .table_tiles
        .values()
        .next()
        .expect("restored transaction table")
        .spec
        .create_model(&ctx)
        .expect("restored transaction table model");
    assert!(model.row_count() > 0);
}
