use super::support::*;
use super::*;

#[test]
fn signal_change_list_model_basic_rows() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let state = load_counter_state_with_variable("tb.clk");
    let ctx = state.table_model_context();
    let spec = TableModelSpec::SignalChangeList {
        variable: VariableRef::from_hierarchy_string("tb.clk"),
        field: vec![],
    };
    let model = spec.create_model(&ctx).expect("model");

    assert!(model.row_count() > 0);
    let row_id = model.row_id_at(0).expect("row");
    let time_text = match model.cell(row_id, 0) {
        TableCell::Text(text) => text,
        TableCell::RichText(text) => text.text().to_string(),
    };
    let value_text = match model.cell(row_id, 1) {
        TableCell::Text(text) => text,
        TableCell::RichText(text) => text.text().to_string(),
    };
    assert!(!time_text.is_empty());
    assert!(!value_text.is_empty());
    assert!(matches!(
        model.sort_key(row_id, 0),
        TableSortKey::Numeric(_)
    ));
    assert!(matches!(
        model.sort_key(row_id, 1),
        TableSortKey::Numeric(_) | TableSortKey::Text(_)
    ));

    let search = model.search_text(row_id);
    assert!(search.contains(&time_text));
    assert!(search.contains(&value_text));

    let time = match model.on_activate(row_id) {
        TableAction::CursorSet(time) => time,
        _ => panic!("expected cursor set"),
    };
    let waves = state.user.waves.as_ref().expect("waves");
    let formatter = crate::time::TimeFormatter::new(
        &waves.inner.as_waves().unwrap().metadata().timescale,
        &state.user.wanted_timeunit,
        &state.get_time_format(),
    );
    assert_eq!(formatter.format(&time), time_text);
}

#[test]
fn signal_change_list_model_missing_field_path_uses_em_dash() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let state = load_counter_state_with_variable("tb.dut.counter");
    let ctx = state.table_model_context();
    let spec = TableModelSpec::SignalChangeList {
        variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
        field: vec!["missing".to_string()],
    };
    let model = spec.create_model(&ctx).expect("model");
    let row_id = model.row_id_at(0).expect("row");
    let value_text = match model.cell(row_id, 1) {
        TableCell::Text(text) => text,
        TableCell::RichText(text) => text.text().to_string(),
    };
    assert_eq!(value_text, "—");
}

#[test]
fn signal_change_list_model_errors() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let state = SystemState::new_default_config().expect("state");
    let ctx = state.table_model_context();
    let spec = TableModelSpec::SignalChangeList {
        variable: VariableRef::from_hierarchy_string("tb.clk"),
        field: vec![],
    };
    assert!(matches!(
        spec.create_model(&ctx),
        Err(TableCacheError::DataUnavailable)
    ));

    let state = load_counter_state();
    let ctx = state.table_model_context();
    let missing_spec = TableModelSpec::SignalChangeList {
        variable: VariableRef::from_hierarchy_string("tb.nope"),
        field: vec![],
    };
    assert!(matches!(
        missing_spec.create_model(&ctx),
        Err(TableCacheError::ModelNotFound { .. })
    ));

    let state = load_counter_state();
    let ctx = state.table_model_context();
    assert!(matches!(
        spec.create_model(&ctx),
        Err(TableCacheError::DataUnavailable)
    ));
}

#[test]
fn signal_analysis_model_requires_loaded_signals() {
    let empty_state = SystemState::new_default_config().expect("state");
    let empty_ctx = empty_state.table_model_context();
    let spec = TableModelSpec::AnalysisResults {
        kind: AnalysisKind::SignalAnalysisV1,
        params: AnalysisParams::SignalAnalysisV1 {
            config: SignalAnalysisConfig {
                sampling: SignalAnalysisSamplingConfig {
                    signal: VariableRef::from_hierarchy_string("tb.clk"),
                },
                signals: vec![SignalAnalysisSignal {
                    variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
                    field: vec![],
                    translator: "Unsigned".to_string(),
                }],
                run_revision: 0,
            },
        },
    };
    assert!(matches!(
        spec.create_model(&empty_ctx),
        Err(TableCacheError::DataUnavailable)
    ));

    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let state = load_counter_state_with_variable("tb.clk");
    let ctx = state.table_model_context();
    assert!(matches!(
        spec.create_model(&ctx),
        Err(TableCacheError::DataUnavailable)
    ));
}

#[test]
fn signal_analysis_model_schema_rows_sort_search_and_activation() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let state = load_counter_state_with_variables(&["tb.clk", "tb.dut.counter"]);
    let ctx = state.table_model_context();
    let spec = TableModelSpec::AnalysisResults {
        kind: AnalysisKind::SignalAnalysisV1,
        params: AnalysisParams::SignalAnalysisV1 {
            config: SignalAnalysisConfig {
                sampling: SignalAnalysisSamplingConfig {
                    signal: VariableRef::from_hierarchy_string("tb.clk"),
                },
                signals: vec![SignalAnalysisSignal {
                    variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
                    field: vec![],
                    translator: "Unsigned".to_string(),
                }],
                run_revision: 0,
            },
        },
    };

    let model = spec.create_model(&ctx).expect("model");
    let schema = model.schema();
    assert_eq!(schema.columns.len(), 6);
    assert_eq!(schema.columns[0].label, "Interval End");
    assert_eq!(schema.columns[1].label, "Info");
    assert_eq!(
        schema.columns[2].key,
        TableColumnKey::Str("signal_analysis:v1:0:avg".to_string())
    );
    assert_eq!(schema.columns[2].label, "tb.dut.counter.avg");

    // No markers => single global row.
    assert_eq!(model.row_count(), 1);
    let row_id = model.row_id_at(0).expect("row");

    let end_text = match model.cell(row_id, 0) {
        TableCell::Text(text) => text,
        TableCell::RichText(text) => text.text().to_string(),
    };
    assert!(!end_text.is_empty());

    let info_text = match model.cell(row_id, 1) {
        TableCell::Text(text) => text,
        TableCell::RichText(text) => text.text().to_string(),
    };
    assert_eq!(info_text, "GLOBAL");

    assert!(matches!(
        model.sort_key(row_id, 0),
        TableSortKey::Numeric(_)
    ));
    assert!(matches!(
        model.sort_key(row_id, 2),
        TableSortKey::Numeric(_) | TableSortKey::None
    ));

    let search_text = model.search_text(row_id);
    assert!(search_text.contains("GLOBAL"));
    assert!(search_text.contains(&end_text));

    let activated_time = match model.on_activate(row_id) {
        TableAction::CursorSet(time) => time,
        _ => panic!("expected cursor set"),
    };
    assert_eq!(
        activated_time,
        state
            .user
            .waves
            .as_ref()
            .expect("waves")
            .num_timestamps()
            .expect("end time")
    );
}

#[test]
fn signal_analysis_model_intervals_and_activation_use_interval_end() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state_with_variables(&["tb.clk", "tb.dut.counter"]);
    state
        .user
        .waves
        .as_mut()
        .expect("waves")
        .markers
        .insert(1, num::BigInt::from(5u64));

    let ctx = state.table_model_context();
    let spec = TableModelSpec::AnalysisResults {
        kind: AnalysisKind::SignalAnalysisV1,
        params: AnalysisParams::SignalAnalysisV1 {
            config: SignalAnalysisConfig {
                sampling: SignalAnalysisSamplingConfig {
                    signal: VariableRef::from_hierarchy_string("tb.clk"),
                },
                signals: vec![SignalAnalysisSignal {
                    variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
                    field: vec![],
                    translator: "Unsigned".to_string(),
                }],
                run_revision: 0,
            },
        },
    };

    let model = spec.create_model(&ctx).expect("model");
    assert_eq!(model.row_count(), 3);

    let first_row = model.row_id_at(0).expect("first row");
    let first_info = match model.cell(first_row, 1) {
        TableCell::Text(text) => text,
        TableCell::RichText(text) => text.text().to_string(),
    };
    assert_eq!(first_info, "start -> Marker 1");

    match model.on_activate(first_row) {
        TableAction::CursorSet(time) => assert_eq!(time, num::BigInt::from(5u64)),
        _ => panic!("expected cursor set"),
    }
}

#[test]
fn signal_analysis_model_non_empty_field_disables_numeric_metrics() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let state = load_counter_state_with_variables(&["tb.clk", "tb.dut.counter"]);
    let ctx = state.table_model_context();
    let spec = TableModelSpec::AnalysisResults {
        kind: AnalysisKind::SignalAnalysisV1,
        params: AnalysisParams::SignalAnalysisV1 {
            config: SignalAnalysisConfig {
                sampling: SignalAnalysisSamplingConfig {
                    signal: VariableRef::from_hierarchy_string("tb.clk"),
                },
                signals: vec![SignalAnalysisSignal {
                    variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
                    field: vec!["value".to_string()],
                    translator: "Unsigned".to_string(),
                }],
                run_revision: 0,
            },
        },
    };

    let model = spec.create_model(&ctx).expect("model");
    let row_id = model.row_id_at(0).expect("row");
    let avg_text = match model.cell(row_id, 2) {
        TableCell::Text(text) => text,
        TableCell::RichText(text) => text.text().to_string(),
    };
    assert_eq!(avg_text, "—");
    assert_eq!(model.sort_key(row_id, 2), TableSortKey::None);
}

#[test]
fn analysis_results_default_view_config_sets_sort_and_activation() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let state = load_counter_state();
    let ctx = state.table_model_context();
    let spec = TableModelSpec::AnalysisResults {
        kind: AnalysisKind::SignalAnalysisV1,
        params: AnalysisParams::SignalAnalysisV1 {
            config: SignalAnalysisConfig {
                sampling: SignalAnalysisSamplingConfig {
                    signal: VariableRef::from_hierarchy_string("tb.clk"),
                },
                signals: vec![SignalAnalysisSignal {
                    variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
                    field: vec![],
                    translator: "Unsigned".to_string(),
                }],
                run_revision: 0,
            },
        },
    };

    let config = spec.default_view_config(&ctx);
    assert_eq!(config.title, "Signal Analysis: tb.clk (posedge)");
    assert_eq!(config.selection_mode, TableSelectionMode::Single);
    assert!(config.activate_on_select);
    assert_eq!(
        config.sort,
        vec![TableSortSpec {
            key: TableColumnKey::Str("interval_end".to_string()),
            direction: TableSortDirection::Ascending,
        }]
    );
}

#[test]
fn run_signal_analysis_creates_analysis_tile_and_preloads_signals() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    let config = SignalAnalysisConfig {
        sampling: SignalAnalysisSamplingConfig {
            signal: VariableRef::from_hierarchy_string("tb.clk"),
        },
        signals: vec![SignalAnalysisSignal {
            variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
            field: vec![],
            translator: "Unsigned".to_string(),
        }],
        run_revision: 3,
    };

    state.update(Message::RunSignalAnalysis {
        config: config.clone(),
    });

    assert_eq!(
        state.user.table_tiles.len(),
        1,
        "analysis tile should be created"
    );
    let (tile_id, tile_state) = state
        .user
        .table_tiles
        .iter()
        .next()
        .expect("analysis tile should exist");
    assert_eq!(tile_state.config.title, "Signal Analysis: tb.clk (posedge)");
    match &tile_state.spec {
        TableModelSpec::AnalysisResults {
            kind: AnalysisKind::SignalAnalysisV1,
            params:
                AnalysisParams::SignalAnalysisV1 {
                    config: spec_config,
                },
        } => {
            assert_eq!(spec_config, &config);
        }
        other => panic!("expected signal-analysis spec, got {other:?}"),
    }
    assert!(
        state.table_runtime.contains_key(tile_id),
        "run path should route through BuildTableCache"
    );

    wait_for_waves_fully_loaded(&mut state, 10);

    let waves = state.user.waves.as_ref().expect("waves");
    let wave_container = waves.inner.as_waves().expect("wave container");
    for variable in std::iter::once(&config.sampling.signal)
        .chain(config.signals.iter().map(|signal| &signal.variable))
    {
        let resolved_variable = wave_container
            .update_variable_ref(variable)
            .unwrap_or_else(|| variable.clone());
        let signal_id = wave_container
            .signal_id(&resolved_variable)
            .expect("signal id should resolve");
        assert!(
            wave_container.is_signal_loaded(&signal_id),
            "signal should be preloaded for analysis run: {}",
            variable.full_path_string()
        );
    }
}

#[test]
fn signal_analysis_table_tile_state_round_trips_through_user_state_ron() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    let config = SignalAnalysisConfig {
        sampling: SignalAnalysisSamplingConfig {
            signal: VariableRef::from_hierarchy_string("tb.clk"),
        },
        signals: vec![SignalAnalysisSignal {
            variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
            field: vec![],
            translator: "Unsigned".to_string(),
        }],
        run_revision: 9,
    };

    state.update(Message::RunSignalAnalysis { config });

    let tile_id = *state.user.table_tiles.keys().next().expect("analysis tile");
    let tile_state = state
        .user
        .table_tiles
        .get(&tile_id)
        .expect("analysis tile state");
    assert_eq!(tile_state.config.title, "Signal Analysis: tb.clk (posedge)");

    let encoded = ron::ser::to_string(&state.user).expect("serialize user state");
    let decoded: crate::state::UserState =
        ron::de::from_str(&encoded).expect("deserialize user state");

    let restored_tile_state = decoded
        .table_tiles
        .get(&tile_id)
        .expect("restored analysis tile");
    assert_eq!(restored_tile_state.spec, tile_state.spec);
    assert_eq!(restored_tile_state.config, tile_state.config);
}

#[test]
fn signal_analysis_build_table_cache_flow_completes_after_run() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    let config = SignalAnalysisConfig {
        sampling: SignalAnalysisSamplingConfig {
            signal: VariableRef::from_hierarchy_string("tb.clk"),
        },
        signals: vec![SignalAnalysisSignal {
            variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
            field: vec![],
            translator: "Unsigned".to_string(),
        }],
        run_revision: 4,
    };

    state.update(Message::RunSignalAnalysis { config });
    let tile_id = *state.user.table_tiles.keys().next().expect("tile");

    // Wait for signal preflight to finish before forcing a rebuild.
    wait_for_waves_fully_loaded(&mut state, 10);

    let tile_state = state.user.table_tiles.get(&tile_id).expect("tile state");
    let cache_key = TableCacheKey {
        model_key: tile_state.spec.model_key_for_tile(tile_id),
        display_filter: tile_state.config.display_filter.clone(),
        pinned_filters: tile_state.config.pinned_filters.clone(),
        view_sort: tile_state.config.sort.clone(),
        generation: state
            .user
            .waves
            .as_ref()
            .map_or(0, |waves| waves.cache_generation),
    };
    state.update(Message::BuildTableCache {
        tile_id,
        cache_key: cache_key.clone(),
    });

    assert!(
        state
            .table_runtime
            .get(&tile_id)
            .is_some_and(|runtime| runtime.model.is_none()),
        "signal-analysis model should be constructed asynchronously"
    );

    let build_start = std::time::Instant::now();
    loop {
        state.handle_async_messages();

        let is_ready = state
            .table_runtime
            .get(&tile_id)
            .and_then(|runtime| runtime.cache.as_ref())
            .is_some_and(|entry| entry.is_ready());
        if is_ready {
            break;
        }

        if build_start.elapsed().as_secs() > 10 {
            panic!("timed out waiting for signal-analysis table cache build");
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    let runtime = state.table_runtime.get(&tile_id).expect("runtime");
    assert!(runtime.last_error.is_none());
    assert!(runtime.model.is_some());
    assert_eq!(runtime.cache_key.as_ref(), Some(&cache_key));

    let cache_entry = runtime.cache.as_ref().expect("cache entry");
    assert!(cache_entry.is_ready());
    let cache = cache_entry.get().expect("ready cache");
    assert!(
        !cache.row_ids.is_empty(),
        "analysis cache should contain at least the global row"
    );
}

#[test]
fn signal_analysis_sort_reuses_cached_model() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    let config = SignalAnalysisConfig {
        sampling: SignalAnalysisSamplingConfig {
            signal: VariableRef::from_hierarchy_string("tb.clk"),
        },
        signals: vec![SignalAnalysisSignal {
            variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
            field: vec![],
            translator: "Unsigned".to_string(),
        }],
        run_revision: 1,
    };

    state.update(Message::RunSignalAnalysis { config });
    let tile_id = *state.user.table_tiles.keys().next().expect("tile");
    wait_for_waves_fully_loaded(&mut state, 10);

    let tile_state = state.user.table_tiles.get(&tile_id).expect("tile state");
    let initial_cache_key = TableCacheKey {
        model_key: tile_state.spec.model_key_for_tile(tile_id),
        display_filter: tile_state.config.display_filter.clone(),
        pinned_filters: tile_state.config.pinned_filters.clone(),
        view_sort: tile_state.config.sort.clone(),
        generation: state
            .user
            .waves
            .as_ref()
            .map_or(0, |waves| waves.cache_generation),
    };
    state.update(Message::BuildTableCache {
        tile_id,
        cache_key: initial_cache_key,
    });

    let initial_build_start = std::time::Instant::now();
    loop {
        state.handle_async_messages();

        let ready = state
            .table_runtime
            .get(&tile_id)
            .is_some_and(|runtime| runtime.model.is_some())
            && state
                .table_runtime
                .get(&tile_id)
                .and_then(|runtime| runtime.cache.as_ref())
                .is_some_and(|entry| entry.is_ready());
        if ready {
            break;
        }

        if initial_build_start.elapsed().as_secs() > 10 {
            panic!("timed out waiting for initial signal-analysis cache build");
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    let model_before = state
        .table_runtime
        .get(&tile_id)
        .and_then(|runtime| runtime.model.clone())
        .expect("initial model");

    state.update(Message::SetTableSort {
        tile_id,
        sort: vec![TableSortSpec {
            key: TableColumnKey::Str("interval_end".to_string()),
            direction: TableSortDirection::Descending,
        }],
    });

    let tile_state = state.user.table_tiles.get(&tile_id).expect("tile state");
    let sort_cache_key = TableCacheKey {
        model_key: tile_state.spec.model_key_for_tile(tile_id),
        display_filter: tile_state.config.display_filter.clone(),
        pinned_filters: tile_state.config.pinned_filters.clone(),
        view_sort: tile_state.config.sort.clone(),
        generation: state
            .user
            .waves
            .as_ref()
            .map_or(0, |waves| waves.cache_generation),
    };

    state.update(Message::BuildTableCache {
        tile_id,
        cache_key: sort_cache_key.clone(),
    });

    let model_after_request = state
        .table_runtime
        .get(&tile_id)
        .and_then(|runtime| runtime.model.clone())
        .expect("model after sort rebuild request");
    assert!(
        Arc::ptr_eq(&model_before, &model_after_request),
        "sort rebuild should reuse existing analysis model"
    );

    let sort_build_start = std::time::Instant::now();
    loop {
        state.handle_async_messages();

        let ready = state
            .table_runtime
            .get(&tile_id)
            .and_then(|runtime| runtime.cache.as_ref())
            .is_some_and(|entry| entry.is_ready() && entry.cache_key == sort_cache_key);
        if ready {
            break;
        }

        if sort_build_start.elapsed().as_secs() > 10 {
            panic!("timed out waiting for sorted signal-analysis cache build");
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    let model_after_sort = state
        .table_runtime
        .get(&tile_id)
        .and_then(|runtime| runtime.model.clone())
        .expect("model after sort rebuild");
    assert!(
        Arc::ptr_eq(&model_before, &model_after_sort),
        "sort rebuild completion should keep the same analysis model"
    );
}

#[test]
fn signal_analysis_model_key_changes_on_refresh_run_revision() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    let config = SignalAnalysisConfig {
        sampling: SignalAnalysisSamplingConfig {
            signal: VariableRef::from_hierarchy_string("tb.clk"),
        },
        signals: vec![SignalAnalysisSignal {
            variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
            field: vec![],
            translator: "Unsigned".to_string(),
        }],
        run_revision: 0,
    };

    state.update(Message::RunSignalAnalysis { config });
    let tile_id = *state.user.table_tiles.keys().next().expect("tile");
    let old_model_key = state.user.table_tiles[&tile_id]
        .spec
        .model_key_for_tile(tile_id);

    wait_for_waves_fully_loaded(&mut state, 10);
    state.update(Message::RefreshSignalAnalysis { tile_id });

    let tile_state = state.user.table_tiles.get(&tile_id).expect("tile state");
    let new_model_key = tile_state.spec.model_key_for_tile(tile_id);
    assert_ne!(old_model_key, new_model_key);

    match &tile_state.spec {
        TableModelSpec::AnalysisResults {
            kind: AnalysisKind::SignalAnalysisV1,
            params:
                AnalysisParams::SignalAnalysisV1 {
                    config: updated_config,
                },
        } => assert_eq!(updated_config.run_revision, 1),
        other => panic!("expected signal-analysis spec, got {other:?}"),
    }

    let runtime = state.table_runtime.get(&tile_id).expect("runtime");
    assert_eq!(
        runtime.cache_key.as_ref().map(|key| key.model_key),
        Some(new_model_key)
    );
}

#[test]
fn refresh_signal_analysis_rebuilds_with_current_markers() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    let config = SignalAnalysisConfig {
        sampling: SignalAnalysisSamplingConfig {
            signal: VariableRef::from_hierarchy_string("tb.clk"),
        },
        signals: vec![SignalAnalysisSignal {
            variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
            field: vec![],
            translator: "Unsigned".to_string(),
        }],
        run_revision: 0,
    };

    state.update(Message::RunSignalAnalysis { config });
    let tile_id = *state.user.table_tiles.keys().next().expect("tile");

    wait_for_waves_fully_loaded(&mut state, 10);
    let initial_model_key = state.user.table_tiles[&tile_id]
        .spec
        .model_key_for_tile(tile_id);
    trigger_table_cache_build_for_tile(&mut state, tile_id);
    wait_for_table_cache_ready(&mut state, tile_id, Some(initial_model_key));

    let initial_row_count = state
        .table_runtime
        .get(&tile_id)
        .and_then(|runtime| runtime.cache.as_ref())
        .and_then(|entry| entry.get())
        .map_or(0, |cache| cache.row_ids.len());
    assert_eq!(
        initial_row_count, 1,
        "expected global-only result before markers"
    );

    state.update(Message::SetMarker {
        id: 1,
        time: num::BigInt::from(5u64),
    });
    state.update(Message::RefreshSignalAnalysis { tile_id });

    let tile_state = state.user.table_tiles.get(&tile_id).expect("tile state");
    let refreshed_model_key = tile_state.spec.model_key_for_tile(tile_id);
    assert_ne!(initial_model_key, refreshed_model_key);
    match &tile_state.spec {
        TableModelSpec::AnalysisResults {
            kind: AnalysisKind::SignalAnalysisV1,
            params:
                AnalysisParams::SignalAnalysisV1 {
                    config: refreshed_config,
                },
        } => assert_eq!(refreshed_config.run_revision, 1),
        other => panic!("expected signal-analysis spec, got {other:?}"),
    }

    wait_for_waves_fully_loaded(&mut state, 10);
    trigger_table_cache_build_for_tile(&mut state, tile_id);
    wait_for_table_cache_ready(&mut state, tile_id, Some(refreshed_model_key));

    let refreshed_row_count = state
        .table_runtime
        .get(&tile_id)
        .and_then(|runtime| runtime.cache.as_ref())
        .and_then(|entry| entry.get())
        .map_or(0, |cache| cache.row_ids.len());
    assert_eq!(refreshed_row_count, 3);
}

#[test]
fn edit_signal_analysis_run_updates_existing_tile_and_bumps_revision() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    let config = SignalAnalysisConfig {
        sampling: SignalAnalysisSamplingConfig {
            signal: VariableRef::from_hierarchy_string("tb.clk"),
        },
        signals: vec![SignalAnalysisSignal {
            variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
            field: vec![],
            translator: "Unsigned".to_string(),
        }],
        run_revision: 6,
    };

    state.update(Message::RunSignalAnalysis { config });
    let tile_id = *state.user.table_tiles.keys().next().expect("tile");
    state.update(Message::EditSignalAnalysis { tile_id });

    assert_eq!(state.user.signal_analysis_wizard_edit_target, Some(tile_id));
    {
        let dialog = state
            .user
            .show_signal_analysis_wizard
            .as_mut()
            .expect("wizard should open");
        dialog.signals[0].translator = "Signed".to_string();
    }

    let run_config = state
        .user
        .show_signal_analysis_wizard
        .as_ref()
        .expect("wizard should be present")
        .to_config()
        .expect("wizard config");
    state.update(Message::RunSignalAnalysis { config: run_config });

    assert_eq!(
        state.user.table_tiles.len(),
        1,
        "should update existing tile"
    );
    assert!(
        state.user.show_signal_analysis_wizard.is_none(),
        "wizard should close after run"
    );
    assert!(
        state.user.signal_analysis_wizard_edit_target.is_none(),
        "edit target should clear after run"
    );

    let tile_state = state.user.table_tiles.get(&tile_id).expect("tile");
    match &tile_state.spec {
        TableModelSpec::AnalysisResults {
            kind: AnalysisKind::SignalAnalysisV1,
            params:
                AnalysisParams::SignalAnalysisV1 {
                    config: updated_config,
                },
        } => {
            assert_eq!(updated_config.run_revision, 7);
            assert_eq!(updated_config.signals[0].translator, "Signed");
        }
        other => panic!("expected signal-analysis spec, got {other:?}"),
    }
}

#[test]
fn stale_signal_analysis_result_does_not_evict_current_inflight_entry() {
    let mut state = SystemState::new_default_config().expect("state");
    let tile_id = TableTileId(321);
    let cache_key = TableCacheKey {
        model_key: TableModelKey(123),
        display_filter: TableSearchSpec::default(),
        pinned_filters: vec![],
        view_sort: vec![],
        generation: 0,
    };
    let current_entry = Arc::new(TableCacheEntry::new(cache_key.clone(), 0, 2));

    state.table_runtime.insert(
        tile_id,
        TableRuntimeState {
            cache_key: Some(cache_key.clone()),
            cache: Some(current_entry.clone()),
            last_error: None,
            selection: TableSelection::default(),

            type_search: TypeSearchState::default(),
            scroll_state: TableScrollState::default(),
            filter_draft: None,
            hidden_selection_count: 0,
            model: None,
            table_revision: 2,
            cancel_token: Arc::new(AtomicBool::new(false)),
        },
    );
    state
        .table_inflight
        .insert(cache_key.clone(), current_entry.clone());

    let stale_entry = Arc::new(TableCacheEntry::new(cache_key.clone(), 0, 1));
    state.update(Message::TableCacheBuilt {
        tile_id,
        revision: 1,
        entry: stale_entry.clone(),
        model: None,
        result: Ok(TableCache {
            row_ids: vec![TableRowId(9)],
            row_index: build_row_index(&[TableRowId(9)]),
            search_texts: Some(vec!["stale".to_string()]),
        }),
    });

    let inflight_entry = state
        .table_inflight
        .get(&cache_key)
        .expect("inflight entry");
    assert!(
        Arc::ptr_eq(inflight_entry, &current_entry),
        "stale completion must not evict current inflight entry"
    );
    assert!(
        !stale_entry.is_ready(),
        "stale cache payload should be ignored"
    );
}

#[test]
fn signal_analysis_menu_visibility_tracks_variable_selection() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    assert!(!state.has_signal_analysis_selection());

    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);
    state.update(Message::AddDivider(Some("---".to_string()), None));

    let clk_ref = VariableRef::from_hierarchy_string("tb.clk");
    let clk_idx = {
        let waves = state.user.waves.as_ref().expect("waves");
        find_visible_index_for_variable(waves, &clk_ref).expect("clk visible")
    };
    let divider_idx = {
        let waves = state.user.waves.as_ref().expect("waves");
        waves
            .items_tree
            .iter_visible()
            .enumerate()
            .find_map(|(idx, node)| {
                matches!(
                    waves.displayed_items.get(&node.item_ref),
                    Some(crate::displayed_item::DisplayedItem::Divider(_))
                )
                .then_some(VisibleItemIndex(idx))
            })
            .expect("divider visible")
    };

    state.update(Message::ItemSelectionClear);
    state.update(Message::SetItemSelected(divider_idx, true));
    assert!(
        !state.has_signal_analysis_selection(),
        "non-variable selection should hide analysis action"
    );

    state.update(Message::ItemSelectionClear);
    state.update(Message::SetItemSelected(clk_idx, true));
    assert!(
        state.has_signal_analysis_selection(),
        "variable selection should show analysis action"
    );
}

#[test]
fn open_signal_analysis_wizard_filters_non_variable_selection() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);
    state.update(Message::AddDivider(Some("---".to_string()), None));
    state.update(Message::ItemSelectAll);

    state.update(Message::OpenSignalAnalysisWizard);

    let dialog = state
        .user
        .show_signal_analysis_wizard
        .as_ref()
        .expect("wizard should open");
    assert_eq!(dialog.signals.len(), 1, "divider should be filtered out");
    assert_eq!(
        dialog.signals[0].variable,
        VariableRef::from_hierarchy_string("tb.clk")
    );
}

#[test]
fn open_signal_analysis_wizard_defaults_sampling_to_first_one_bit_signal() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state_with_variables(&["tb.dut.counter", "tb.clk"]);

    let (clk_idx, counter_idx) = {
        let waves = state.user.waves.as_ref().expect("waves");
        let clk_ref = VariableRef::from_hierarchy_string("tb.clk");
        let counter_ref = VariableRef::from_hierarchy_string("tb.dut.counter");
        (
            find_visible_index_for_variable(waves, &clk_ref).expect("clk visible"),
            find_visible_index_for_variable(waves, &counter_ref).expect("counter visible"),
        )
    };

    state.update(Message::ItemSelectionClear);
    state.update(Message::SetItemSelected(counter_idx, true));
    state.update(Message::SetItemSelected(clk_idx, true));
    state.update(Message::OpenSignalAnalysisWizard);

    let dialog = state
        .user
        .show_signal_analysis_wizard
        .as_ref()
        .expect("wizard should open");
    let clk_ref = VariableRef::from_hierarchy_string("tb.clk");
    assert_eq!(dialog.sampling_signal, clk_ref);
    assert_eq!(
        state.signal_analysis_sampling_mode(&dialog.sampling_signal),
        Some(SignalAnalysisSamplingMode::PosEdge)
    );
}

#[test]
fn open_signal_analysis_wizard_requires_selected_variables() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state_with_variable("tb.clk");

    state.update(Message::ItemSelectionClear);
    state.update(Message::OpenSignalAnalysisWizard);

    assert!(
        state.user.show_signal_analysis_wizard.is_none(),
        "wizard should not open without selected variables"
    );
}

#[test]
fn open_signal_change_list_adds_table_tile_for_focused_item() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state_with_variable("tb.clk");
    let variable = VariableRef::from_hierarchy_string("tb.clk");
    let waves = state.user.waves.as_ref().expect("waves");
    let vidx = find_visible_index_for_variable(waves, &variable).expect("visible");
    state.update(Message::FocusItem(vidx));
    state.update(Message::OpenSignalChangeList {
        target: MessageTarget::CurrentSelection,
    });

    assert_eq!(state.user.table_tiles.len(), 1);
    let tile_state = state.user.table_tiles.values().next().expect("tile state");
    match &tile_state.spec {
        TableModelSpec::SignalChangeList { variable, field } => {
            assert_eq!(
                variable.full_path_string(),
                VariableRef::from_hierarchy_string("tb.clk").full_path_string()
            );
            assert!(field.is_empty());
        }
        _ => panic!("expected signal change list"),
    }
}

#[test]
fn table_view_command_opens_table_for_focused_item() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state_with_variable("tb.clk");
    let variable = VariableRef::from_hierarchy_string("tb.clk");
    let waves = state.user.waves.as_ref().expect("waves");
    let vidx = find_visible_index_for_variable(waves, &variable).expect("visible");
    state.update(Message::FocusItem(vidx));

    let parser = crate::command_parser::get_parser(&state);
    let msg = crate::fzcmd::parse_command("table_view", parser).expect("command");
    state.update(msg);

    assert_eq!(state.user.table_tiles.len(), 1);
}

#[test]
fn table_activate_selection_moves_cursor() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state_with_variable("tb.clk");
    let spec = TableModelSpec::SignalChangeList {
        variable: VariableRef::from_hierarchy_string("tb.clk"),
        field: vec![],
    };
    state.update(Message::AddTableTile { spec: spec.clone() });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile");
    let ctx = state.table_model_context();
    let model = spec.create_model(&ctx).expect("model");
    let row_id = model.row_id_at(0).expect("row");
    let expected_time = match model.on_activate(row_id) {
        TableAction::CursorSet(time) => time,
        _ => panic!("expected cursor set"),
    };

    let runtime = state.table_runtime.entry(tile_id).or_default();
    runtime.model = Some(model);
    let mut selection = TableSelection::new();
    selection.rows.insert(row_id);
    selection.anchor = Some(row_id);
    runtime.selection = selection;

    state.update(Message::TableActivateSelection { tile_id });
    let cursor = state.user.waves.as_ref().expect("waves").cursor.clone();
    assert_eq!(cursor, Some(expected_time));
}

#[test]
fn signal_analysis_selection_moves_cursor_to_interval_end() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    state.update(Message::SetMarker {
        id: 1,
        time: num::BigInt::from(5u64),
    });

    state.update(Message::RunSignalAnalysis {
        config: SignalAnalysisConfig {
            sampling: SignalAnalysisSamplingConfig {
                signal: VariableRef::from_hierarchy_string("tb.clk"),
            },
            signals: vec![SignalAnalysisSignal {
                variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
                field: vec![],
                translator: "Unsigned".to_string(),
            }],
            run_revision: 0,
        },
    });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile");
    wait_for_waves_fully_loaded(&mut state, 10);
    let model_key = state.user.table_tiles[&tile_id]
        .spec
        .model_key_for_tile(tile_id);
    trigger_table_cache_build_for_tile(&mut state, tile_id);
    wait_for_table_cache_ready(&mut state, tile_id, Some(model_key));

    let model = state
        .table_runtime
        .get(&tile_id)
        .and_then(|runtime| runtime.model.clone())
        .expect("analysis model");
    let row_id = model.row_id_at(0).expect("first interval row");
    let expected_time = match model.on_activate(row_id) {
        TableAction::CursorSet(time) => time,
        other => panic!("expected CursorSet, got {other:?}"),
    };

    let mut selection = TableSelection::new();
    selection.rows.insert(row_id);
    selection.anchor = Some(row_id);
    state.update(Message::SetTableSelection { tile_id, selection });

    let cursor = state.user.waves.as_ref().expect("waves").cursor.clone();
    assert_eq!(cursor, Some(expected_time));
}

#[test]
fn signal_change_selection_moves_cursor() {
    // Tests that selecting a row via SetTableSelection updates cursor
    // when activate_on_select is true (SignalChangeList default)
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state_with_variable("tb.clk");
    let spec = TableModelSpec::SignalChangeList {
        variable: VariableRef::from_hierarchy_string("tb.clk"),
        field: vec![],
    };
    state.update(Message::AddTableTile { spec: spec.clone() });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile");
    let ctx = state.table_model_context();
    let model = spec.create_model(&ctx).expect("model");
    let row_id = model.row_id_at(0).expect("row");
    let expected_time = match model.on_activate(row_id) {
        TableAction::CursorSet(time) => time,
        _ => panic!("expected cursor set"),
    };

    // Verify activate_on_select is enabled for SignalChangeList
    let tile_state = state.user.table_tiles.get(&tile_id).expect("tile");
    assert!(
        tile_state.config.activate_on_select,
        "SignalChangeList should have activate_on_select=true"
    );

    // Set cached model in runtime so activation works
    let runtime = state.table_runtime.entry(tile_id).or_default();
    runtime.model = Some(model);

    // Select row via SetTableSelection - should trigger activation
    let mut selection = TableSelection::new();
    selection.rows.insert(row_id);
    selection.anchor = Some(row_id);
    state.update(Message::SetTableSelection { tile_id, selection });

    let cursor = state.user.waves.as_ref().expect("waves").cursor.clone();
    assert_eq!(cursor, Some(expected_time));
}

#[test]
fn virtual_table_selection_does_not_move_cursor() {
    // Tests that selecting a row via SetTableSelection does NOT move cursor
    // when activate_on_select is false (Virtual model default)
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state_with_variable("tb.clk");
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 3,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec: spec.clone() });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile");

    // Verify activate_on_select is disabled for Virtual model
    let tile_state = state.user.table_tiles.get(&tile_id).expect("tile");
    assert!(
        !tile_state.config.activate_on_select,
        "Virtual model should have activate_on_select=false"
    );

    // Clear any existing cursor
    state.update(Message::CursorSet(num::BigInt::from(100)));
    let cursor_before = state.user.waves.as_ref().expect("waves").cursor.clone();

    // Select row via SetTableSelection - should NOT change cursor
    let row_id = TableRowId(0);
    let mut selection = TableSelection::new();
    selection.rows.insert(row_id);
    selection.anchor = Some(row_id);
    state.update(Message::SetTableSelection { tile_id, selection });

    let cursor_after = state.user.waves.as_ref().expect("waves").cursor.clone();
    assert_eq!(
        cursor_before, cursor_after,
        "Cursor should not change for Virtual table selection"
    );
}

// ========================
// FilterDraft Tests
// ========================

#[test]
fn filter_draft_from_spec() {
    let spec = TableSearchSpec {
        text: "foo".into(),
        column: None,
        mode: TableSearchMode::Contains,
        case_sensitive: false,
    };
    let draft = FilterDraft::from_spec(&spec);

    assert_eq!(draft.text, "foo");
    assert_eq!(draft.mode, TableSearchMode::Contains);
    assert!(!draft.case_sensitive);
    assert!(draft.column.is_none());
    assert!(draft.last_changed.is_none());
}

#[test]
fn filter_draft_to_spec() {
    let draft = FilterDraft {
        text: "bar".into(),
        mode: TableSearchMode::Regex,
        case_sensitive: true,
        column: None,
        last_changed: Some(std::time::Instant::now()),
    };
    let spec = draft.to_spec();

    assert_eq!(spec.text, "bar");
    assert_eq!(spec.mode, TableSearchMode::Regex);
    assert!(spec.case_sensitive);
    assert!(spec.column.is_none());
}

#[test]
fn filter_draft_is_dirty() {
    let spec = TableSearchSpec {
        text: "foo".into(),
        column: None,
        mode: TableSearchMode::Contains,
        case_sensitive: false,
    };
    let draft = FilterDraft::from_spec(&spec);

    // Same values → not dirty
    assert!(!draft.is_dirty(&spec));

    // Different text → dirty
    let mut draft2 = draft.clone();
    draft2.text = "bar".into();
    assert!(draft2.is_dirty(&spec));

    // Different mode → dirty
    let mut draft3 = draft.clone();
    draft3.mode = TableSearchMode::Regex;
    assert!(draft3.is_dirty(&spec));

    // Different case → dirty
    let mut draft4 = draft.clone();
    draft4.case_sensitive = true;
    assert!(draft4.is_dirty(&spec));
}

#[test]
fn filter_draft_debounce_elapsed_with_injected_time() {
    use std::time::{Duration, Instant};

    let mut draft = FilterDraft::default();

    // No last_changed → not elapsed
    let now = Instant::now();
    assert!(!draft.debounce_elapsed(now));

    // Just changed → not elapsed
    draft.last_changed = Some(now);
    assert!(!draft.debounce_elapsed(now));

    // 100ms later → not elapsed
    let later_100ms = now + Duration::from_millis(100);
    assert!(!draft.debounce_elapsed(later_100ms));

    // 200ms later → elapsed
    let later_200ms = now + Duration::from_millis(FILTER_DEBOUNCE_MS);
    assert!(draft.debounce_elapsed(later_200ms));

    // 300ms later → still elapsed
    let later_300ms = now + Duration::from_millis(300);
    assert!(draft.debounce_elapsed(later_300ms));
}

#[test]
fn filter_draft_round_trip() {
    let spec = TableSearchSpec {
        text: "test query".into(),
        column: None,
        mode: TableSearchMode::Fuzzy,
        case_sensitive: true,
    };
    let draft = FilterDraft::from_spec(&spec);
    let round_tripped = draft.to_spec();

    assert_eq!(spec.text, round_tripped.text);
    assert_eq!(spec.mode, round_tripped.mode);
    assert_eq!(spec.case_sensitive, round_tripped.case_sensitive);
    assert_eq!(spec.column, round_tripped.column);
}

#[test]
fn filter_draft_round_trip_preserves_column_target() {
    let spec = TableSearchSpec {
        text: "READ".into(),
        mode: TableSearchMode::Exact,
        case_sensitive: true,
        column: Some(TableColumnKey::Str("action".to_string())),
    };
    let draft = FilterDraft::from_spec(&spec);
    let round_tripped = draft.to_spec();

    assert_eq!(
        round_tripped.column,
        Some(TableColumnKey::Str("action".to_string()))
    );
}

#[test]
fn filter_draft_default() {
    let draft = FilterDraft::default();

    assert!(draft.text.is_empty());
    assert_eq!(draft.mode, TableSearchMode::Contains);
    assert!(!draft.case_sensitive);
    assert!(draft.column.is_none());
    assert!(draft.last_changed.is_none());
}

#[test]
fn set_table_display_filter_syncs_draft() {
    // Tests that SetTableDisplayFilter updates both config and runtime draft
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 3,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec });
    let tile_id = *state.user.table_tiles.keys().next().expect("tile");

    // Initialize runtime
    state.table_runtime.entry(tile_id).or_default();

    // Set filter
    let filter = TableSearchSpec {
        mode: TableSearchMode::Regex,
        case_sensitive: true,
        text: "test".to_string(),
        column: None,
    };
    state.update(Message::SetTableDisplayFilter {
        tile_id,
        filter: filter.clone(),
    });

    // Verify config updated
    assert_eq!(
        state.user.table_tiles[&tile_id].config.display_filter,
        filter
    );

    // Verify draft synced
    let runtime = state.table_runtime.get(&tile_id).expect("runtime");
    let draft = runtime.filter_draft.as_ref().expect("draft");
    assert_eq!(draft.text, "test");
    assert_eq!(draft.mode, TableSearchMode::Regex);
    assert!(draft.case_sensitive);
    // Timestamp should be None after sync (to prevent immediate re-apply)
    assert!(draft.last_changed.is_none());
}
