use super::support::build_row_index;
use super::*;

// ========================
// Stage 10 Tests - Scroll Target Computation
// ========================

#[test]
fn scroll_target_after_sort_with_selection_finds_row() {
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(5));
    selection.anchor = Some(TableRowId(5));

    // New sorted order puts row 5 at index 2
    let new_visible = vec![
        TableRowId(3),
        TableRowId(1),
        TableRowId(5),
        TableRowId(0),
        TableRowId(2),
    ];

    let result = scroll_target_after_sort(&selection, &new_visible, &build_row_index(&new_visible));
    assert_eq!(result, ScrollTarget::ToRow(TableRowId(5)));
}

#[test]
fn scroll_target_after_sort_selection_at_top() {
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(0));
    selection.anchor = Some(TableRowId(0));

    let new_visible = vec![TableRowId(0), TableRowId(1), TableRowId(2)];

    let result = scroll_target_after_sort(&selection, &new_visible, &build_row_index(&new_visible));
    assert_eq!(result, ScrollTarget::ToRow(TableRowId(0)));
}

#[test]
fn scroll_target_after_sort_selection_at_bottom() {
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(2));
    selection.anchor = Some(TableRowId(2));

    let new_visible = vec![TableRowId(0), TableRowId(1), TableRowId(2)];

    let result = scroll_target_after_sort(&selection, &new_visible, &build_row_index(&new_visible));
    assert_eq!(result, ScrollTarget::ToRow(TableRowId(2)));
}

#[test]
fn scroll_target_after_sort_no_selection_preserves() {
    let selection = TableSelection::new();
    let new_visible = vec![TableRowId(0), TableRowId(1), TableRowId(2)];

    let result = scroll_target_after_sort(&selection, &new_visible, &build_row_index(&new_visible));
    assert_eq!(result, ScrollTarget::Preserve);
}

#[test]
fn scroll_target_after_sort_multi_selection_uses_first() {
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(5));
    selection.rows.insert(TableRowId(3));
    selection.rows.insert(TableRowId(7));
    selection.anchor = Some(TableRowId(5));

    // New order puts row 3 first among selected
    let new_visible = vec![
        TableRowId(1),
        TableRowId(3), // First selected in display order
        TableRowId(5),
        TableRowId(7),
        TableRowId(9),
    ];

    let result = scroll_target_after_sort(&selection, &new_visible, &build_row_index(&new_visible));
    assert_eq!(result, ScrollTarget::ToRow(TableRowId(3)));
}

#[test]
fn scroll_target_after_filter_selected_row_visible() {
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(2));
    selection.anchor = Some(TableRowId(2));

    let new_visible = vec![TableRowId(0), TableRowId(2), TableRowId(4)];

    let result =
        scroll_target_after_filter(&selection, &new_visible, &build_row_index(&new_visible));
    assert_eq!(result, ScrollTarget::ToRow(TableRowId(2)));
}

#[test]
fn scroll_target_after_filter_selected_row_hidden() {
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(3)); // Row 3 is filtered out
    selection.anchor = Some(TableRowId(3));

    let new_visible = vec![TableRowId(0), TableRowId(2), TableRowId(4)]; // Row 3 not in list

    let result =
        scroll_target_after_filter(&selection, &new_visible, &build_row_index(&new_visible));
    assert_eq!(result, ScrollTarget::ToTop);
}

#[test]
fn scroll_target_after_filter_no_selection() {
    let selection = TableSelection::new();
    let new_visible = vec![TableRowId(0), TableRowId(1)];

    let result =
        scroll_target_after_filter(&selection, &new_visible, &build_row_index(&new_visible));
    assert_eq!(result, ScrollTarget::Preserve);
}

#[test]
fn scroll_target_after_filter_all_selected_hidden() {
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(3));
    selection.rows.insert(TableRowId(5));

    let new_visible = vec![TableRowId(0), TableRowId(2), TableRowId(4)];

    let result =
        scroll_target_after_filter(&selection, &new_visible, &build_row_index(&new_visible));
    assert_eq!(result, ScrollTarget::ToTop);
}

#[test]
fn scroll_target_after_activation_returns_to_row() {
    let result = scroll_target_after_activation(TableRowId(42));
    assert_eq!(result, ScrollTarget::ToRow(TableRowId(42)));
}

// ========================
// Stage 10 Tests - Column Resize
// ========================

#[test]
fn resize_column_updates_width() {
    let columns = vec![TableColumnConfig {
        key: TableColumnKey::Str("col_0".to_string()),
        width: Some(100.0),
        visible: true,
        resizable: true,
    }];

    let result = resize_column(
        &columns,
        &TableColumnKey::Str("col_0".to_string()),
        150.0,
        MIN_COLUMN_WIDTH,
    );

    assert!(result.changed);
    assert_eq!(result.columns[0].width, Some(150.0));
}

#[test]
fn resize_column_respects_min_width() {
    let columns = vec![TableColumnConfig {
        key: TableColumnKey::Str("col_0".to_string()),
        width: Some(100.0),
        visible: true,
        resizable: true,
    }];

    let result = resize_column(
        &columns,
        &TableColumnKey::Str("col_0".to_string()),
        5.0, // Below minimum
        MIN_COLUMN_WIDTH,
    );

    assert!(result.changed);
    assert_eq!(result.columns[0].width, Some(MIN_COLUMN_WIDTH));
}

#[test]
fn resize_column_unknown_key_no_change() {
    let columns = vec![TableColumnConfig {
        key: TableColumnKey::Str("col_0".to_string()),
        width: Some(100.0),
        visible: true,
        resizable: true,
    }];

    let result = resize_column(
        &columns,
        &TableColumnKey::Str("unknown".to_string()),
        150.0,
        MIN_COLUMN_WIDTH,
    );

    assert!(!result.changed);
    assert_eq!(result.columns[0].width, Some(100.0));
}

#[test]
fn resize_column_preserves_other_columns() {
    let columns = vec![
        TableColumnConfig {
            key: TableColumnKey::Str("col_0".to_string()),
            width: Some(100.0),
            visible: true,
            resizable: true,
        },
        TableColumnConfig {
            key: TableColumnKey::Str("col_1".to_string()),
            width: Some(80.0),
            visible: true,
            resizable: true,
        },
    ];

    let result = resize_column(
        &columns,
        &TableColumnKey::Str("col_0".to_string()),
        150.0,
        MIN_COLUMN_WIDTH,
    );

    assert_eq!(result.columns[1].width, Some(80.0));
}

#[test]
fn resize_column_zero_width_uses_min() {
    let columns = vec![TableColumnConfig {
        key: TableColumnKey::Str("col_0".to_string()),
        width: Some(100.0),
        visible: true,
        resizable: true,
    }];

    let result = resize_column(
        &columns,
        &TableColumnKey::Str("col_0".to_string()),
        0.0,
        MIN_COLUMN_WIDTH,
    );

    assert!(result.changed);
    assert_eq!(result.columns[0].width, Some(MIN_COLUMN_WIDTH));
}

#[test]
fn resize_column_negative_width_uses_min() {
    let columns = vec![TableColumnConfig {
        key: TableColumnKey::Str("col_0".to_string()),
        width: Some(100.0),
        visible: true,
        resizable: true,
    }];

    let result = resize_column(
        &columns,
        &TableColumnKey::Str("col_0".to_string()),
        -50.0,
        MIN_COLUMN_WIDTH,
    );

    assert!(result.changed);
    assert_eq!(result.columns[0].width, Some(MIN_COLUMN_WIDTH));
}

#[test]
fn resize_column_same_width_no_change() {
    let columns = vec![TableColumnConfig {
        key: TableColumnKey::Str("col_0".to_string()),
        width: Some(100.0),
        visible: true,
        resizable: true,
    }];

    let result = resize_column(
        &columns,
        &TableColumnKey::Str("col_0".to_string()),
        100.0,
        MIN_COLUMN_WIDTH,
    );

    assert!(!result.changed);
}

#[test]
fn resize_column_float_precision() {
    let columns = vec![TableColumnConfig {
        key: TableColumnKey::Str("col_0".to_string()),
        width: Some(100.0),
        visible: true,
        resizable: true,
    }];

    // Small change should not register as a change (within 0.1 tolerance)
    let result = resize_column(
        &columns,
        &TableColumnKey::Str("col_0".to_string()),
        100.05,
        MIN_COLUMN_WIDTH,
    );

    assert!(!result.changed);
}

// ========================
// Stage 10 Tests - Column Visibility
// ========================

#[test]
fn toggle_column_visibility_hides_visible() {
    let columns = vec![
        TableColumnConfig {
            key: TableColumnKey::Str("col_0".to_string()),
            width: None,
            visible: true,
            resizable: true,
        },
        TableColumnConfig {
            key: TableColumnKey::Str("col_1".to_string()),
            width: None,
            visible: true,
            resizable: true,
        },
    ];

    let result = toggle_column_visibility(&columns, &TableColumnKey::Str("col_0".to_string()));

    assert!(!result[0].visible);
    assert!(result[1].visible);
}

#[test]
fn toggle_column_visibility_shows_hidden() {
    let columns = vec![
        TableColumnConfig {
            key: TableColumnKey::Str("col_0".to_string()),
            width: None,
            visible: false,
            resizable: true,
        },
        TableColumnConfig {
            key: TableColumnKey::Str("col_1".to_string()),
            width: None,
            visible: true,
            resizable: true,
        },
    ];

    let result = toggle_column_visibility(&columns, &TableColumnKey::Str("col_0".to_string()));

    assert!(result[0].visible);
}

#[test]
fn toggle_column_visibility_unknown_key() {
    let columns = vec![TableColumnConfig {
        key: TableColumnKey::Str("col_0".to_string()),
        width: None,
        visible: true,
        resizable: true,
    }];

    let result = toggle_column_visibility(&columns, &TableColumnKey::Str("unknown".to_string()));

    // No change
    assert!(result[0].visible);
}

#[test]
fn visible_columns_returns_ordered_list() {
    let columns = vec![
        TableColumnConfig {
            key: TableColumnKey::Str("col_0".to_string()),
            width: None,
            visible: true,
            resizable: true,
        },
        TableColumnConfig {
            key: TableColumnKey::Str("col_1".to_string()),
            width: None,
            visible: true,
            resizable: true,
        },
        TableColumnConfig {
            key: TableColumnKey::Str("col_2".to_string()),
            width: None,
            visible: true,
            resizable: true,
        },
    ];

    let result = visible_columns(&columns);

    assert_eq!(result.len(), 3);
    assert_eq!(result[0], TableColumnKey::Str("col_0".to_string()));
    assert_eq!(result[1], TableColumnKey::Str("col_1".to_string()));
    assert_eq!(result[2], TableColumnKey::Str("col_2".to_string()));
}

#[test]
fn visible_columns_excludes_hidden() {
    let columns = vec![
        TableColumnConfig {
            key: TableColumnKey::Str("col_0".to_string()),
            width: None,
            visible: true,
            resizable: true,
        },
        TableColumnConfig {
            key: TableColumnKey::Str("col_1".to_string()),
            width: None,
            visible: false,
            resizable: true,
        },
        TableColumnConfig {
            key: TableColumnKey::Str("col_2".to_string()),
            width: None,
            visible: true,
            resizable: true,
        },
    ];

    let result = visible_columns(&columns);

    assert_eq!(result.len(), 2);
    assert_eq!(result[0], TableColumnKey::Str("col_0".to_string()));
    assert_eq!(result[1], TableColumnKey::Str("col_2".to_string()));
}

#[test]
fn hidden_columns_returns_hidden_only() {
    let columns = vec![
        TableColumnConfig {
            key: TableColumnKey::Str("col_0".to_string()),
            width: None,
            visible: true,
            resizable: true,
        },
        TableColumnConfig {
            key: TableColumnKey::Str("col_1".to_string()),
            width: None,
            visible: false,
            resizable: true,
        },
    ];

    let result = hidden_columns(&columns);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0], TableColumnKey::Str("col_1".to_string()));
}

#[test]
fn visible_columns_empty_config() {
    let columns: Vec<TableColumnConfig> = vec![];
    let result = visible_columns(&columns);
    assert!(result.is_empty());
}

#[test]
fn toggle_last_visible_column_stays_visible() {
    let columns = vec![
        TableColumnConfig {
            key: TableColumnKey::Str("col_0".to_string()),
            width: None,
            visible: true,
            resizable: true,
        },
        TableColumnConfig {
            key: TableColumnKey::Str("col_1".to_string()),
            width: None,
            visible: false,
            resizable: true,
        },
    ];

    // Try to hide the last visible column - should not hide it
    let result = toggle_column_visibility(&columns, &TableColumnKey::Str("col_0".to_string()));

    assert!(result[0].visible); // Should still be visible
}

// ========================
// Stage 10 Tests - Generation Tracking
// ========================

#[test]
fn generation_change_triggers_clear() {
    assert!(should_clear_selection_on_generation_change(2, 1));
}

#[test]
fn generation_same_no_clear() {
    assert!(!should_clear_selection_on_generation_change(5, 5));
}

#[test]
fn generation_zero_to_nonzero_clears() {
    assert!(should_clear_selection_on_generation_change(1, 0));
}

#[test]
fn generation_rollover_handled() {
    // Test that even u64::MAX to 0 detects a change
    assert!(should_clear_selection_on_generation_change(0, u64::MAX));
}

// ========================
// Stage 10 Tests - TableScrollState
// ========================

#[test]
fn scroll_state_default_no_target() {
    let state = TableScrollState::default();
    assert!(state.scroll_target.is_none());
    assert!(state.pending_scroll_op.is_none());
}

#[test]
fn scroll_state_set_target() {
    let mut state = TableScrollState::default();
    state.set_scroll_target(ScrollTarget::ToTop);
    assert_eq!(state.scroll_target, Some(ScrollTarget::ToTop));
}

#[test]
fn scroll_state_take_target_consumes() {
    let mut state = TableScrollState::default();
    state.set_scroll_target(ScrollTarget::ToRow(TableRowId(5)));

    let taken = state.take_scroll_target();
    assert_eq!(taken, Some(ScrollTarget::ToRow(TableRowId(5))));
    assert!(state.scroll_target.is_none());
}

#[test]
fn scroll_state_take_empty_returns_none() {
    let mut state = TableScrollState::default();
    let taken = state.take_scroll_target();
    assert!(taken.is_none());
}

#[test]
fn scroll_state_set_overwrites_previous() {
    let mut state = TableScrollState::default();
    state.set_scroll_target(ScrollTarget::ToTop);
    state.set_scroll_target(ScrollTarget::ToBottom);
    assert_eq!(state.scroll_target, Some(ScrollTarget::ToBottom));
}

// ========================
// Stage 10 Tests - Integration: Column Resize Messages
// ========================

#[test]
fn resize_column_message_updates_config() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 3,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Set up column config
    state
        .user
        .table_tiles
        .get_mut(&tile_id)
        .unwrap()
        .config
        .columns = vec![TableColumnConfig {
        key: TableColumnKey::Str("col_0".to_string()),
        width: Some(100.0),
        visible: true,
        resizable: true,
    }];

    state.update(Message::ResizeTableColumn {
        tile_id,
        column_key: TableColumnKey::Str("col_0".to_string()),
        new_width: 150.0,
    });

    let tile_state = state.user.table_tiles.get(&tile_id).unwrap();
    assert_eq!(tile_state.config.columns[0].width, Some(150.0));
}

#[test]
fn resize_column_nonexistent_tile_ignored() {
    let mut state = SystemState::new_default_config().expect("state");

    // Should not panic
    state.update(Message::ResizeTableColumn {
        tile_id: TableTileId(9999),
        column_key: TableColumnKey::Str("col_0".to_string()),
        new_width: 150.0,
    });
}

#[test]
fn resize_column_nonexistent_column_ignored() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 3,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Set up column config
    state
        .user
        .table_tiles
        .get_mut(&tile_id)
        .unwrap()
        .config
        .columns = vec![TableColumnConfig {
        key: TableColumnKey::Str("col_0".to_string()),
        width: Some(100.0),
        visible: true,
        resizable: true,
    }];

    state.update(Message::ResizeTableColumn {
        tile_id,
        column_key: TableColumnKey::Str("unknown".to_string()),
        new_width: 150.0,
    });

    // Original column unchanged
    let tile_state = state.user.table_tiles.get(&tile_id).unwrap();
    assert_eq!(tile_state.config.columns[0].width, Some(100.0));
}

// ========================
// Stage 10 Tests - Integration: Column Visibility Messages
// ========================

#[test]
fn toggle_visibility_message_updates_config() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 3,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Set up column config with two visible columns
    state
        .user
        .table_tiles
        .get_mut(&tile_id)
        .unwrap()
        .config
        .columns = vec![
        TableColumnConfig {
            key: TableColumnKey::Str("col_0".to_string()),
            width: None,
            visible: true,
            resizable: true,
        },
        TableColumnConfig {
            key: TableColumnKey::Str("col_1".to_string()),
            width: None,
            visible: true,
            resizable: true,
        },
    ];

    state.update(Message::ToggleTableColumnVisibility {
        tile_id,
        column_key: TableColumnKey::Str("col_0".to_string()),
    });

    let tile_state = state.user.table_tiles.get(&tile_id).unwrap();
    assert!(!tile_state.config.columns[0].visible);
    assert!(tile_state.config.columns[1].visible);
}

#[test]
fn toggle_visibility_message_initializes_empty_columns_from_model_schema() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 3,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");
    assert!(
        state
            .user
            .table_tiles
            .get(&tile_id)
            .is_some_and(|tile_state| tile_state.config.columns.is_empty()),
        "default table config should start with empty columns"
    );

    // Seed runtime model as it would be after first cache build/render.
    state.table_runtime.entry(tile_id).or_default().model =
        Some(Arc::new(VirtualTableModel::new(10, 3, 42)));

    state.update(Message::ToggleTableColumnVisibility {
        tile_id,
        column_key: TableColumnKey::Str("col_1".to_string()),
    });

    let tile_state = state.user.table_tiles.get(&tile_id).expect("tile exists");
    assert_eq!(tile_state.config.columns.len(), 3);
    assert!(
        tile_state
            .config
            .columns
            .iter()
            .find(|col| col.key == TableColumnKey::Str("col_1".to_string()))
            .is_some_and(|col| !col.visible),
        "target column should be hidden after toggle"
    );
}

#[test]
fn set_column_visibility_bulk_update() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 3,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Set up column config
    state
        .user
        .table_tiles
        .get_mut(&tile_id)
        .unwrap()
        .config
        .columns = vec![
        TableColumnConfig {
            key: TableColumnKey::Str("col_0".to_string()),
            width: None,
            visible: true,
            resizable: true,
        },
        TableColumnConfig {
            key: TableColumnKey::Str("col_1".to_string()),
            width: None,
            visible: true,
            resizable: true,
        },
        TableColumnConfig {
            key: TableColumnKey::Str("col_2".to_string()),
            width: None,
            visible: true,
            resizable: true,
        },
    ];

    // Set only col_1 visible
    state.update(Message::SetTableColumnVisibility {
        tile_id,
        visible_columns: vec![TableColumnKey::Str("col_1".to_string())],
    });

    let tile_state = state.user.table_tiles.get(&tile_id).unwrap();
    assert!(!tile_state.config.columns[0].visible);
    assert!(tile_state.config.columns[1].visible);
    assert!(!tile_state.config.columns[2].visible);
}

#[test]
fn set_column_visibility_initializes_empty_columns_from_model_schema() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 3,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");
    assert!(
        state
            .user
            .table_tiles
            .get(&tile_id)
            .is_some_and(|tile_state| tile_state.config.columns.is_empty()),
        "default table config should start with empty columns"
    );

    state.table_runtime.entry(tile_id).or_default().model =
        Some(Arc::new(VirtualTableModel::new(10, 3, 42)));

    state.update(Message::SetTableColumnVisibility {
        tile_id,
        visible_columns: vec![TableColumnKey::Str("col_2".to_string())],
    });

    let tile_state = state.user.table_tiles.get(&tile_id).expect("tile exists");
    assert_eq!(tile_state.config.columns.len(), 3);
    assert!(
        tile_state
            .config
            .columns
            .iter()
            .find(|col| col.key == TableColumnKey::Str("col_2".to_string()))
            .is_some_and(|col| col.visible),
        "requested visible column should be visible"
    );
    assert!(
        tile_state
            .config
            .columns
            .iter()
            .filter(|col| col.visible)
            .count()
            == 1,
        "all non-requested columns should be hidden"
    );
}

// ========================
// Stage 10 Tests - Integration: Scroll Behavior
// ========================

#[test]
fn sort_change_sets_pending_scroll_op() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 3,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Initialize runtime
    state
        .table_runtime
        .entry(tile_id)
        .or_insert_with(TableRuntimeState::default);

    state.update(Message::SetTableSort {
        tile_id,
        sort: vec![TableSortSpec {
            key: TableColumnKey::Str("col_0".to_string()),
            direction: TableSortDirection::Ascending,
        }],
    });

    let runtime = state.table_runtime.get(&tile_id).unwrap();
    assert_eq!(
        runtime.scroll_state.pending_scroll_op,
        Some(PendingScrollOp::AfterSort)
    );
}

#[test]
fn filter_change_sets_pending_scroll_op() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 3,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Initialize runtime
    state
        .table_runtime
        .entry(tile_id)
        .or_insert_with(TableRuntimeState::default);

    state.update(Message::SetTableDisplayFilter {
        tile_id,
        filter: TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: "test".to_string(),
            column: None,
        },
    });

    let runtime = state.table_runtime.get(&tile_id).unwrap();
    assert_eq!(
        runtime.scroll_state.pending_scroll_op,
        Some(PendingScrollOp::AfterFilter)
    );
}
