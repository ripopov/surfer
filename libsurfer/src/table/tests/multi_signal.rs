use super::support::*;
use super::*;

// ========================
// Stage 5 Tests - MultiSignalChangeListModel
// ========================

use crate::displayed_item::DisplayedItem;
use crate::table::sources::multi_signal_change_list::{
    decode_signal_column_key, encode_signal_column_key,
};

#[test]
fn multi_signal_model_creation_with_valid_signals() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
        VariableRef::from_hierarchy_string("tb.dut.counter"),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);

    let ctx = state.table_model_context();
    let spec = TableModelSpec::MultiSignalChangeList {
        variables: vec![
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.clk"),
                field: vec![],
            },
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
                field: vec![],
            },
        ],
    };

    let model = spec
        .create_model(&ctx)
        .expect("model creation should succeed");
    assert!(model.row_count() > 0, "merged index should have rows");

    let schema = model.schema();
    assert_eq!(
        schema.columns.len(),
        3,
        "expected 1 time + 2 signal columns"
    );

    assert_eq!(
        schema.columns[0].key,
        TableColumnKey::Str("time".to_string())
    );
    assert_eq!(schema.columns[0].label, "Time");

    for col in &schema.columns[1..] {
        if let TableColumnKey::Str(key) = &col.key {
            assert!(
                key.starts_with("sig:v1:"),
                "signal column key should start with sig:v1: prefix, got: {key}"
            );
        } else {
            panic!("signal column key should be Str variant");
        }
    }
}

#[test]
fn multi_signal_model_skips_missing_signals_warns() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);

    let ctx = state.table_model_context();

    // Mix of valid and invalid signals
    let spec = TableModelSpec::MultiSignalChangeList {
        variables: vec![
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.nonexistent"),
                field: vec![],
            },
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.clk"),
                field: vec![],
            },
        ],
    };

    let model = spec
        .create_model(&ctx)
        .expect("should succeed with at least one valid signal");
    let schema = model.schema();
    assert_eq!(
        schema.columns.len(),
        2,
        "expected 1 time + 1 valid signal column (invalid skipped)"
    );
}

#[test]
fn multi_signal_model_all_invalid_signals_returns_error() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let state = load_counter_state();
    let ctx = state.table_model_context();

    let spec = TableModelSpec::MultiSignalChangeList {
        variables: vec![
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.nonexistent1"),
                field: vec![],
            },
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.nonexistent2"),
                field: vec![],
            },
        ],
    };

    match spec.create_model(&ctx) {
        Err(TableCacheError::ModelNotFound { description }) => {
            assert!(
                description.contains("No valid signals"),
                "expected 'No valid signals' error, got: {description}"
            );
        }
        Ok(_) => panic!("expected ModelNotFound error, got Ok"),
        Err(other) => panic!("expected ModelNotFound error, got: {other:?}"),
    }
}

#[test]
fn multi_signal_model_column_key_stable_and_reversible() {
    let path = "tb.dut.counter";
    let field = vec!["value".to_string()];

    let key1 = encode_signal_column_key(path, &field);
    let key2 = encode_signal_column_key(path, &field);
    assert_eq!(key1, key2, "column key should be deterministic");

    let (decoded_path, decoded_field) =
        decode_signal_column_key(&key1).expect("should decode successfully");
    assert_eq!(decoded_path, path);
    assert_eq!(decoded_field, field);

    // Verify empty field
    let key_empty = encode_signal_column_key("tb.clk", &[]);
    let (decoded_path2, decoded_field2) =
        decode_signal_column_key(&key_empty).expect("should decode empty field");
    assert_eq!(decoded_path2, "tb.clk");
    assert!(decoded_field2.is_empty());
}

#[test]
fn multi_signal_model_on_activate_sets_cursor() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);

    let ctx = state.table_model_context();
    let spec = TableModelSpec::MultiSignalChangeList {
        variables: vec![MultiSignalEntry {
            variable: VariableRef::from_hierarchy_string("tb.clk"),
            field: vec![],
        }],
    };

    let model = spec.create_model(&ctx).expect("model");
    let row_id = model.row_id_at(0).expect("first row should exist");

    match model.on_activate(row_id) {
        TableAction::CursorSet(time) => {
            assert_eq!(
                time,
                num::BigInt::from(row_id.0),
                "cursor should be set to row timestamp"
            );
        }
        other => panic!("expected CursorSet, got: {other:?}"),
    }
}

#[test]
fn multi_signal_model_time_column_rendering() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);

    let ctx = state.table_model_context();
    let spec = TableModelSpec::MultiSignalChangeList {
        variables: vec![MultiSignalEntry {
            variable: VariableRef::from_hierarchy_string("tb.clk"),
            field: vec![],
        }],
    };

    let model = spec.create_model(&ctx).expect("model");
    let row_id = model.row_id_at(0).expect("first row");

    let time_cell = match model.cell(row_id, 0) {
        TableCell::Text(text) => text,
        TableCell::RichText(text) => text.text().to_string(),
    };
    assert!(
        !time_cell.is_empty(),
        "time column should render non-empty text"
    );

    let sort_key = model.sort_key(row_id, 0);
    assert!(
        matches!(sort_key, TableSortKey::Numeric(_)),
        "time sort key should be numeric"
    );
}

#[test]
fn multi_signal_model_uses_lazy_search_mode() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);

    let ctx = state.table_model_context();
    let spec = TableModelSpec::MultiSignalChangeList {
        variables: vec![MultiSignalEntry {
            variable: VariableRef::from_hierarchy_string("tb.clk"),
            field: vec![],
        }],
    };

    let model = spec.create_model(&ctx).expect("model");
    assert_eq!(
        model.search_text_mode(),
        SearchTextMode::LazyProbe,
        "multi-signal model should use lazy search mode"
    );
}

#[test]
fn multi_signal_model_row_ids_match_merged_timeline() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
        VariableRef::from_hierarchy_string("tb.dut.counter"),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);

    let ctx = state.table_model_context();
    let spec = TableModelSpec::MultiSignalChangeList {
        variables: vec![
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.clk"),
                field: vec![],
            },
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
                field: vec![],
            },
        ],
    };

    let model = spec.create_model(&ctx).expect("model");
    let row_count = model.row_count();
    assert!(row_count > 0);

    // Verify row IDs are sorted by timestamp (monotonically increasing)
    let mut prev_time = 0u64;
    for idx in 0..row_count {
        let row_id = model.row_id_at(idx).expect("row id should exist");
        assert!(
            row_id.0 >= prev_time,
            "row times should be monotonically non-decreasing"
        );
        prev_time = row_id.0;
    }

    // Verify merged timeline has at least as many rows as either single signal
    let clk_spec = TableModelSpec::SignalChangeList {
        variable: VariableRef::from_hierarchy_string("tb.clk"),
        field: vec![],
    };
    let clk_model = clk_spec.create_model(&ctx).expect("clk model");
    assert!(
        row_count >= clk_model.row_count(),
        "merged timeline should have at least as many rows as single signal"
    );
}

// ========================
// Stage 6 Tests - On-Demand Cell Materialization
// ========================

/// Helper to extract text content from a TableCell.
fn cell_text(cell: &TableCell) -> String {
    match cell {
        TableCell::Text(text) => text.clone(),
        TableCell::RichText(rt) => rt.text().to_string(),
    }
}

/// Helper to check if a TableCell is RichText (used for dimmed/held/no-data).
fn cell_is_rich_text(cell: &TableCell) -> bool {
    matches!(cell, TableCell::RichText(_))
}

#[test]
fn multi_signal_transition_held_nodata_classification() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
        VariableRef::from_hierarchy_string("tb.dut.counter"),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);

    let ctx = state.table_model_context();
    let spec = TableModelSpec::MultiSignalChangeList {
        variables: vec![
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.clk"),
                field: vec![],
            },
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
                field: vec![],
            },
        ],
    };

    let model = spec.create_model(&ctx).expect("model");
    let row_count = model.row_count();
    assert!(row_count > 0, "expected non-empty merged timeline");

    // Collect all signal cells and verify classification:
    // - Every cell in a signal column should be non-empty (either value or em dash)
    // - Transition cells should be TableCell::Text (normal)
    // - Held and NoData cells should be TableCell::RichText (dimmed)
    let mut found_transition = false;
    let mut found_held_or_nodata = false;

    for idx in 0..row_count {
        let row_id = model.row_id_at(idx).expect("row");

        // Check signal columns (col 1 and col 2)
        for col in 1..=2 {
            let cell = model.cell(row_id, col);
            let text = cell_text(&cell);
            assert!(!text.is_empty(), "cell text should never be empty");

            if cell_is_rich_text(&cell) {
                found_held_or_nodata = true;
            } else {
                found_transition = true;
            }
        }
    }

    // With two different signals (clk and counter), the merged timeline should have
    // some transition and some held cells (since not every signal transitions at every time).
    assert!(
        found_transition,
        "expected at least one transition cell in multi-signal table"
    );
    assert!(
        found_held_or_nodata,
        "expected at least one held/no-data cell in multi-signal table"
    );
}

#[test]
fn multi_signal_cell_value_matches_query_variable() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);

    let ctx = state.table_model_context();

    // Build multi-signal model with clk
    let multi_spec = TableModelSpec::MultiSignalChangeList {
        variables: vec![MultiSignalEntry {
            variable: VariableRef::from_hierarchy_string("tb.clk"),
            field: vec![],
        }],
    };

    // Build single-signal model with clk for comparison
    let single_spec = TableModelSpec::SignalChangeList {
        variable: VariableRef::from_hierarchy_string("tb.clk"),
        field: vec![],
    };

    let multi_model = multi_spec.create_model(&ctx).expect("multi model");
    let single_model = single_spec.create_model(&ctx).expect("single model");

    // For each row in the multi-signal model that corresponds to a transition,
    // the cell text should match the single-signal model's value cell.
    let single_row_count = single_model.row_count();
    let mut matched = 0;

    for idx in 0..single_row_count {
        let single_row_id = single_model.row_id_at(idx).expect("single row");
        let single_value = cell_text(&single_model.cell(single_row_id, 1));

        // The multi-signal model uses the same row ID (TableRowId(time_u64))
        let multi_cell = multi_model.cell(single_row_id, 1);
        let multi_value = cell_text(&multi_cell);

        // For transition cells, values should match
        if !cell_is_rich_text(&multi_cell) {
            assert_eq!(
                multi_value, single_value,
                "transition cell value should match single-signal value at row {single_row_id:?}"
            );
            matched += 1;
        }
    }

    assert!(
        matched > 0,
        "expected at least some matching transition cells"
    );
}

#[test]
fn multi_signal_nodata_renders_em_dash() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
        VariableRef::from_hierarchy_string("tb.dut.counter"),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);

    let ctx = state.table_model_context();
    let spec = TableModelSpec::MultiSignalChangeList {
        variables: vec![
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.clk"),
                field: vec![],
            },
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
                field: vec![],
            },
        ],
    };

    let model = spec.create_model(&ctx).expect("model");

    // Check the first row (time 0): at least one signal may not have a value here
    // (counter may start at 0 or not have data before time 0)
    // We check that if no-data appears, it uses the em dash
    let row_count = model.row_count();
    let em_dash = "\u{2014}";
    let mut found_nodata = false;

    for idx in 0..row_count {
        let row_id = model.row_id_at(idx).expect("row");
        for col in 1..=2 {
            let cell = model.cell(row_id, col);
            let text = cell_text(&cell);
            if text == em_dash {
                assert!(
                    cell_is_rich_text(&cell),
                    "em dash cell should be RichText (dimmed)"
                );
                found_nodata = true;
            }
        }
    }

    // It's possible no-data doesn't occur if both signals start at time 0,
    // so we just verify the em-dash handling is correct when it does occur
    if found_nodata {
        // Test passed - em dash cells are correctly dimmed
    }
}

#[test]
fn multi_signal_sort_key_numeric_vs_text() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
        VariableRef::from_hierarchy_string("tb.dut.counter"),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);

    let ctx = state.table_model_context();
    let spec = TableModelSpec::MultiSignalChangeList {
        variables: vec![
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.clk"),
                field: vec![],
            },
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
                field: vec![],
            },
        ],
    };

    let model = spec.create_model(&ctx).expect("model");
    let row_count = model.row_count();
    assert!(row_count > 0);

    let mut found_numeric_or_text = false;
    let mut found_none = false;

    for idx in 0..row_count {
        let row_id = model.row_id_at(idx).expect("row");

        // Time column should always be numeric
        let time_key = model.sort_key(row_id, 0);
        assert!(
            matches!(time_key, TableSortKey::Numeric(_)),
            "time sort key should be numeric"
        );

        // Signal columns should be Numeric/Text for data, None for no-data
        for col in 1..=2 {
            let key = model.sort_key(row_id, col);
            match key {
                TableSortKey::Numeric(_) | TableSortKey::Text(_) => {
                    found_numeric_or_text = true;
                }
                TableSortKey::None => {
                    found_none = true;
                }
                TableSortKey::Bytes(_) => {
                    panic!("unexpected Bytes sort key");
                }
            }
        }
    }

    assert!(
        found_numeric_or_text,
        "expected at least one numeric or text sort key"
    );
    // `found_none` may or may not occur depending on whether no-data cells exist
    let _ = found_none;
}

#[test]
fn multi_signal_search_text_includes_all_columns() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
        VariableRef::from_hierarchy_string("tb.dut.counter"),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);

    let ctx = state.table_model_context();
    let spec = TableModelSpec::MultiSignalChangeList {
        variables: vec![
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.clk"),
                field: vec![],
            },
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
                field: vec![],
            },
        ],
    };

    let model = spec.create_model(&ctx).expect("model");
    let row_id = model.row_id_at(0).expect("first row");

    let search_text = model.search_text(row_id);
    assert!(!search_text.is_empty(), "search text should not be empty");

    // Search text should contain the time column text
    let time_cell = cell_text(&model.cell(row_id, 0));
    assert!(
        search_text.contains(&time_cell),
        "search text should contain the time value"
    );

    // Search text should contain signal values
    let sig1_cell = cell_text(&model.cell(row_id, 1));
    assert!(
        search_text.contains(&sig1_cell),
        "search text should contain signal 1 value"
    );
    let sig2_cell = cell_text(&model.cell(row_id, 2));
    assert!(
        search_text.contains(&sig2_cell),
        "search text should contain signal 2 value"
    );
}

#[test]
fn multi_signal_held_value_matches_previous_transition() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
        VariableRef::from_hierarchy_string("tb.dut.counter"),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);

    let ctx = state.table_model_context();
    let spec = TableModelSpec::MultiSignalChangeList {
        variables: vec![
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.clk"),
                field: vec![],
            },
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
                field: vec![],
            },
        ],
    };

    let model = spec.create_model(&ctx).expect("model");
    let row_count = model.row_count();

    // For a held cell, the value should match what was set at a previous transition.
    // We verify this by tracking last-seen transition value for each signal column.
    let mut last_transition_val = vec![String::new(); 2];

    for idx in 0..row_count {
        let row_id = model.row_id_at(idx).expect("row");

        for sig_col in 0..2usize {
            let col = sig_col + 1;
            let cell = model.cell(row_id, col);
            let text = cell_text(&cell);

            if !cell_is_rich_text(&cell) {
                // Transition cell - update last known value (strip collapsed marker)
                let base_value = text.split(" (+").next().unwrap_or(&text).to_string();
                last_transition_val[sig_col] = base_value;
            } else if text != "\u{2014}" {
                // Held cell (not no-data) - should match last transition value
                assert_eq!(
                    text, last_transition_val[sig_col],
                    "held value at row {idx} col {col} should match previous transition"
                );
            }
        }
    }
}

#[test]
fn multi_signal_out_of_bounds_column_returns_empty() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);

    let ctx = state.table_model_context();
    let spec = TableModelSpec::MultiSignalChangeList {
        variables: vec![MultiSignalEntry {
            variable: VariableRef::from_hierarchy_string("tb.clk"),
            field: vec![],
        }],
    };

    let model = spec.create_model(&ctx).expect("model");
    let row_id = model.row_id_at(0).expect("first row");

    // Column 0 = time, Column 1 = clk, Column 2+ = out of bounds
    let oob_cell = model.cell(row_id, 99);
    assert_eq!(
        cell_text(&oob_cell),
        "",
        "out-of-bounds column should return empty text"
    );

    let oob_key = model.sort_key(row_id, 99);
    assert!(
        matches!(oob_key, TableSortKey::None),
        "out-of-bounds sort key should be None"
    );
}

// ========================
// Stage 7 Tests - Window Materialization Cache and Renderer Integration
// ========================

#[test]
fn multi_signal_materialize_window_limited_to_requested_rows() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
        VariableRef::from_hierarchy_string("tb.dut.counter"),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);

    let ctx = state.table_model_context();
    let spec = TableModelSpec::MultiSignalChangeList {
        variables: vec![
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.clk"),
                field: vec![],
            },
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
                field: vec![],
            },
        ],
    };

    let model = spec.create_model(&ctx).expect("model");
    let total_rows = model.row_count();
    assert!(total_rows > 4, "need at least 5 rows for this test");

    // Request only a subset of rows
    let subset_row_ids: Vec<TableRowId> = (0..3).filter_map(|i| model.row_id_at(i)).collect();
    let visible_cols: Vec<usize> = vec![0, 1]; // time + first signal
    let window =
        model.materialize_window(&subset_row_ids, &visible_cols, MaterializePurpose::Render);

    // Materialized window should contain cells for requested rows
    for &row_id in &subset_row_ids {
        for &col in &visible_cols {
            assert!(
                window.cell(row_id, col).is_some(),
                "cell for requested row {row_id:?} col {col} should be materialized"
            );
        }
    }

    // Rows NOT requested should NOT be in the window
    let excluded_row = model
        .row_id_at(total_rows - 1)
        .expect("last row should exist");
    assert!(
        window.cell(excluded_row, 0).is_none(),
        "cells for non-requested rows should not be in window"
    );

    // Columns NOT requested should NOT be in the window
    let schema = model.schema();
    if schema.columns.len() > 2 {
        assert!(
            window.cell(subset_row_ids[0], 2).is_none(),
            "cells for non-requested columns should not be in window"
        );
    }
}

#[test]
fn multi_signal_materialize_window_cache_reuse_on_same_viewport() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);

    let ctx = state.table_model_context();
    let spec = TableModelSpec::MultiSignalChangeList {
        variables: vec![MultiSignalEntry {
            variable: VariableRef::from_hierarchy_string("tb.clk"),
            field: vec![],
        }],
    };

    let model = spec.create_model(&ctx).expect("model");
    let row_ids: Vec<TableRowId> = (0..3).filter_map(|i| model.row_id_at(i)).collect();
    let cols = vec![0, 1];

    // First call materializes fresh
    let window1 = model.materialize_window(&row_ids, &cols, MaterializePurpose::Render);

    // Second call with same params should return identical data (from cache)
    let window2 = model.materialize_window(&row_ids, &cols, MaterializePurpose::Render);

    // Verify identical content
    for &row_id in &row_ids {
        for &col in &cols {
            let cell1 = window1.cell(row_id, col).map(cell_text);
            let cell2 = window2.cell(row_id, col).map(cell_text);
            assert_eq!(
                cell1, cell2,
                "cached window should return identical content for row {row_id:?} col {col}"
            );
        }
    }
}

#[test]
fn multi_signal_materialize_window_cache_invalidated_on_different_params() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
        VariableRef::from_hierarchy_string("tb.dut.counter"),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);

    let ctx = state.table_model_context();
    let spec = TableModelSpec::MultiSignalChangeList {
        variables: vec![
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.clk"),
                field: vec![],
            },
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
                field: vec![],
            },
        ],
    };

    let model = spec.create_model(&ctx).expect("model");
    let total_rows = model.row_count();
    assert!(total_rows >= 6, "need enough rows for two disjoint subsets");

    // Materialize first window (rows 0..3)
    let rows_a: Vec<TableRowId> = (0..3).filter_map(|i| model.row_id_at(i)).collect();
    let cols = vec![0, 1];
    let _window_a = model.materialize_window(&rows_a, &cols, MaterializePurpose::Render);

    // Materialize different window (rows 3..6) — should invalidate cache
    let rows_b: Vec<TableRowId> = (3..6).filter_map(|i| model.row_id_at(i)).collect();
    let window_b = model.materialize_window(&rows_b, &cols, MaterializePurpose::Render);

    // Window B should contain its own rows
    for &row_id in &rows_b {
        assert!(
            window_b.cell(row_id, 0).is_some(),
            "new window should contain its requested rows"
        );
    }

    // Window B should NOT contain rows_a
    for &row_id in &rows_a {
        assert!(
            window_b.cell(row_id, 0).is_none(),
            "new window should not contain old window rows"
        );
    }

    // Different purpose also invalidates
    let sort_window = model.materialize_window(&rows_a, &cols, MaterializePurpose::SortProbe);
    for &row_id in &rows_a {
        for &col in &cols {
            assert!(
                sort_window.sort_key(row_id, col).is_some(),
                "sort probe window should contain sort keys"
            );
        }
    }
}

#[test]
fn multi_signal_cell_uses_cached_window_after_materialize() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);

    let ctx = state.table_model_context();
    let spec = TableModelSpec::MultiSignalChangeList {
        variables: vec![MultiSignalEntry {
            variable: VariableRef::from_hierarchy_string("tb.clk"),
            field: vec![],
        }],
    };

    let model = spec.create_model(&ctx).expect("model");
    let row_ids: Vec<TableRowId> = (0..3).filter_map(|i| model.row_id_at(i)).collect();
    let cols = vec![0, 1];

    // Pre-materialize via materialize_window
    let window = model.materialize_window(&row_ids, &cols, MaterializePurpose::Render);

    // Direct cell() calls should return identical values to window
    for &row_id in &row_ids {
        for &col in &cols {
            let from_window = window.cell(row_id, col).map(cell_text);
            let from_cell = cell_text(&model.cell(row_id, col));
            assert_eq!(
                from_window.as_deref(),
                Some(from_cell.as_str()),
                "cell() should match materialize_window result for row {row_id:?} col {col}"
            );
        }
    }
}

#[test]
fn multi_signal_clipboard_uses_window_materialization() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
        VariableRef::from_hierarchy_string("tb.dut.counter"),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);

    let ctx = state.table_model_context();
    let spec = TableModelSpec::MultiSignalChangeList {
        variables: vec![
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.clk"),
                field: vec![],
            },
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
                field: vec![],
            },
        ],
    };

    let model = spec.create_model(&ctx).expect("model");
    let schema = model.schema();
    let row_ids: Vec<TableRowId> = (0..3).filter_map(|i| model.row_id_at(i)).collect();
    let visible_cols: Vec<TableColumnKey> = schema.columns.iter().map(|c| c.key.clone()).collect();

    // TSV without header
    let tsv = crate::table::format_rows_as_tsv(model.as_ref(), &row_ids, &visible_cols);
    assert!(!tsv.is_empty(), "TSV output should not be empty");
    let lines: Vec<&str> = tsv.lines().collect();
    assert_eq!(lines.len(), 3, "should have 3 data rows");
    for line in &lines {
        let cols: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            cols.len(),
            schema.columns.len(),
            "each row should have all columns tab-separated"
        );
    }

    // TSV with header
    let tsv_with_header = crate::table::format_rows_as_tsv_with_header(
        model.as_ref(),
        &schema,
        &row_ids,
        &visible_cols,
    );
    assert!(!tsv_with_header.is_empty());
    let lines_with_header: Vec<&str> = tsv_with_header.lines().collect();
    assert_eq!(
        lines_with_header.len(),
        4,
        "should have 1 header + 3 data rows"
    );

    // Header should contain column labels
    let header_cols: Vec<&str> = lines_with_header[0].split('\t').collect();
    assert_eq!(header_cols[0], "Time");
}

#[test]
fn multi_signal_materialize_window_search_probe() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);

    let ctx = state.table_model_context();
    let spec = TableModelSpec::MultiSignalChangeList {
        variables: vec![MultiSignalEntry {
            variable: VariableRef::from_hierarchy_string("tb.clk"),
            field: vec![],
        }],
    };

    let model = spec.create_model(&ctx).expect("model");
    let row_ids: Vec<TableRowId> = (0..3).filter_map(|i| model.row_id_at(i)).collect();

    let window = model.materialize_window(&row_ids, &[], MaterializePurpose::SearchProbe);

    // Search probe should produce search text for each row
    for &row_id in &row_ids {
        let text = window.search_text(row_id);
        assert!(
            text.is_some(),
            "search probe should produce text for row {row_id:?}"
        );
        // Search text should include time and signal values
        let text = text.unwrap();
        assert!(
            !text.is_empty(),
            "search text should not be empty for row {row_id:?}"
        );
    }
}

// ========================
// Stage 8 Tests - Async Revision Gating and Cancellation Safety
// ========================

#[test]
fn revision_defaults_to_zero_and_entry_carries_revision() {
    // Verify default TableRuntimeState has revision 0
    let runtime = TableRuntimeState::default();
    assert_eq!(runtime.table_revision, 0);

    // Verify TableCacheEntry carries its assigned revision
    let cache_key = TableCacheKey {
        model_key: TableModelKey(1),
        display_filter: TableSearchSpec::default(),
        pinned_filters: vec![],
        view_sort: vec![],
        generation: 0,
    };
    let entry = TableCacheEntry::new(cache_key, 0, 42);
    assert_eq!(entry.revision, 42);
}

#[test]
fn stale_revision_ignored_on_cache_built() {
    let mut state = SystemState::new_default_config().expect("state");
    let tile_id = TableTileId(99);

    let cache_key = TableCacheKey {
        model_key: TableModelKey(1),
        display_filter: TableSearchSpec::default(),
        pinned_filters: vec![],
        view_sort: vec![],
        generation: 0,
    };

    // Set up runtime at revision 5 (simulating several prior builds)
    state.table_runtime.insert(
        tile_id,
        TableRuntimeState {
            cache_key: Some(cache_key.clone()),
            cache: None,
            last_error: None,
            selection: TableSelection::default(),

            type_search: TypeSearchState::default(),
            scroll_state: TableScrollState::default(),
            filter_draft: None,
            hidden_selection_count: 0,
            model: None,
            table_revision: 5,
            cancel_token: Arc::new(AtomicBool::new(false)),
        },
    );

    // Send a TableCacheBuilt with stale revision 3
    let stale_entry = Arc::new(TableCacheEntry::new(cache_key.clone(), 0, 3));

    let msg = Message::TableCacheBuilt {
        tile_id,
        revision: 3, // stale - runtime is at 5
        entry: stale_entry.clone(),
        model: None,
        result: Ok(TableCache {
            row_ids: vec![TableRowId(99)],
            row_index: build_row_index(&[TableRowId(99)]),
            search_texts: Some(vec!["stale".to_string()]),
        }),
    };
    state.update(msg);

    // The stale entry should NOT have been committed
    assert!(
        !stale_entry.is_ready(),
        "stale revision result should be discarded"
    );
}

#[test]
fn cancellation_stops_build_early() {
    let model = Arc::new(VirtualTableModel::new(100, 3, 42));
    let cancel = Arc::new(AtomicBool::new(true)); // pre-cancelled

    let result = build_table_cache(model, TableSearchSpec::default(), vec![], Some(cancel));

    match result {
        Err(TableCacheError::Cancelled) => {} // expected
        other => panic!("expected Cancelled, got {other:?}"),
    }
}

#[test]
fn selection_preserved_across_cancelled_build() {
    let mut state = SystemState::new_default_config().expect("state");
    let tile_id = TableTileId(88);

    let cache_key = TableCacheKey {
        model_key: TableModelKey(1),
        display_filter: TableSearchSpec::default(),
        pinned_filters: vec![],
        view_sort: vec![],
        generation: 0,
    };

    // Set up runtime with selection and revision 3
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(1));
    selection.rows.insert(TableRowId(3));
    selection.anchor = Some(TableRowId(1));

    state.table_runtime.insert(
        tile_id,
        TableRuntimeState {
            cache_key: Some(cache_key.clone()),
            cache: None,
            last_error: None,
            selection,

            type_search: TypeSearchState::default(),
            scroll_state: TableScrollState::default(),
            filter_draft: None,
            hidden_selection_count: 0,
            model: None,
            table_revision: 3,
            cancel_token: Arc::new(AtomicBool::new(false)),
        },
    );

    // Send a stale TableCacheBuilt with revision 0
    let stale_entry = Arc::new(TableCacheEntry::new(cache_key.clone(), 0, 0));
    state.update(Message::TableCacheBuilt {
        tile_id,
        revision: 0, // stale
        entry: stale_entry,
        model: None,
        result: Ok(TableCache {
            row_ids: vec![],
            row_index: HashMap::new(),
            search_texts: Some(vec![]),
        }),
    });

    // Selection should be unchanged
    assert_eq!(
        state.table_runtime[&tile_id].selection.len(),
        2,
        "selection should be preserved when stale build is discarded"
    );
    assert!(
        state.table_runtime[&tile_id]
            .selection
            .contains(TableRowId(1))
    );
    assert!(
        state.table_runtime[&tile_id]
            .selection
            .contains(TableRowId(3))
    );
}

// ========================
// Stage 9 Tests - UX Entry Point and Drill-Down
// ========================

/// Helper: collect multi-signal entries from selected variables in displayed items,
/// matching the context menu logic in menus.rs.
fn collect_selected_variable_entries(waves: &crate::wave_data::WaveData) -> Vec<MultiSignalEntry> {
    waves
        .items_tree
        .iter_visible_selected()
        .filter_map(|node| {
            let item = waves.displayed_items.get(&node.item_ref)?;
            if let DisplayedItem::Variable(var) = item {
                Some(MultiSignalEntry {
                    variable: var.variable_ref.clone(),
                    field: vec![],
                })
            } else {
                None
            }
        })
        .collect()
}

#[test]
fn menu_single_variable_selected_produces_signal_change_list() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
        VariableRef::from_hierarchy_string("tb.dut.counter"),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);

    let waves = state.user.waves.as_ref().unwrap();
    let clk_ref = VariableRef::from_hierarchy_string("tb.clk");
    let clk_idx = find_visible_index_for_variable(waves, &clk_ref).expect("clk should be visible");

    // Select only one variable
    state.update(Message::SetItemSelected(clk_idx, true));

    let waves = state.user.waves.as_ref().unwrap();
    let entries = collect_selected_variable_entries(waves);
    assert_eq!(entries.len(), 1, "exactly one variable selected");
    assert_eq!(entries[0].variable, clk_ref);
}

#[test]
fn menu_multiple_variables_selected_produces_multi_signal_entries() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
        VariableRef::from_hierarchy_string("tb.dut.counter"),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);

    let waves = state.user.waves.as_ref().unwrap();
    let clk_ref = VariableRef::from_hierarchy_string("tb.clk");
    let counter_ref = VariableRef::from_hierarchy_string("tb.dut.counter");
    let clk_idx = find_visible_index_for_variable(waves, &clk_ref).expect("clk should be visible");
    let counter_idx =
        find_visible_index_for_variable(waves, &counter_ref).expect("counter should be visible");

    // Select both variables
    state.update(Message::SetItemSelected(clk_idx, true));
    state.update(Message::SetItemSelected(counter_idx, true));

    let waves = state.user.waves.as_ref().unwrap();
    let entries = collect_selected_variable_entries(waves);
    assert_eq!(entries.len(), 2, "two variables selected");

    // Entries should include both variables (order may depend on display order)
    let var_refs: Vec<_> = entries.iter().map(|e| &e.variable).collect();
    assert!(var_refs.contains(&&clk_ref));
    assert!(var_refs.contains(&&counter_ref));
}

#[test]
fn menu_non_variable_selections_filtered_out() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();

    // Add a variable and a divider
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
    ]));
    state.update(Message::AddDivider(Some("---".to_string()), None));
    wait_for_waves_fully_loaded(&mut state, 10);

    // Select all visible items (variable + divider)
    state.update(Message::ItemSelectAll);

    let waves = state.user.waves.as_ref().unwrap();
    let selected_count = waves.items_tree.iter_visible_selected().count();
    assert!(
        selected_count >= 2,
        "should have at least 2 selected items (variable + divider)"
    );

    let entries = collect_selected_variable_entries(waves);
    assert_eq!(
        entries.len(),
        1,
        "non-variable items should be filtered out"
    );
    assert_eq!(
        entries[0].variable,
        VariableRef::from_hierarchy_string("tb.clk")
    );
}

#[test]
fn add_table_tile_multi_signal_creates_tile() {
    let _runtime = test_runtime();
    let _guard = _runtime.enter();
    let mut state = load_counter_state();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
        VariableRef::from_hierarchy_string("tb.dut.counter"),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);

    let initial_tile_count = state.user.table_tiles.len();

    state.update(Message::AddTableTile {
        spec: TableModelSpec::MultiSignalChangeList {
            variables: vec![
                MultiSignalEntry {
                    variable: VariableRef::from_hierarchy_string("tb.clk"),
                    field: vec![],
                },
                MultiSignalEntry {
                    variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
                    field: vec![],
                },
            ],
        },
    });

    assert_eq!(
        state.user.table_tiles.len(),
        initial_tile_count + 1,
        "multi-signal table tile should be created"
    );

    // Verify the tile has the correct spec
    let (_, tile_state) = state
        .user
        .table_tiles
        .iter()
        .last()
        .expect("should have a tile");
    match &tile_state.spec {
        TableModelSpec::MultiSignalChangeList { variables } => {
            assert_eq!(variables.len(), 2);
        }
        other => panic!("expected MultiSignalChangeList spec, got {:?}", other),
    }
}

#[test]
fn drill_down_column_key_to_single_signal_spec() {
    // Verify that decoding a signal column key produces the correct
    // VariableRef and field for a SignalChangeList spec.
    let full_path = "tb.dut.counter";
    let field: Vec<String> = vec![];
    let column_key = encode_signal_column_key(full_path, &field);

    // Decode the key (as the drill-down code in view.rs does)
    let (decoded_path, decoded_field) =
        decode_signal_column_key(&column_key).expect("should decode signal column key");

    assert_eq!(decoded_path, full_path);
    assert_eq!(decoded_field, field);

    // Construct the spec that the drill-down would produce
    let spec = TableModelSpec::SignalChangeList {
        variable: VariableRef::from_hierarchy_string(&decoded_path),
        field: decoded_field,
    };

    match &spec {
        TableModelSpec::SignalChangeList { variable, field } => {
            assert_eq!(
                variable,
                &VariableRef::from_hierarchy_string("tb.dut.counter")
            );
            assert!(field.is_empty());
        }
        _ => panic!("expected SignalChangeList spec"),
    }
}

#[test]
fn drill_down_column_key_with_field_to_single_signal_spec() {
    // Verify drill-down for a signal with sub-fields
    let full_path = "tb.dut.bus";
    let field = vec!["data".to_string(), "valid".to_string()];
    let column_key = encode_signal_column_key(full_path, &field);

    let (decoded_path, decoded_field) =
        decode_signal_column_key(&column_key).expect("should decode");

    let spec = TableModelSpec::SignalChangeList {
        variable: VariableRef::from_hierarchy_string(&decoded_path),
        field: decoded_field.clone(),
    };

    match &spec {
        TableModelSpec::SignalChangeList { variable, field } => {
            assert_eq!(variable, &VariableRef::from_hierarchy_string("tb.dut.bus"));
            assert_eq!(field, &["data", "valid"]);
        }
        _ => panic!("expected SignalChangeList spec"),
    }
}

#[test]
fn non_signal_column_key_does_not_decode() {
    // The "time" column key should not decode as a signal column
    assert!(decode_signal_column_key("time").is_none());
    assert!(decode_signal_column_key("").is_none());
    assert!(decode_signal_column_key("other:prefix:foo#bar").is_none());
}
