use super::*;
use crate::SystemState;
use crate::message::Message;
use crate::table::sources::VirtualTableModel;
use crate::wave_container::VariableRefExt;
use std::sync::Arc;

// ========================
// Stage 1 Tests
// ========================

#[test]
fn table_ids_round_trip() {
    let tile_id = TableTileId(42);
    let row_id = TableRowId(9001);

    let tile_encoded = ron::ser::to_string(&tile_id).expect("serialize TableTileId");
    let row_encoded = ron::ser::to_string(&row_id).expect("serialize TableRowId");

    let tile_decoded: TableTileId =
        ron::de::from_str(&tile_encoded).expect("deserialize TableTileId");
    let row_decoded: TableRowId = ron::de::from_str(&row_encoded).expect("deserialize TableRowId");

    assert_eq!(tile_id, tile_decoded);
    assert_eq!(row_id, row_decoded);
}

#[test]
fn table_model_spec_virtual_ron_format() {
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 3,
        seed: 42,
    };

    let encoded = ron::ser::to_string(&spec).expect("serialize TableModelSpec::Virtual");
    let normalized: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();

    assert_eq!(normalized, "Virtual(rows:10,columns:3,seed:42)");
}

#[test]
fn table_view_config_round_trip() {
    let config = TableViewConfig {
        title: "Example".to_string(),
        columns: vec![
            TableColumnConfig {
                key: TableColumnKey::Str("col_0".to_string()),
                width: Some(120.0),
                visible: true,
                resizable: true,
            },
            TableColumnConfig {
                key: TableColumnKey::Id(1),
                width: None,
                visible: false,
                resizable: false,
            },
        ],
        sort: vec![TableSortSpec {
            key: TableColumnKey::Str("col_0".to_string()),
            direction: TableSortDirection::Ascending,
        }],
        display_filter: TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: "needle".to_string(),
        },
        selection_mode: TableSelectionMode::Multi,
        dense_rows: true,
        sticky_header: false,
    };

    let encoded = ron::ser::to_string(&config).expect("serialize TableViewConfig");
    let decoded: TableViewConfig =
        ron::de::from_str(&encoded).expect("deserialize TableViewConfig");

    assert_eq!(config, decoded);
}

// ========================
// Stage 2 Tests
// ========================

#[test]
fn table_model_spec_create_virtual_model() {
    let spec = TableModelSpec::Virtual {
        rows: 100,
        columns: 5,
        seed: 42,
    };

    let model = spec.create_model();
    assert!(model.is_some(), "Virtual model should be created");

    let model = model.unwrap();
    assert_eq!(model.row_count(), 100);
    assert_eq!(model.schema().columns.len(), 5);
}

#[test]
fn table_model_spec_create_unimplemented_returns_none() {
    let signal_spec = TableModelSpec::SignalChangeList {
        variable: crate::wave_container::VariableRef::from_hierarchy_string(""),
        field: vec![],
    };
    assert!(
        signal_spec.create_model().is_none(),
        "SignalChangeList not yet implemented"
    );

    let custom_spec = TableModelSpec::Custom {
        key: "test".to_string(),
        payload: "{}".to_string(),
    };
    assert!(
        custom_spec.create_model().is_none(),
        "Custom not yet implemented"
    );
}

#[test]
fn virtual_model_via_factory_deterministic() {
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 3,
        seed: 42,
    };

    let model1 = spec.create_model().unwrap();
    let model2 = spec.create_model().unwrap();

    // Both models should produce identical output
    for row_idx in 0..10 {
        let row = TableRowId(row_idx as u64);
        for col in 0..3 {
            let cell1 = model1.cell(row, col);
            let cell2 = model2.cell(row, col);

            let text1 = match cell1 {
                TableCell::Text(s) => s,
                _ => panic!("Expected Text cell"),
            };
            let text2 = match cell2 {
                TableCell::Text(s) => s,
                _ => panic!("Expected Text cell"),
            };

            assert_eq!(text1, text2);
        }
    }
}

#[test]
fn virtual_model_schema_keys_match_expected_format() {
    let spec = TableModelSpec::Virtual {
        rows: 5,
        columns: 3,
        seed: 0,
    };

    let model = spec.create_model().unwrap();
    let schema = model.schema();

    // Verify keys are "col_0", "col_1", "col_2"
    let keys: Vec<_> = schema.columns.iter().map(|c| &c.key).collect();
    assert_eq!(keys[0], &TableColumnKey::Str("col_0".to_string()));
    assert_eq!(keys[1], &TableColumnKey::Str("col_1".to_string()));
    assert_eq!(keys[2], &TableColumnKey::Str("col_2".to_string()));

    // Verify labels are "Col 0", "Col 1", "Col 2"
    let labels: Vec<_> = schema.columns.iter().map(|c| &c.label).collect();
    assert_eq!(labels[0], "Col 0");
    assert_eq!(labels[1], "Col 1");
    assert_eq!(labels[2], "Col 2");
}

// ========================
// Stage 3 Tests
// ========================

#[test]
fn table_cache_entry_ready_state() {
    let cache_key = TableCacheKey {
        model_key: TableModelKey(1),
        display_filter: TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: String::new(),
        },
        view_sort: vec![],
        generation: 0,
    };
    let entry = TableCacheEntry::new(cache_key, 0);
    assert!(!entry.is_ready());

    entry.set(TableCache {
        row_ids: vec![],
        search_texts: vec![],
        sort_keys: vec![],
    });
    assert!(entry.is_ready());
}

#[test]
fn table_cache_builder_unfiltered_unsorted() {
    let model = Arc::new(VirtualTableModel::new(5, 2, 0));
    let cache = build_table_cache(
        model,
        TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: String::new(),
        },
        vec![],
    )
    .expect("cache build should succeed");

    let expected: Vec<_> = (0..5).map(|idx| TableRowId(idx as u64)).collect();
    assert_eq!(cache.row_ids, expected);
    assert_eq!(cache.search_texts.len(), expected.len());
    assert_eq!(cache.sort_keys.len(), expected.len());
}

#[test]
fn table_cache_builder_filters_contains() {
    let model = Arc::new(VirtualTableModel::new(10, 2, 0));
    let cache = build_table_cache(
        model,
        TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: "r3c0".to_string(),
        },
        vec![],
    )
    .expect("cache build should succeed");

    assert_eq!(cache.row_ids, vec![TableRowId(3)]);
}

#[test]
fn table_cache_builder_sorts_rows() {
    #[derive(Clone)]
    struct TestModel {
        rows: Vec<(TableRowId, f64, String)>,
    }

    impl TableModel for TestModel {
        fn schema(&self) -> TableSchema {
            TableSchema {
                columns: vec![TableColumn {
                    key: TableColumnKey::Str("col".to_string()),
                    label: "Col".to_string(),
                    default_width: None,
                    default_visible: true,
                    default_resizable: true,
                }],
            }
        }

        fn row_count(&self) -> usize {
            self.rows.len()
        }

        fn row_id_at(&self, index: usize) -> Option<TableRowId> {
            self.rows.get(index).map(|(id, _, _)| *id)
        }

        fn cell(&self, row: TableRowId, _col: usize) -> TableCell {
            let text = self
                .rows
                .iter()
                .find(|(id, _, _)| *id == row)
                .map(|(_, _, text)| text.clone())
                .unwrap_or_default();
            TableCell::Text(text)
        }

        fn sort_key(&self, row: TableRowId, _col: usize) -> TableSortKey {
            self.rows
                .iter()
                .find(|(id, _, _)| *id == row)
                .map(|(_, value, _)| TableSortKey::Numeric(*value))
                .unwrap_or(TableSortKey::None)
        }

        fn search_text(&self, row: TableRowId) -> String {
            self.rows
                .iter()
                .find(|(id, _, _)| *id == row)
                .map(|(_, _, text)| text.clone())
                .unwrap_or_default()
        }

        fn on_activate(&self, _row: TableRowId) -> TableAction {
            TableAction::None
        }
    }

    let model = Arc::new(TestModel {
        rows: vec![
            (TableRowId(0), 5.0, "alpha".to_string()),
            (TableRowId(1), 1.0, "beta".to_string()),
            (TableRowId(2), 3.0, "gamma".to_string()),
        ],
    });

    let cache = build_table_cache(
        model.clone(),
        TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: String::new(),
        },
        vec![TableSortSpec {
            key: TableColumnKey::Str("col".to_string()),
            direction: TableSortDirection::Ascending,
        }],
    )
    .expect("cache build should succeed");

    assert_eq!(
        cache.row_ids,
        vec![TableRowId(1), TableRowId(2), TableRowId(0)]
    );

    let cache_desc = build_table_cache(
        model,
        TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: String::new(),
        },
        vec![TableSortSpec {
            key: TableColumnKey::Str("col".to_string()),
            direction: TableSortDirection::Descending,
        }],
    )
    .expect("cache build should succeed");

    assert_eq!(
        cache_desc.row_ids,
        vec![TableRowId(0), TableRowId(2), TableRowId(1)]
    );
}

#[test]
fn table_cache_builder_empty_result() {
    let model = Arc::new(VirtualTableModel::new(5, 2, 0));
    let cache = build_table_cache(
        model,
        TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: true,
            text: "nope".to_string(),
        },
        vec![],
    )
    .expect("cache build should succeed");

    assert!(cache.row_ids.is_empty());
    assert!(cache.search_texts.is_empty());
    assert!(cache.sort_keys.is_empty());
}

#[test]
fn table_cache_builder_invalid_regex() {
    let model = Arc::new(VirtualTableModel::new(5, 2, 0));
    let result = build_table_cache(
        model,
        TableSearchSpec {
            mode: TableSearchMode::Regex,
            case_sensitive: false,
            text: "(".to_string(),
        },
        vec![],
    );

    match result {
        Err(TableCacheError::InvalidSearch { pattern, .. }) => {
            assert_eq!(pattern, "(");
        }
        other => panic!("Expected invalid regex error, got {other:?}"),
    }
}

#[test]
fn table_cache_built_stale_key_ignored() {
    let mut state = SystemState::new_default_config().expect("state");
    let tile_id = TableTileId(1);
    let old_key = TableCacheKey {
        model_key: TableModelKey(1),
        display_filter: TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: String::new(),
        },
        view_sort: vec![],
        generation: 1,
    };
    let new_key = TableCacheKey {
        model_key: TableModelKey(1),
        display_filter: TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: "new".to_string(),
        },
        view_sort: vec![],
        generation: 1,
    };

    state.table_runtime.insert(
        tile_id,
        TableRuntimeState {
            cache_key: Some(new_key),
            cache: None,
            last_error: None,
            selection: TableSelection::default(),
            scroll_offset: 0.0,
        },
    );

    let entry = Arc::new(TableCacheEntry::new(old_key.clone(), old_key.generation));
    state.table_inflight.insert(old_key, entry.clone());

    let msg = Message::TableCacheBuilt {
        tile_id,
        entry: entry.clone(),
        result: Ok(TableCache {
            row_ids: vec![],
            search_texts: vec![],
            sort_keys: vec![],
        }),
    };

    state.update(msg);
    assert!(!entry.is_ready(), "stale cache should be ignored");
}

// ========================
// Stage 4 Tests
// ========================

#[test]
fn add_table_tile_creates_tile_in_tree() {
    let mut state = SystemState::new_default_config().expect("state");

    // Initially no table tiles
    assert!(state.user.table_tiles.is_empty());

    // Add a table tile via message
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 3,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec });

    // Verify table tile was created
    assert_eq!(state.user.table_tiles.len(), 1);

    // Verify the tile tree contains the table pane
    let has_table_pane = state.user.tile_tree.tree.tiles.iter().any(|(_, tile)| {
        matches!(
            tile,
            egui_tiles::Tile::Pane(crate::tiles::SurferPane::Table(_))
        )
    });
    assert!(has_table_pane, "Tile tree should contain a Table pane");
}

#[test]
fn remove_table_tile_cleans_up_state() {
    let mut state = SystemState::new_default_config().expect("state");

    // Add a table tile
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 3,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec });

    // Get the tile ID that was created
    let tile_id = *state
        .user
        .table_tiles
        .keys()
        .next()
        .expect("should have one tile");

    // Add some runtime state
    state
        .table_runtime
        .insert(tile_id, TableRuntimeState::default());

    // Remove the tile
    state.update(Message::RemoveTableTile { tile_id });

    // Verify both table_tiles and table_runtime were cleaned up
    assert!(
        state.user.table_tiles.is_empty(),
        "table_tiles should be empty after removal"
    );
    assert!(
        state.table_runtime.is_empty(),
        "table_runtime should be empty after removal"
    );
}

#[test]
fn table_tile_state_serialization_round_trip() {
    let tile_state = TableTileState {
        spec: TableModelSpec::Virtual {
            rows: 100,
            columns: 5,
            seed: 123,
        },
        config: TableViewConfig {
            title: "Test Table".to_string(),
            columns: vec![],
            sort: vec![],
            display_filter: TableSearchSpec::default(),
            selection_mode: TableSelectionMode::Multi,
            dense_rows: true,
            sticky_header: false,
        },
    };

    let encoded = ron::ser::to_string(&tile_state).expect("serialize TableTileState");
    let decoded: TableTileState = ron::de::from_str(&encoded).expect("deserialize TableTileState");

    assert_eq!(tile_state.spec, decoded.spec);
    assert_eq!(tile_state.config.title, decoded.config.title);
    assert_eq!(
        tile_state.config.selection_mode,
        decoded.config.selection_mode
    );
    assert_eq!(tile_state.config.dense_rows, decoded.config.dense_rows);
}

#[test]
fn table_runtime_state_not_serialized() {
    // TableRuntimeState should NOT derive Serialize/Deserialize
    // This test verifies that runtime state fields are present but not serialized

    let runtime = TableRuntimeState {
        cache_key: Some(TableCacheKey {
            model_key: TableModelKey(1),
            display_filter: TableSearchSpec::default(),
            view_sort: vec![],
            generation: 0,
        }),
        cache: None,
        last_error: None,
        selection: TableSelection::default(),
        scroll_offset: 42.0,
    };

    // Verify the runtime state has the expected fields
    assert!(runtime.cache_key.is_some());
    assert!(runtime.cache.is_none());
    assert!(runtime.last_error.is_none());
    assert!(runtime.selection.rows.is_empty());
    assert_eq!(runtime.scroll_offset, 42.0);

    // Note: We can't directly test that TableRuntimeState doesn't implement Serialize,
    // but the type system enforces this - if it derived Serialize, it wouldn't compile
    // because OnceLock doesn't implement Serialize.
}

#[test]
fn table_tile_id_generation_unique() {
    use crate::tiles::SurferTileTree;

    let mut tree = SurferTileTree::default();

    let id1 = tree.next_table_id();
    let id2 = tree.next_table_id();
    let id3 = tree.next_table_id();

    assert_ne!(id1, id2);
    assert_ne!(id2, id3);
    assert_ne!(id1, id3);
}

// ========================
// Stage 6 Tests - Sort Spec Manipulation
// ========================

#[test]
fn sort_spec_click_unsorted_column_sets_primary_ascending() {
    // Given: no current sort
    // When: click on "col_0"
    // Then: sort becomes [col_0 Ascending]
    let current: Vec<TableSortSpec> = vec![];
    let clicked = TableColumnKey::Str("col_0".to_string());
    let result = sort_spec_on_click(&current, &clicked);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].key, clicked);
    assert_eq!(result[0].direction, TableSortDirection::Ascending);
}

#[test]
fn sort_spec_click_primary_column_toggles_direction() {
    // Given: sort is [col_0 Ascending]
    // When: click on "col_0"
    // Then: sort becomes [col_0 Descending]
    let current = vec![TableSortSpec {
        key: TableColumnKey::Str("col_0".to_string()),
        direction: TableSortDirection::Ascending,
    }];
    let clicked = TableColumnKey::Str("col_0".to_string());
    let result = sort_spec_on_click(&current, &clicked);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].direction, TableSortDirection::Descending);

    // Click again: Descending -> Ascending
    let result2 = sort_spec_on_click(&result, &clicked);
    assert_eq!(result2[0].direction, TableSortDirection::Ascending);
}

#[test]
fn sort_spec_click_different_column_replaces_sort() {
    // Given: sort is [col_0 Descending]
    // When: click on "col_1"
    // Then: sort becomes [col_1 Ascending] (col_0 removed)
    let current = vec![TableSortSpec {
        key: TableColumnKey::Str("col_0".to_string()),
        direction: TableSortDirection::Descending,
    }];
    let clicked = TableColumnKey::Str("col_1".to_string());
    let result = sort_spec_on_click(&current, &clicked);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].key, clicked);
    assert_eq!(result[0].direction, TableSortDirection::Ascending);
}

#[test]
fn sort_spec_click_secondary_column_promotes_to_primary() {
    // Given: sort is [col_0 Asc, col_1 Desc]
    // When: click on "col_1" (secondary)
    // Then: sort becomes [col_1 Ascending] (promoted, direction reset, others cleared)
    let current = vec![
        TableSortSpec {
            key: TableColumnKey::Str("col_0".to_string()),
            direction: TableSortDirection::Ascending,
        },
        TableSortSpec {
            key: TableColumnKey::Str("col_1".to_string()),
            direction: TableSortDirection::Descending,
        },
    ];
    let clicked = TableColumnKey::Str("col_1".to_string());
    let result = sort_spec_on_click(&current, &clicked);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].key, clicked);
    assert_eq!(result[0].direction, TableSortDirection::Ascending);
}

#[test]
fn sort_spec_shift_click_adds_secondary_sort() {
    // Given: sort is [col_0 Ascending]
    // When: Shift+click on "col_1"
    // Then: sort becomes [col_0 Ascending, col_1 Ascending]
    let current = vec![TableSortSpec {
        key: TableColumnKey::Str("col_0".to_string()),
        direction: TableSortDirection::Ascending,
    }];
    let clicked = TableColumnKey::Str("col_1".to_string());
    let result = sort_spec_on_shift_click(&current, &clicked);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].key, TableColumnKey::Str("col_0".to_string()));
    assert_eq!(result[1].key, clicked);
    assert_eq!(result[1].direction, TableSortDirection::Ascending);
}

#[test]
fn sort_spec_shift_click_existing_column_toggles_direction() {
    // Given: sort is [col_0 Asc, col_1 Asc]
    // When: Shift+click on "col_1"
    // Then: sort becomes [col_0 Asc, col_1 Desc] (position preserved)
    let current = vec![
        TableSortSpec {
            key: TableColumnKey::Str("col_0".to_string()),
            direction: TableSortDirection::Ascending,
        },
        TableSortSpec {
            key: TableColumnKey::Str("col_1".to_string()),
            direction: TableSortDirection::Ascending,
        },
    ];
    let clicked = TableColumnKey::Str("col_1".to_string());
    let result = sort_spec_on_shift_click(&current, &clicked);
    assert_eq!(result.len(), 2);
    assert_eq!(result[1].key, clicked);
    assert_eq!(result[1].direction, TableSortDirection::Descending);
}

#[test]
fn sort_spec_shift_click_on_unsorted_table_sets_primary() {
    // Given: no current sort
    // When: Shift+click on "col_0"
    // Then: sort becomes [col_0 Ascending]
    let current: Vec<TableSortSpec> = vec![];
    let clicked = TableColumnKey::Str("col_0".to_string());
    let result = sort_spec_on_shift_click(&current, &clicked);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].direction, TableSortDirection::Ascending);
}

#[test]
fn sort_indicator_no_sort_returns_none() {
    let sort: Vec<TableSortSpec> = vec![];
    let key = TableColumnKey::Str("col_0".to_string());
    assert_eq!(sort_indicator(&sort, &key), None);
}

#[test]
fn sort_indicator_column_not_in_sort_returns_none() {
    let sort = vec![TableSortSpec {
        key: TableColumnKey::Str("col_0".to_string()),
        direction: TableSortDirection::Ascending,
    }];
    let key = TableColumnKey::Str("col_1".to_string());
    assert_eq!(sort_indicator(&sort, &key), None);
}

#[test]
fn sort_indicator_single_column_no_number() {
    // Single-column sort: just arrow, no number
    let sort = vec![TableSortSpec {
        key: TableColumnKey::Str("col_0".to_string()),
        direction: TableSortDirection::Ascending,
    }];
    let key = TableColumnKey::Str("col_0".to_string());
    assert_eq!(sort_indicator(&sort, &key), Some("▲".to_string()));

    let sort_desc = vec![TableSortSpec {
        key: TableColumnKey::Str("col_0".to_string()),
        direction: TableSortDirection::Descending,
    }];
    assert_eq!(sort_indicator(&sort_desc, &key), Some("▼".to_string()));
}

#[test]
fn sort_indicator_multi_column_shows_priority() {
    // Multi-column sort: arrow + priority number
    let sort = vec![
        TableSortSpec {
            key: TableColumnKey::Str("col_0".to_string()),
            direction: TableSortDirection::Ascending,
        },
        TableSortSpec {
            key: TableColumnKey::Str("col_1".to_string()),
            direction: TableSortDirection::Descending,
        },
    ];
    assert_eq!(
        sort_indicator(&sort, &TableColumnKey::Str("col_0".to_string())),
        Some("▲1".to_string())
    );
    assert_eq!(
        sort_indicator(&sort, &TableColumnKey::Str("col_1".to_string())),
        Some("▼2".to_string())
    );
}

// ========================
// Stage 6 Tests - Message Handling Integration
// ========================

#[test]
fn set_table_sort_updates_config() {
    // Setup: create state with a table tile
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 3,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Initially: no sort
    assert!(state.user.table_tiles[&tile_id].config.sort.is_empty());

    // Action: send SetTableSort message
    let new_sort = vec![TableSortSpec {
        key: TableColumnKey::Str("col_0".to_string()),
        direction: TableSortDirection::Ascending,
    }];
    state.update(Message::SetTableSort {
        tile_id,
        sort: new_sort.clone(),
    });

    // Verify: config updated
    assert_eq!(state.user.table_tiles[&tile_id].config.sort, new_sort);
}

#[test]
fn set_table_sort_nonexistent_tile_ignored() {
    // Setup: state with no table tiles
    let mut state = SystemState::new_default_config().expect("state");

    // Action: send SetTableSort for non-existent tile
    let fake_tile_id = TableTileId(9999);
    state.update(Message::SetTableSort {
        tile_id: fake_tile_id,
        sort: vec![TableSortSpec {
            key: TableColumnKey::Str("col_0".to_string()),
            direction: TableSortDirection::Ascending,
        }],
    });

    // Verify: no crash, no state change
    assert!(state.user.table_tiles.is_empty());
}

#[test]
fn multi_column_sort_via_messages() {
    // Test setting up multi-column sort through message updates
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 3,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Set multi-column sort
    let multi_sort = vec![
        TableSortSpec {
            key: TableColumnKey::Str("col_0".to_string()),
            direction: TableSortDirection::Ascending,
        },
        TableSortSpec {
            key: TableColumnKey::Str("col_1".to_string()),
            direction: TableSortDirection::Descending,
        },
    ];
    state.update(Message::SetTableSort {
        tile_id,
        sort: multi_sort.clone(),
    });

    assert_eq!(state.user.table_tiles[&tile_id].config.sort, multi_sort);
}
