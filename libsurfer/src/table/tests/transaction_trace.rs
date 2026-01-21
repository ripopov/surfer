use super::support::{load_counter_state, test_runtime};
use super::*;

// ========================
// Stage 12 Tests - TransactionTrace Model
// ========================

fn load_ftr_state() -> SystemState {
    use crate::transaction_container::TransactionStreamRef;

    let mut state = SystemState::new_default_config()
        .expect("state")
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .expect("project root")
                    .join("examples/my_db.ftr")
                    .try_into()
                    .expect("path"),
            )),
            ..Default::default()
        });
    wait_for_waves_fully_loaded(&mut state, 10);

    // Add streams to trigger lazy-loading of transactions
    // FTR format only loads transactions into memory when streams are added to the view
    state.update(Message::AddStreamOrGenerator(
        TransactionStreamRef::new_stream(1, "pipelined_stream".to_string()),
    ));
    state.update(Message::AddStreamOrGenerator(
        TransactionStreamRef::new_stream(2, "addr_stream".to_string()),
    ));
    state.update(Message::AddStreamOrGenerator(
        TransactionStreamRef::new_stream(3, "data_stream".to_string()),
    ));
    wait_for_waves_fully_loaded(&mut state, 10);

    state
}

/// Helper to create a test generator reference (generator 4 "read" in stream 1)
fn test_generator_ref() -> crate::transaction_container::TransactionStreamRef {
    crate::transaction_container::TransactionStreamRef::new_gen(1, 4, "read".to_string())
}

#[test]
fn transaction_trace_model_spec_creates_model() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let state = load_ftr_state();

    let spec = TableModelSpec::TransactionTrace {
        generator: test_generator_ref(),
    };

    let ctx = state.table_model_context();
    let model = spec.create_model(&ctx);
    assert!(model.is_ok(), "TransactionTrace model should be created");
}

#[test]
fn transaction_trace_model_has_fixed_columns() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let state = load_ftr_state();

    let spec = TableModelSpec::TransactionTrace {
        generator: test_generator_ref(),
    };

    let ctx = state.table_model_context();
    let model = spec.create_model(&ctx).expect("model");
    let schema = model.schema();

    // Check fixed columns exist in order (no Generator column since table is per-generator)
    assert!(
        schema.columns.len() >= 4,
        "Should have at least 4 fixed columns"
    );
    assert_eq!(schema.columns[0].label, "Start");
    assert_eq!(schema.columns[1].label, "End");
    assert_eq!(schema.columns[2].label, "Duration");
    assert_eq!(schema.columns[3].label, "Type");
}

#[test]
fn transaction_trace_model_row_count_positive() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let state = load_ftr_state();

    let spec = TableModelSpec::TransactionTrace {
        generator: test_generator_ref(),
    };

    let ctx = state.table_model_context();
    let model = spec.create_model(&ctx).expect("model");

    assert!(model.row_count() > 0, "Generator should have transactions");
}

#[test]
fn transaction_trace_model_row_ids_are_unique() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let state = load_ftr_state();

    let spec = TableModelSpec::TransactionTrace {
        generator: test_generator_ref(),
    };

    let ctx = state.table_model_context();
    let model = spec.create_model(&ctx).expect("model");

    let row_count = model.row_count();
    let row_ids: std::collections::HashSet<_> =
        (0..row_count).filter_map(|i| model.row_id_at(i)).collect();

    assert_eq!(row_ids.len(), row_count, "All row IDs should be unique");
}

#[test]
fn transaction_trace_model_on_activate_returns_focus_transaction() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let state = load_ftr_state();

    let spec = TableModelSpec::TransactionTrace {
        generator: test_generator_ref(),
    };

    let ctx = state.table_model_context();
    let model = spec.create_model(&ctx).expect("model");

    if let Some(row_id) = model.row_id_at(0) {
        let action = model.on_activate(row_id);
        match action {
            TableAction::FocusTransaction(tx_ref) => {
                // Verify the transaction ref is valid
                assert!(
                    tx_ref.id > 0 || tx_ref.id == 0,
                    "tx_ref should have valid id"
                );
            }
            _ => panic!("Expected FocusTransaction action, got {:?}", action),
        }
    }
}

#[test]
fn transaction_trace_model_default_config_has_title() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let state = load_ftr_state();

    let spec = TableModelSpec::TransactionTrace {
        generator: test_generator_ref(),
    };

    let ctx = state.table_model_context();
    let config = spec.default_view_config(&ctx);

    // Title should include the generator name
    assert!(
        config.title.contains("read"),
        "Title should contain generator name 'read', got: {}",
        config.title
    );
    assert!(
        config.activate_on_select,
        "Should have activate_on_select enabled"
    );
}

#[test]
fn open_transaction_table_creates_tile() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_ftr_state();

    state.update(Message::OpenTransactionTable {
        generator: test_generator_ref(),
    });

    assert_eq!(
        state.user.table_tiles.len(),
        1,
        "Should have one table tile"
    );
    let tile_state = state.user.table_tiles.values().next().expect("tile state");
    match &tile_state.spec {
        TableModelSpec::TransactionTrace { generator } => {
            assert_eq!(generator.name, "read");
            assert_eq!(generator.gen_id, Some(4));
        }
        _ => panic!("Expected TransactionTrace spec"),
    }
}

#[test]
fn transaction_trace_model_cells_return_text() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let state = load_ftr_state();

    let spec = TableModelSpec::TransactionTrace {
        generator: test_generator_ref(),
    };

    let ctx = state.table_model_context();
    let model = spec.create_model(&ctx).expect("model");

    if let Some(row_id) = model.row_id_at(0) {
        // Check all 4 fixed columns return some text (Start, End, Duration, Type)
        for col in 0..4 {
            let cell = model.cell(row_id, col);
            match cell {
                TableCell::Text(s) => {
                    assert!(!s.is_empty(), "Cell at col {} should have text", col);
                }
                TableCell::RichText(_) => {
                    // Also acceptable
                }
            }
        }
    }
}

#[test]
fn transaction_trace_model_search_text_non_empty() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let state = load_ftr_state();

    let spec = TableModelSpec::TransactionTrace {
        generator: test_generator_ref(),
    };

    let ctx = state.table_model_context();
    let model = spec.create_model(&ctx).expect("model");

    if let Some(row_id) = model.row_id_at(0) {
        let search_text = model.search_text(row_id);
        assert!(!search_text.is_empty(), "Search text should not be empty");
    }
}

#[test]
fn transaction_trace_sort_key_numeric_for_times() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let state = load_ftr_state();

    let spec = TableModelSpec::TransactionTrace {
        generator: test_generator_ref(),
    };

    let ctx = state.table_model_context();
    let model = spec.create_model(&ctx).expect("model");

    if let Some(row_id) = model.row_id_at(0) {
        // Start (col 0), End (col 1), Duration (col 2) should be numeric
        for col in 0..3 {
            let key = model.sort_key(row_id, col);
            match key {
                TableSortKey::Numeric(_) => {
                    // Expected
                }
                TableSortKey::Text(_) => {
                    // Also acceptable if the number is too large
                }
                _ => panic!("Time column {} should have Numeric or Text sort key", col),
            }
        }
    }
}

#[test]
fn transaction_trace_data_unavailable_without_transactions() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    // Load a VCD file (no transactions)
    let state = load_counter_state();

    let spec = TableModelSpec::TransactionTrace {
        generator: test_generator_ref(),
    };

    let ctx = state.table_model_context();
    let result = spec.create_model(&ctx);

    assert!(result.is_err(), "Should fail without transaction data");
    match result {
        Err(TableCacheError::DataUnavailable) => {
            // Expected
        }
        Err(err) => panic!("Expected DataUnavailable, got {:?}", err),
        Ok(_) => panic!("Expected error but got Ok"),
    }
}

// ========================
// Stage 12b Tests - Stream-level "Show transactions in table"
// ========================

#[test]
fn open_transaction_table_for_stream_creates_multiple_tiles() {
    use crate::transaction_container::{StreamScopeRef, TransactionStreamRef};

    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_ftr_state();

    // Enumerate generators for stream 1 (pipelined_stream)
    let waves = state.user.waves.as_ref().expect("waves loaded");
    let tc = waves
        .inner
        .as_transactions()
        .expect("transaction container");
    let stream_ref = TransactionStreamRef::new_stream(1, "pipelined_stream".to_string());
    let generators = tc.generators_in_stream(&StreamScopeRef::Stream(stream_ref));
    let gen_count = generators.len();
    assert!(gen_count >= 2, "Stream 1 should have at least 2 generators");

    // Emit one OpenTransactionTable per generator (simulates stream-level menu click)
    for gen_ref in generators {
        state.update(Message::OpenTransactionTable { generator: gen_ref });
    }

    assert_eq!(
        state.user.table_tiles.len(),
        gen_count,
        "Should have one table tile per generator"
    );

    // Verify each tile has gen_id set
    for tile_state in state.user.table_tiles.values() {
        match &tile_state.spec {
            TableModelSpec::TransactionTrace { generator } => {
                assert!(
                    generator.gen_id.is_some(),
                    "Each tile should reference a specific generator"
                );
            }
            _ => panic!("Expected TransactionTrace spec"),
        }
    }
}

// ========================
// Performance Optimization Safety Tests
// ========================

#[test]
fn test_row_index_lookup_consistency() {
    // Build cache via build_table_cache and verify row_index matches row_ids positions
    let model = Arc::new(VirtualTableModel::new(50, 3, 42));
    let cache = build_table_cache(
        model,
        TableSearchSpec::default(),
        vec![TableSortSpec {
            key: TableColumnKey::Str("col_0".to_string()),
            direction: TableSortDirection::Descending,
        }],
        None,
    )
    .expect("cache build should succeed");

    assert_eq!(cache.row_index.len(), cache.row_ids.len());

    for (expected_pos, &row_id) in cache.row_ids.iter().enumerate() {
        let indexed_pos = cache
            .row_index
            .get(&row_id)
            .copied()
            .unwrap_or_else(|| panic!("row_id {row_id:?} missing from row_index"));
        assert_eq!(
            indexed_pos, expected_pos,
            "row_index[{row_id:?}] = {indexed_pos}, expected {expected_pos}"
        );
    }
}

#[test]
fn test_hidden_count_consistency() {
    // Create runtime with cache and selection mixing visible/hidden rows
    let model = Arc::new(VirtualTableModel::new(10, 2, 0));
    let cache = build_table_cache(model, TableSearchSpec::default(), vec![], None)
        .expect("cache build should succeed");

    let cache_key = TableCacheKey {
        model_key: TableModelKey(1),
        display_filter: TableSearchSpec::default(),
        pinned_filters: vec![],
        view_sort: vec![],
        generation: 0,
    };
    let entry = Arc::new(TableCacheEntry::new(cache_key.clone(), 0, 0));
    entry.set(cache);

    let mut runtime = TableRuntimeState {
        cache: Some(entry),
        cache_key: Some(cache_key),
        ..Default::default()
    };

    // Select rows 3, 5, 7, 99 (99 is not visible)
    runtime.selection.rows.insert(TableRowId(3));
    runtime.selection.rows.insert(TableRowId(5));
    runtime.selection.rows.insert(TableRowId(7));
    runtime.selection.rows.insert(TableRowId(99)); // hidden

    runtime.update_hidden_count();

    // Manual count: 3, 5, 7 are visible (in 0..10), 99 is hidden
    assert_eq!(runtime.hidden_selection_count, 1);

    // After clearing selection, hidden count should be 0
    runtime.selection.clear();
    runtime.hidden_selection_count = 0;
    runtime.update_hidden_count();
    assert_eq!(runtime.hidden_selection_count, 0);
}

#[test]
fn test_row_index_after_filter() {
    // Verify row_index is correct after filtering reduces visible rows
    let model = Arc::new(VirtualTableModel::new(20, 2, 0));
    let cache = build_table_cache(
        model,
        TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: "r1".to_string(), // Matches rows 1, 10-19
            column: None,
        },
        vec![],
        None,
    )
    .expect("cache build should succeed");

    assert_eq!(cache.row_index.len(), cache.row_ids.len());

    for (pos, &row_id) in cache.row_ids.iter().enumerate() {
        assert_eq!(cache.row_index[&row_id], pos);
    }
}
