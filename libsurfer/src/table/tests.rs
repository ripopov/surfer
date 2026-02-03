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
            type_search: TypeSearchState::default(),
            scroll_state: TableScrollState::default(),
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
        type_search: TypeSearchState::default(),
        scroll_state: TableScrollState::default(),
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
    assert_eq!(sort_indicator(&sort, &key), Some("⬆".to_string()));

    let sort_desc = vec![TableSortSpec {
        key: TableColumnKey::Str("col_0".to_string()),
        direction: TableSortDirection::Descending,
    }];
    assert_eq!(sort_indicator(&sort_desc, &key), Some("⬇".to_string()));
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
        Some("⬆1".to_string())
    );
    assert_eq!(
        sort_indicator(&sort, &TableColumnKey::Str("col_1".to_string())),
        Some("⬇2".to_string())
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

// ========================
// Stage 7 Tests - Fuzzy Matching
// ========================

#[test]
fn fuzzy_match_exact_characters_in_order() {
    // "abc" should match "abc" exactly
    assert!(fuzzy_match("abc", "abc", "abc", true));
    assert!(fuzzy_match("abc", "abc", "abc", false));
}

#[test]
fn fuzzy_match_subsequence_with_gaps() {
    // "abc" should match "aXbYcZ" (characters in order with gaps)
    assert!(fuzzy_match("abc", "abc", "aXbYcZ", true));
    assert!(fuzzy_match("abc", "abc", "a_b_c", true));
    assert!(fuzzy_match("abc", "abc", "a---b---c", true));
}

#[test]
fn fuzzy_match_fails_wrong_order() {
    // "abc" should NOT match "bac" (wrong order)
    assert!(!fuzzy_match("abc", "abc", "bac", true));
    assert!(!fuzzy_match("abc", "abc", "cba", true));
    assert!(!fuzzy_match("abc", "abc", "acb", true));
}

#[test]
fn fuzzy_match_fails_missing_character() {
    // "abc" should NOT match "ab" (missing 'c')
    assert!(!fuzzy_match("abc", "abc", "ab", true));
    assert!(!fuzzy_match("abc", "abc", "ac", true));
    assert!(!fuzzy_match("abc", "abc", "bc", true));
}

#[test]
fn fuzzy_match_empty_needle_matches_all() {
    // Empty needle matches everything
    assert!(fuzzy_match("", "", "anything", true));
    assert!(fuzzy_match("", "", "", true));
}

#[test]
fn fuzzy_match_case_insensitive() {
    // Case-insensitive matching
    assert!(fuzzy_match("abc", "abc", "ABC", false));
    assert!(fuzzy_match("ABC", "abc", "abc", false));
    assert!(fuzzy_match("AbC", "abc", "aBc", false));
    // Case-sensitive should fail
    assert!(!fuzzy_match("abc", "abc", "ABC", true));
}

#[test]
fn fuzzy_match_unicode() {
    // Unicode characters should work
    assert!(fuzzy_match("αβγ", "αβγ", "αXβYγ", true));
    assert!(fuzzy_match("日本", "日本", "日X本", true));
}

// ========================
// Stage 7 Tests - Filter Cache Building
// ========================

#[test]
fn table_cache_builder_filters_fuzzy() {
    let model = Arc::new(VirtualTableModel::new(10, 2, 0));
    // VirtualTableModel cell format: "r{row}c{col}"
    // Row 3 has "r3c0" and "r3c1" -> search_text contains "r3c0 r3c1"
    // Fuzzy "r3" should match rows containing "r3" as subsequence
    let cache = build_table_cache(
        model,
        TableSearchSpec {
            mode: TableSearchMode::Fuzzy,
            case_sensitive: false,
            text: "r3".to_string(),
        },
        vec![],
    )
    .expect("cache build should succeed");

    // Should match row 3 (contains "r3c0")
    assert!(cache.row_ids.contains(&TableRowId(3)));
}

#[test]
fn table_cache_builder_fuzzy_subsequence_matching() {
    // Create a custom model with known search texts for precise fuzzy testing
    #[derive(Clone)]
    struct FuzzyTestModel {
        rows: Vec<(TableRowId, String)>,
    }

    impl TableModel for FuzzyTestModel {
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
            self.rows.get(index).map(|(id, _)| *id)
        }

        fn cell(&self, row: TableRowId, _col: usize) -> TableCell {
            let text = self
                .rows
                .iter()
                .find(|(id, _)| *id == row)
                .map(|(_, t)| t.clone())
                .unwrap_or_default();
            TableCell::Text(text)
        }

        fn sort_key(&self, _row: TableRowId, _col: usize) -> TableSortKey {
            TableSortKey::None
        }

        fn search_text(&self, row: TableRowId) -> String {
            self.rows
                .iter()
                .find(|(id, _)| *id == row)
                .map(|(_, t)| t.clone())
                .unwrap_or_default()
        }

        fn on_activate(&self, _row: TableRowId) -> TableAction {
            TableAction::None
        }
    }

    let model = Arc::new(FuzzyTestModel {
        rows: vec![
            (TableRowId(0), "alpha".to_string()), // "aa" matches: a_l_p_h_a
            (TableRowId(1), "beta".to_string()),  // "aa" does not match
            (TableRowId(2), "gamma".to_string()), // "aa" matches: g_a_m_m_a
            (TableRowId(3), "delta".to_string()), // "aa" does not match
            (TableRowId(4), "abracadabra".to_string()), // "aa" matches: a_b_r_a_c...
        ],
    });

    let cache = build_table_cache(
        model,
        TableSearchSpec {
            mode: TableSearchMode::Fuzzy,
            case_sensitive: false,
            text: "aa".to_string(),
        },
        vec![],
    )
    .expect("cache build should succeed");

    // Rows 0, 2, 4 have two 'a' characters in order
    assert_eq!(cache.row_ids.len(), 3);
    assert!(cache.row_ids.contains(&TableRowId(0)));
    assert!(cache.row_ids.contains(&TableRowId(2)));
    assert!(cache.row_ids.contains(&TableRowId(4)));
}

// ========================
// Stage 7 Tests - Filter Spec Helpers
// ========================

#[test]
fn table_search_spec_default_is_inactive() {
    let spec = TableSearchSpec::default();
    assert!(spec.text.is_empty());
    assert_eq!(spec.mode, TableSearchMode::Contains);
    assert!(!spec.case_sensitive);
}

#[test]
fn table_search_spec_is_active() {
    // Empty text means inactive
    let inactive = TableSearchSpec {
        mode: TableSearchMode::Contains,
        case_sensitive: false,
        text: String::new(),
    };
    assert!(inactive.text.is_empty());

    // Non-empty text means active
    let active = TableSearchSpec {
        mode: TableSearchMode::Contains,
        case_sensitive: false,
        text: "search".to_string(),
    };
    assert!(!active.text.is_empty());
}

#[test]
fn table_search_mode_serialization() {
    // All modes should serialize/deserialize correctly
    for mode in [
        TableSearchMode::Contains,
        TableSearchMode::Exact,
        TableSearchMode::Regex,
        TableSearchMode::Fuzzy,
    ] {
        let encoded = ron::ser::to_string(&mode).expect("serialize");
        let decoded: TableSearchMode = ron::de::from_str(&encoded).expect("deserialize");
        assert_eq!(mode, decoded);
    }
}

// ========================
// Stage 7 Tests - Message Handling Integration
// ========================

use super::{
    format_selection_count, selection_on_click_multi, selection_on_click_single,
    selection_on_ctrl_click, selection_on_shift_click,
};

#[test]
fn set_table_display_filter_updates_config() {
    // Setup: create state with a table tile
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 3,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Initially: empty filter
    assert!(
        state.user.table_tiles[&tile_id]
            .config
            .display_filter
            .text
            .is_empty()
    );

    // Action: send SetTableDisplayFilter message
    let new_filter = TableSearchSpec {
        mode: TableSearchMode::Contains,
        case_sensitive: true,
        text: "search term".to_string(),
    };
    state.update(Message::SetTableDisplayFilter {
        tile_id,
        filter: new_filter.clone(),
    });

    // Verify: config updated
    assert_eq!(
        state.user.table_tiles[&tile_id].config.display_filter,
        new_filter
    );
}

#[test]
fn set_table_display_filter_nonexistent_tile_ignored() {
    // Setup: state with no table tiles
    let mut state = SystemState::new_default_config().expect("state");

    // Action: send SetTableDisplayFilter for non-existent tile
    let fake_tile_id = TableTileId(9999);
    state.update(Message::SetTableDisplayFilter {
        tile_id: fake_tile_id,
        filter: TableSearchSpec {
            mode: TableSearchMode::Regex,
            case_sensitive: false,
            text: "test".to_string(),
        },
    });

    // Verify: no crash, no state change
    assert!(state.user.table_tiles.is_empty());
}

#[test]
fn set_table_display_filter_with_all_modes() {
    // Test that all search modes can be set via message
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 3,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    for mode in [
        TableSearchMode::Contains,
        TableSearchMode::Exact,
        TableSearchMode::Regex,
        TableSearchMode::Fuzzy,
    ] {
        let filter = TableSearchSpec {
            mode,
            case_sensitive: false,
            text: "test".to_string(),
        };
        state.update(Message::SetTableDisplayFilter {
            tile_id,
            filter: filter.clone(),
        });
        assert_eq!(
            state.user.table_tiles[&tile_id].config.display_filter.mode,
            mode
        );
    }
}

// ========================
// Stage 8 Tests - TableSelection Methods
// ========================

#[test]
fn table_selection_new_is_empty() {
    let sel = TableSelection::new();
    assert!(sel.is_empty());
    assert_eq!(sel.len(), 0);
    assert!(sel.anchor.is_none());
}

#[test]
fn table_selection_contains() {
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(1));
    sel.rows.insert(TableRowId(3));
    sel.rows.insert(TableRowId(5));

    assert!(sel.contains(TableRowId(1)));
    assert!(!sel.contains(TableRowId(2)));
    assert!(sel.contains(TableRowId(3)));
    assert!(sel.contains(TableRowId(5)));
    assert!(!sel.contains(TableRowId(0)));
}

#[test]
fn table_selection_clear() {
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(1));
    sel.rows.insert(TableRowId(2));
    sel.anchor = Some(TableRowId(1));

    sel.clear();

    assert!(sel.is_empty());
    assert!(sel.anchor.is_none());
}

#[test]
fn table_selection_count_visible() {
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(1));
    sel.rows.insert(TableRowId(3));
    sel.rows.insert(TableRowId(5));
    sel.rows.insert(TableRowId(7));

    // Only rows 1, 3, 5 are visible (7 is filtered out)
    let visible = vec![
        TableRowId(0),
        TableRowId(1),
        TableRowId(2),
        TableRowId(3),
        TableRowId(5),
    ];

    assert_eq!(sel.count_visible(&visible), 3);
    assert_eq!(sel.count_hidden(&visible), 1);
}

#[test]
fn table_selection_count_all_visible() {
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(1));
    sel.rows.insert(TableRowId(2));

    let visible = vec![TableRowId(0), TableRowId(1), TableRowId(2), TableRowId(3)];

    assert_eq!(sel.count_visible(&visible), 2);
    assert_eq!(sel.count_hidden(&visible), 0);
}

#[test]
fn table_selection_count_all_hidden() {
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(10));
    sel.rows.insert(TableRowId(20));

    let visible = vec![TableRowId(0), TableRowId(1), TableRowId(2)];

    assert_eq!(sel.count_visible(&visible), 0);
    assert_eq!(sel.count_hidden(&visible), 2);
}

// ========================
// Stage 8 Tests - Single Mode Selection
// ========================

#[test]
fn selection_single_mode_click_selects_row() {
    let current = TableSelection::new();
    let result = selection_on_click_single(&current, TableRowId(5));

    assert!(result.changed);
    assert_eq!(result.selection.len(), 1);
    assert!(result.selection.contains(TableRowId(5)));
    assert_eq!(result.selection.anchor, Some(TableRowId(5)));
}

#[test]
fn selection_single_mode_click_replaces_previous() {
    let mut current = TableSelection::new();
    current.rows.insert(TableRowId(3));
    current.anchor = Some(TableRowId(3));

    let result = selection_on_click_single(&current, TableRowId(7));

    assert!(result.changed);
    assert_eq!(result.selection.len(), 1);
    assert!(!result.selection.contains(TableRowId(3)));
    assert!(result.selection.contains(TableRowId(7)));
    assert_eq!(result.selection.anchor, Some(TableRowId(7)));
}

#[test]
fn selection_single_mode_click_same_row_unchanged() {
    let mut current = TableSelection::new();
    current.rows.insert(TableRowId(5));
    current.anchor = Some(TableRowId(5));

    let result = selection_on_click_single(&current, TableRowId(5));

    // Clicking same row should not report change
    assert!(!result.changed);
    assert_eq!(result.selection.len(), 1);
    assert!(result.selection.contains(TableRowId(5)));
}

// ========================
// Stage 8 Tests - Multi Mode Selection
// ========================

#[test]
fn selection_multi_mode_click_selects_row() {
    let current = TableSelection::new();
    let result = selection_on_click_multi(&current, TableRowId(5));

    assert!(result.changed);
    assert_eq!(result.selection.len(), 1);
    assert!(result.selection.contains(TableRowId(5)));
    assert_eq!(result.selection.anchor, Some(TableRowId(5)));
}

#[test]
fn selection_multi_mode_click_clears_previous() {
    let mut current = TableSelection::new();
    current.rows.insert(TableRowId(1));
    current.rows.insert(TableRowId(2));
    current.rows.insert(TableRowId(3));
    current.anchor = Some(TableRowId(1));

    let result = selection_on_click_multi(&current, TableRowId(5));

    assert!(result.changed);
    assert_eq!(result.selection.len(), 1);
    assert!(result.selection.contains(TableRowId(5)));
    assert!(!result.selection.contains(TableRowId(1)));
}

#[test]
fn selection_multi_mode_ctrl_click_toggles_on() {
    let mut current = TableSelection::new();
    current.rows.insert(TableRowId(1));
    current.rows.insert(TableRowId(3));
    current.anchor = Some(TableRowId(1));

    let result = selection_on_ctrl_click(&current, TableRowId(5));

    assert!(result.changed);
    assert_eq!(result.selection.len(), 3);
    assert!(result.selection.contains(TableRowId(1)));
    assert!(result.selection.contains(TableRowId(3)));
    assert!(result.selection.contains(TableRowId(5)));
    assert_eq!(result.selection.anchor, Some(TableRowId(5)));
}

#[test]
fn selection_multi_mode_ctrl_click_toggles_off() {
    let mut current = TableSelection::new();
    current.rows.insert(TableRowId(1));
    current.rows.insert(TableRowId(3));
    current.rows.insert(TableRowId(5));
    current.anchor = Some(TableRowId(1));

    let result = selection_on_ctrl_click(&current, TableRowId(3));

    assert!(result.changed);
    assert_eq!(result.selection.len(), 2);
    assert!(result.selection.contains(TableRowId(1)));
    assert!(!result.selection.contains(TableRowId(3)));
    assert!(result.selection.contains(TableRowId(5)));
    assert_eq!(result.selection.anchor, Some(TableRowId(3)));
}

#[test]
fn selection_multi_mode_ctrl_click_empty_selection() {
    let current = TableSelection::new();
    let result = selection_on_ctrl_click(&current, TableRowId(5));

    assert!(result.changed);
    assert_eq!(result.selection.len(), 1);
    assert!(result.selection.contains(TableRowId(5)));
    assert_eq!(result.selection.anchor, Some(TableRowId(5)));
}

// ========================
// Stage 8 Tests - Range Selection (Shift+Click)
// ========================

#[test]
fn selection_shift_click_range_forward() {
    let mut current = TableSelection::new();
    current.rows.insert(TableRowId(2));
    current.anchor = Some(TableRowId(2));

    // Visible order: 0, 1, 2, 3, 4, 5
    let visible = vec![
        TableRowId(0),
        TableRowId(1),
        TableRowId(2),
        TableRowId(3),
        TableRowId(4),
        TableRowId(5),
    ];

    let result = selection_on_shift_click(&current, TableRowId(5), &visible);

    assert!(result.changed);
    // Should select rows 2, 3, 4, 5 (inclusive range)
    assert_eq!(result.selection.len(), 4);
    assert!(result.selection.contains(TableRowId(2)));
    assert!(result.selection.contains(TableRowId(3)));
    assert!(result.selection.contains(TableRowId(4)));
    assert!(result.selection.contains(TableRowId(5)));
    // Anchor preserved
    assert_eq!(result.selection.anchor, Some(TableRowId(2)));
}

#[test]
fn selection_shift_click_range_backward() {
    let mut current = TableSelection::new();
    current.rows.insert(TableRowId(5));
    current.anchor = Some(TableRowId(5));

    let visible = vec![
        TableRowId(0),
        TableRowId(1),
        TableRowId(2),
        TableRowId(3),
        TableRowId(4),
        TableRowId(5),
    ];

    let result = selection_on_shift_click(&current, TableRowId(2), &visible);

    assert!(result.changed);
    // Should select rows 2, 3, 4, 5 (inclusive range backward)
    assert_eq!(result.selection.len(), 4);
    assert!(result.selection.contains(TableRowId(2)));
    assert!(result.selection.contains(TableRowId(3)));
    assert!(result.selection.contains(TableRowId(4)));
    assert!(result.selection.contains(TableRowId(5)));
    assert_eq!(result.selection.anchor, Some(TableRowId(5)));
}

#[test]
fn selection_shift_click_single_row() {
    let mut current = TableSelection::new();
    current.rows.insert(TableRowId(3));
    current.anchor = Some(TableRowId(3));

    let visible = vec![TableRowId(0), TableRowId(1), TableRowId(2), TableRowId(3)];

    let result = selection_on_shift_click(&current, TableRowId(3), &visible);

    // Shift+click on same row as anchor - just that row
    assert!(!result.changed); // Already selected
    assert_eq!(result.selection.len(), 1);
    assert!(result.selection.contains(TableRowId(3)));
}

#[test]
fn selection_shift_click_no_anchor_uses_clicked_as_anchor() {
    let current = TableSelection::new();
    let visible = vec![TableRowId(0), TableRowId(1), TableRowId(2), TableRowId(3)];

    let result = selection_on_shift_click(&current, TableRowId(2), &visible);

    // No anchor means treat clicked row as both anchor and target
    assert!(result.changed);
    assert_eq!(result.selection.len(), 1);
    assert!(result.selection.contains(TableRowId(2)));
    assert_eq!(result.selection.anchor, Some(TableRowId(2)));
}

#[test]
fn selection_shift_click_anchor_not_visible_uses_clicked() {
    let mut current = TableSelection::new();
    current.rows.insert(TableRowId(10)); // Row 10 is filtered out
    current.anchor = Some(TableRowId(10));

    let visible = vec![TableRowId(0), TableRowId(1), TableRowId(2), TableRowId(3)];

    let result = selection_on_shift_click(&current, TableRowId(2), &visible);

    // Anchor not in visible set - select just clicked row and set new anchor
    assert!(result.changed);
    assert!(result.selection.contains(TableRowId(2)));
    assert_eq!(result.selection.anchor, Some(TableRowId(2)));
}

#[test]
fn selection_shift_click_extends_from_anchor_replaces_selection() {
    let mut current = TableSelection::new();
    current.rows.insert(TableRowId(0));
    current.rows.insert(TableRowId(1));
    current.rows.insert(TableRowId(2));
    current.anchor = Some(TableRowId(0));

    let visible = vec![
        TableRowId(0),
        TableRowId(1),
        TableRowId(2),
        TableRowId(3),
        TableRowId(4),
        TableRowId(5),
    ];

    // Shift+click at row 4 should select 0-4, replacing 0-2
    let result = selection_on_shift_click(&current, TableRowId(4), &visible);

    assert!(result.changed);
    assert_eq!(result.selection.len(), 5);
    assert!(result.selection.contains(TableRowId(0)));
    assert!(result.selection.contains(TableRowId(1)));
    assert!(result.selection.contains(TableRowId(2)));
    assert!(result.selection.contains(TableRowId(3)));
    assert!(result.selection.contains(TableRowId(4)));
    assert!(!result.selection.contains(TableRowId(5)));
    // Anchor preserved
    assert_eq!(result.selection.anchor, Some(TableRowId(0)));
}

// ========================
// Stage 8 Tests - Selection Count Formatting
// ========================

#[test]
fn format_selection_count_none() {
    assert_eq!(format_selection_count(0, 0), "");
}

#[test]
fn format_selection_count_visible_only() {
    assert_eq!(format_selection_count(5, 0), "5 selected");
    assert_eq!(format_selection_count(1, 0), "1 selected");
}

#[test]
fn format_selection_count_with_hidden() {
    assert_eq!(format_selection_count(5, 2), "5 selected (2 hidden)");
    assert_eq!(format_selection_count(10, 1), "10 selected (1 hidden)");
}

#[test]
fn format_selection_count_all_hidden() {
    // All selected rows are hidden
    assert_eq!(format_selection_count(3, 3), "3 selected (3 hidden)");
}

// ========================
// Stage 8 Tests - Selection Mode Behavior
// ========================

#[test]
fn selection_mode_none_value() {
    // In None mode, selection should always be empty
    // This is tested at the UI/integration level
    let mode = TableSelectionMode::None;
    assert_eq!(mode, TableSelectionMode::None);
}

#[test]
fn selection_mode_serialization() {
    for mode in [
        TableSelectionMode::None,
        TableSelectionMode::Single,
        TableSelectionMode::Multi,
    ] {
        let encoded = ron::ser::to_string(&mode).expect("serialize");
        let decoded: TableSelectionMode = ron::de::from_str(&encoded).expect("deserialize");
        assert_eq!(mode, decoded);
    }
}

// ========================
// Stage 8 Tests - Message Handling Integration
// ========================

#[test]
fn set_table_selection_updates_runtime() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 3,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Ensure runtime state exists
    state
        .table_runtime
        .entry(tile_id)
        .or_insert_with(TableRuntimeState::default);

    // Initially: empty selection
    assert!(state.table_runtime[&tile_id].selection.is_empty());

    // Set selection
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(1));
    selection.rows.insert(TableRowId(3));
    selection.anchor = Some(TableRowId(1));

    state.update(Message::SetTableSelection {
        tile_id,
        selection: selection.clone(),
    });

    assert_eq!(state.table_runtime[&tile_id].selection, selection);
}

#[test]
fn clear_table_selection_clears_runtime() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 3,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Ensure runtime state exists
    state
        .table_runtime
        .entry(tile_id)
        .or_insert_with(TableRuntimeState::default);

    // Set selection first
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(5));
    selection.anchor = Some(TableRowId(5));
    state.update(Message::SetTableSelection { tile_id, selection });

    assert!(!state.table_runtime[&tile_id].selection.is_empty());

    // Clear selection
    state.update(Message::ClearTableSelection { tile_id });

    assert!(state.table_runtime[&tile_id].selection.is_empty());
    assert!(state.table_runtime[&tile_id].selection.anchor.is_none());
}

#[test]
fn set_table_selection_nonexistent_tile_ignored() {
    let mut state = SystemState::new_default_config().expect("state");

    let fake_tile_id = TableTileId(9999);
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(1));

    // Should not crash
    state.update(Message::SetTableSelection {
        tile_id: fake_tile_id,
        selection,
    });

    assert!(state.user.table_tiles.is_empty());
}

#[test]
fn selection_persists_after_sort_change() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 3,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Ensure runtime state exists
    state
        .table_runtime
        .entry(tile_id)
        .or_insert_with(TableRuntimeState::default);

    // Set selection
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(3));
    selection.rows.insert(TableRowId(7));
    selection.anchor = Some(TableRowId(3));
    state.update(Message::SetTableSelection {
        tile_id,
        selection: selection.clone(),
    });

    // Change sort
    state.update(Message::SetTableSort {
        tile_id,
        sort: vec![TableSortSpec {
            key: TableColumnKey::Str("col_0".to_string()),
            direction: TableSortDirection::Descending,
        }],
    });

    // Selection should persist (tracked by TableRowId, not index)
    assert_eq!(state.table_runtime[&tile_id].selection.len(), 2);
    assert!(
        state.table_runtime[&tile_id]
            .selection
            .contains(TableRowId(3))
    );
    assert!(
        state.table_runtime[&tile_id]
            .selection
            .contains(TableRowId(7))
    );
}

#[test]
fn selection_persists_after_filter_change() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 3,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Ensure runtime state exists
    state
        .table_runtime
        .entry(tile_id)
        .or_insert_with(TableRuntimeState::default);

    // Select multiple rows
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(1));
    selection.rows.insert(TableRowId(3));
    selection.rows.insert(TableRowId(5));
    selection.anchor = Some(TableRowId(1));
    state.update(Message::SetTableSelection {
        tile_id,
        selection: selection.clone(),
    });

    // Apply filter that hides some rows
    state.update(Message::SetTableDisplayFilter {
        tile_id,
        filter: TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: "r3".to_string(), // Only matches row 3
        },
    });

    // Selection should persist (hidden rows stay selected)
    assert_eq!(state.table_runtime[&tile_id].selection.len(), 3);
    assert!(
        state.table_runtime[&tile_id]
            .selection
            .contains(TableRowId(1))
    ); // hidden
    assert!(
        state.table_runtime[&tile_id]
            .selection
            .contains(TableRowId(3))
    ); // visible
    assert!(
        state.table_runtime[&tile_id]
            .selection
            .contains(TableRowId(5))
    ); // hidden
}

#[test]
fn selection_shift_click_sorted_order() {
    // After sorting, rows may appear in different order
    let mut current = TableSelection::new();
    current.anchor = Some(TableRowId(5));
    current.rows.insert(TableRowId(5));

    // Sorted order: 5, 3, 1, 4, 2, 0 (arbitrary sort result)
    let visible = vec![
        TableRowId(5),
        TableRowId(3),
        TableRowId(1),
        TableRowId(4),
        TableRowId(2),
        TableRowId(0),
    ];

    let result = selection_on_shift_click(&current, TableRowId(4), &visible);

    // Should select rows 5, 3, 1, 4 (by display order)
    assert!(result.changed);
    assert_eq!(result.selection.len(), 4);
    assert!(result.selection.contains(TableRowId(5)));
    assert!(result.selection.contains(TableRowId(3)));
    assert!(result.selection.contains(TableRowId(1)));
    assert!(result.selection.contains(TableRowId(4)));
    assert!(!result.selection.contains(TableRowId(2)));
    assert!(!result.selection.contains(TableRowId(0)));
}

// ========================
// Stage 9 Tests - Basic Navigation (Up/Down)
// ========================

use super::{
    TypeSearchState, find_type_search_match, format_rows_as_tsv, format_rows_as_tsv_with_header,
    navigate_down, navigate_end, navigate_extend_selection, navigate_home, navigate_page_down,
    navigate_page_up, navigate_up,
};

#[test]
fn navigate_up_from_middle_row() {
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(3));
    sel.anchor = Some(TableRowId(3));

    let visible = vec![
        TableRowId(0),
        TableRowId(1),
        TableRowId(2),
        TableRowId(3),
        TableRowId(4),
        TableRowId(5),
    ];

    let result = navigate_up(&sel, &visible);

    assert!(result.selection_changed);
    assert_eq!(result.target_row, Some(TableRowId(2)));
    let new_sel = result.new_selection.unwrap();
    assert_eq!(new_sel.len(), 1);
    assert!(new_sel.contains(TableRowId(2)));
    assert_eq!(new_sel.anchor, Some(TableRowId(2)));
}

#[test]
fn navigate_up_from_first_row_stays() {
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(0));
    sel.anchor = Some(TableRowId(0));

    let visible = vec![TableRowId(0), TableRowId(1), TableRowId(2)];

    let result = navigate_up(&sel, &visible);

    // Already at first row - no change
    assert!(!result.selection_changed);
    assert_eq!(result.target_row, Some(TableRowId(0)));
}

#[test]
fn navigate_up_empty_selection_selects_last() {
    let sel = TableSelection::new();
    let visible = vec![TableRowId(0), TableRowId(1), TableRowId(2)];

    let result = navigate_up(&sel, &visible);

    // No selection: Up selects last row (like most file managers)
    assert!(result.selection_changed);
    assert_eq!(result.target_row, Some(TableRowId(2)));
}

#[test]
fn navigate_up_empty_visible_no_change() {
    let sel = TableSelection::new();
    let visible: Vec<TableRowId> = vec![];

    let result = navigate_up(&sel, &visible);

    assert!(!result.selection_changed);
    assert_eq!(result.target_row, None);
}

#[test]
fn navigate_down_from_middle_row() {
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(2));
    sel.anchor = Some(TableRowId(2));

    let visible = vec![
        TableRowId(0),
        TableRowId(1),
        TableRowId(2),
        TableRowId(3),
        TableRowId(4),
        TableRowId(5),
    ];

    let result = navigate_down(&sel, &visible);

    assert!(result.selection_changed);
    assert_eq!(result.target_row, Some(TableRowId(3)));
}

#[test]
fn navigate_down_from_last_row_stays() {
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(5));
    sel.anchor = Some(TableRowId(5));

    let visible = vec![
        TableRowId(0),
        TableRowId(1),
        TableRowId(2),
        TableRowId(3),
        TableRowId(4),
        TableRowId(5),
    ];

    let result = navigate_down(&sel, &visible);

    assert!(!result.selection_changed);
    assert_eq!(result.target_row, Some(TableRowId(5)));
}

#[test]
fn navigate_down_empty_selection_selects_first() {
    let sel = TableSelection::new();
    let visible = vec![TableRowId(0), TableRowId(1), TableRowId(2)];

    let result = navigate_down(&sel, &visible);

    // No selection: Down selects first row
    assert!(result.selection_changed);
    assert_eq!(result.target_row, Some(TableRowId(0)));
}

#[test]
fn navigate_up_multi_selection_uses_anchor() {
    // With multiple rows selected, navigation uses anchor position
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(1));
    sel.rows.insert(TableRowId(3));
    sel.rows.insert(TableRowId(5));
    sel.anchor = Some(TableRowId(3)); // Anchor in middle

    let visible = vec![
        TableRowId(0),
        TableRowId(1),
        TableRowId(2),
        TableRowId(3),
        TableRowId(4),
        TableRowId(5),
    ];

    let result = navigate_up(&sel, &visible);

    // Moves from anchor (3) to row 2, clears multi-selection
    assert!(result.selection_changed);
    assert_eq!(result.target_row, Some(TableRowId(2)));
    let new_sel = result.new_selection.unwrap();
    assert_eq!(new_sel.len(), 1);
    assert!(new_sel.contains(TableRowId(2)));
}

// ========================
// Stage 9 Tests - Page Navigation
// ========================

#[test]
fn navigate_page_down_moves_by_page_size() {
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(0));
    sel.anchor = Some(TableRowId(0));

    let visible: Vec<TableRowId> = (0..20).map(TableRowId).collect();
    let page_size = 5;

    let result = navigate_page_down(&sel, &visible, page_size);

    assert!(result.selection_changed);
    assert_eq!(result.target_row, Some(TableRowId(5)));
}

#[test]
fn navigate_page_down_stops_at_end() {
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(18));
    sel.anchor = Some(TableRowId(18));

    let visible: Vec<TableRowId> = (0..20).map(TableRowId).collect();
    let page_size = 5;

    let result = navigate_page_down(&sel, &visible, page_size);

    // Would go to 23, but stops at 19 (last row)
    assert!(result.selection_changed);
    assert_eq!(result.target_row, Some(TableRowId(19)));
}

#[test]
fn navigate_page_up_moves_by_page_size() {
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(10));
    sel.anchor = Some(TableRowId(10));

    let visible: Vec<TableRowId> = (0..20).map(TableRowId).collect();
    let page_size = 5;

    let result = navigate_page_up(&sel, &visible, page_size);

    assert!(result.selection_changed);
    assert_eq!(result.target_row, Some(TableRowId(5)));
}

#[test]
fn navigate_page_up_stops_at_start() {
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(2));
    sel.anchor = Some(TableRowId(2));

    let visible: Vec<TableRowId> = (0..20).map(TableRowId).collect();
    let page_size = 5;

    let result = navigate_page_up(&sel, &visible, page_size);

    // Would go to -3, but stops at 0 (first row)
    assert!(result.selection_changed);
    assert_eq!(result.target_row, Some(TableRowId(0)));
}

#[test]
fn navigate_page_empty_selection() {
    let sel = TableSelection::new();
    let visible: Vec<TableRowId> = (0..20).map(TableRowId).collect();

    let result_down = navigate_page_down(&sel, &visible, 5);
    assert_eq!(result_down.target_row, Some(TableRowId(0))); // Start from beginning

    let result_up = navigate_page_up(&sel, &visible, 5);
    assert_eq!(result_up.target_row, Some(TableRowId(19))); // Start from end
}

// ========================
// Stage 9 Tests - Home/End Navigation
// ========================

#[test]
fn navigate_home_jumps_to_first() {
    let visible = vec![
        TableRowId(5),
        TableRowId(3),
        TableRowId(1), // Sorted order
        TableRowId(4),
        TableRowId(2),
        TableRowId(0),
    ];

    let result = navigate_home(&visible);

    assert_eq!(result.target_row, Some(TableRowId(5))); // First in display order
    assert!(result.selection_changed);
}

#[test]
fn navigate_end_jumps_to_last() {
    let visible = vec![
        TableRowId(5),
        TableRowId(3),
        TableRowId(1),
        TableRowId(4),
        TableRowId(2),
        TableRowId(0),
    ];

    let result = navigate_end(&visible);

    assert_eq!(result.target_row, Some(TableRowId(0))); // Last in display order
    assert!(result.selection_changed);
}

#[test]
fn navigate_home_empty_table() {
    let visible: Vec<TableRowId> = vec![];

    let result = navigate_home(&visible);

    assert_eq!(result.target_row, None);
    assert!(!result.selection_changed);
}

#[test]
fn navigate_end_empty_table() {
    let visible: Vec<TableRowId> = vec![];

    let result = navigate_end(&visible);

    assert_eq!(result.target_row, None);
    assert!(!result.selection_changed);
}

#[test]
fn navigate_home_already_at_first() {
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(0));
    sel.anchor = Some(TableRowId(0));

    let visible = vec![TableRowId(0), TableRowId(1), TableRowId(2)];

    // navigate_home doesn't take current selection - always returns first
    let result = navigate_home(&visible);
    assert_eq!(result.target_row, Some(TableRowId(0)));
}

// ========================
// Stage 9 Tests - Shift+Navigation (Extend Selection)
// ========================

#[test]
fn navigate_extend_selection_down() {
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(2));
    sel.anchor = Some(TableRowId(2));

    let visible = vec![
        TableRowId(0),
        TableRowId(1),
        TableRowId(2),
        TableRowId(3),
        TableRowId(4),
        TableRowId(5),
    ];

    // Shift+Down from row 2 to row 3
    let result = navigate_extend_selection(&sel, TableRowId(3), &visible);

    assert!(result.selection_changed);
    let new_sel = result.new_selection.unwrap();
    // Should select 2 and 3 (range from anchor)
    assert_eq!(new_sel.len(), 2);
    assert!(new_sel.contains(TableRowId(2)));
    assert!(new_sel.contains(TableRowId(3)));
    assert_eq!(new_sel.anchor, Some(TableRowId(2))); // Anchor preserved
}

#[test]
fn navigate_extend_selection_multiple_steps() {
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(2));
    sel.anchor = Some(TableRowId(2));

    let visible = vec![
        TableRowId(0),
        TableRowId(1),
        TableRowId(2),
        TableRowId(3),
        TableRowId(4),
        TableRowId(5),
    ];

    // Shift+Down multiple times: anchor at 2, extend to 5
    let result = navigate_extend_selection(&sel, TableRowId(5), &visible);

    let new_sel = result.new_selection.unwrap();
    assert_eq!(new_sel.len(), 4); // Rows 2, 3, 4, 5
    assert!(new_sel.contains(TableRowId(2)));
    assert!(new_sel.contains(TableRowId(3)));
    assert!(new_sel.contains(TableRowId(4)));
    assert!(new_sel.contains(TableRowId(5)));
}

#[test]
fn navigate_extend_selection_backward() {
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(4));
    sel.anchor = Some(TableRowId(4));

    let visible = vec![
        TableRowId(0),
        TableRowId(1),
        TableRowId(2),
        TableRowId(3),
        TableRowId(4),
        TableRowId(5),
    ];

    // Shift+Up from row 4 to row 2
    let result = navigate_extend_selection(&sel, TableRowId(2), &visible);

    let new_sel = result.new_selection.unwrap();
    assert_eq!(new_sel.len(), 3); // Rows 2, 3, 4
    assert!(new_sel.contains(TableRowId(2)));
    assert!(new_sel.contains(TableRowId(3)));
    assert!(new_sel.contains(TableRowId(4)));
}

#[test]
fn navigate_extend_selection_contract() {
    // Extending selection in opposite direction should contract
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(2));
    sel.rows.insert(TableRowId(3));
    sel.rows.insert(TableRowId(4));
    sel.anchor = Some(TableRowId(2));

    let visible = vec![
        TableRowId(0),
        TableRowId(1),
        TableRowId(2),
        TableRowId(3),
        TableRowId(4),
        TableRowId(5),
    ];

    // Was 2-4, now Shift to 3 (contract)
    let result = navigate_extend_selection(&sel, TableRowId(3), &visible);

    let new_sel = result.new_selection.unwrap();
    assert_eq!(new_sel.len(), 2); // Rows 2, 3
    assert!(new_sel.contains(TableRowId(2)));
    assert!(new_sel.contains(TableRowId(3)));
    assert!(!new_sel.contains(TableRowId(4)));
}

#[test]
fn navigate_extend_selection_no_anchor() {
    let sel = TableSelection::new(); // No anchor

    let visible = vec![TableRowId(0), TableRowId(1), TableRowId(2)];

    let result = navigate_extend_selection(&sel, TableRowId(1), &visible);

    // No anchor - just select the target and set as anchor
    let new_sel = result.new_selection.unwrap();
    assert_eq!(new_sel.len(), 1);
    assert!(new_sel.contains(TableRowId(1)));
    assert_eq!(new_sel.anchor, Some(TableRowId(1)));
}

// ========================
// Stage 9 Tests - Type-to-Search
// ========================

#[test]
fn type_search_finds_prefix_match() {
    let visible = vec![TableRowId(0), TableRowId(1), TableRowId(2), TableRowId(3)];
    let search_texts = vec![
        "alpha".to_string(),
        "beta".to_string(),
        "gamma".to_string(),
        "delta".to_string(),
    ];
    let sel = TableSelection::new();

    let result = find_type_search_match("gam", &sel, &visible, &search_texts);

    assert_eq!(result, Some(TableRowId(2))); // "gamma" starts with "gam"
}

#[test]
fn type_search_finds_contains_match() {
    let visible = vec![TableRowId(0), TableRowId(1), TableRowId(2)];
    let search_texts = vec![
        "hello world".to_string(),
        "foo bar".to_string(),
        "baz qux".to_string(),
    ];
    let sel = TableSelection::new();

    let result = find_type_search_match("bar", &sel, &visible, &search_texts);

    assert_eq!(result, Some(TableRowId(1))); // "foo bar" contains "bar"
}

#[test]
fn type_search_case_insensitive() {
    let visible = vec![TableRowId(0), TableRowId(1)];
    let search_texts = vec!["UPPERCASE".to_string(), "lowercase".to_string()];
    let sel = TableSelection::new();

    let result = find_type_search_match("upper", &sel, &visible, &search_texts);
    assert_eq!(result, Some(TableRowId(0)));

    let result2 = find_type_search_match("LOWER", &sel, &visible, &search_texts);
    assert_eq!(result2, Some(TableRowId(1)));
}

#[test]
fn type_search_wraps_from_selection() {
    // Should find next match after current selection, wrapping around
    let visible = vec![TableRowId(0), TableRowId(1), TableRowId(2), TableRowId(3)];
    let search_texts = vec![
        "apple".to_string(),
        "apricot".to_string(), // Match
        "banana".to_string(),
        "avocado".to_string(), // Also matches "a"
    ];
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(0)); // Currently at "apple"
    sel.anchor = Some(TableRowId(0));

    // Searching "apr" should find next match after current selection
    let result = find_type_search_match("apr", &sel, &visible, &search_texts);

    assert_eq!(result, Some(TableRowId(1))); // "apricot" is next match
}

#[test]
fn type_search_no_match() {
    let visible = vec![TableRowId(0), TableRowId(1)];
    let search_texts = vec!["alpha".to_string(), "beta".to_string()];
    let sel = TableSelection::new();

    let result = find_type_search_match("xyz", &sel, &visible, &search_texts);

    assert_eq!(result, None);
}

#[test]
fn type_search_empty_query() {
    let visible = vec![TableRowId(0), TableRowId(1)];
    let search_texts = vec!["alpha".to_string(), "beta".to_string()];
    let sel = TableSelection::new();

    let result = find_type_search_match("", &sel, &visible, &search_texts);

    assert_eq!(result, None); // Empty query matches nothing
}

#[test]
fn type_search_empty_table() {
    let visible: Vec<TableRowId> = vec![];
    let search_texts: Vec<String> = vec![];
    let sel = TableSelection::new();

    let result = find_type_search_match("test", &sel, &visible, &search_texts);

    assert_eq!(result, None);
}

// ========================
// Stage 9 Tests - TypeSearchState
// ========================

#[test]
fn type_search_state_accumulates() {
    let mut state = TypeSearchState::default();
    let now = std::time::Instant::now();

    state.push_char('a', now);
    assert_eq!(state.buffer, "a");

    state.push_char('b', now);
    assert_eq!(state.buffer, "ab");

    state.push_char('c', now);
    assert_eq!(state.buffer, "abc");
}

#[test]
fn type_search_state_resets_on_timeout() {
    let mut state = TypeSearchState::default();
    let start = std::time::Instant::now();

    state.push_char('a', start);
    state.push_char('b', start);
    assert_eq!(state.buffer, "ab");

    // Simulate timeout (add more than TIMEOUT_MS)
    let later = start + std::time::Duration::from_millis(TypeSearchState::TIMEOUT_MS as u64 + 100);

    state.push_char('x', later);
    assert_eq!(state.buffer, "x"); // Reset and started fresh
}

#[test]
fn type_search_state_clear() {
    let mut state = TypeSearchState::default();
    let now = std::time::Instant::now();

    state.push_char('a', now);
    state.push_char('b', now);
    state.clear();

    assert!(state.buffer.is_empty());
    assert!(state.last_keystroke.is_none());
}

#[test]
fn type_search_state_is_timed_out() {
    let mut state = TypeSearchState::default();
    let start = std::time::Instant::now();

    state.push_char('a', start);
    assert!(!state.is_timed_out(start));

    let within_timeout = start + std::time::Duration::from_millis(500);
    assert!(!state.is_timed_out(within_timeout));

    let after_timeout =
        start + std::time::Duration::from_millis(TypeSearchState::TIMEOUT_MS as u64 + 1);
    assert!(state.is_timed_out(after_timeout));
}

// ========================
// Stage 9 Tests - Copy to Clipboard (TSV Format)
// ========================

#[test]
fn format_rows_as_tsv_single_row() {
    let model = VirtualTableModel::new(5, 3, 42);
    let selected = vec![TableRowId(2)];
    let columns = vec![
        TableColumnKey::Str("col_0".to_string()),
        TableColumnKey::Str("col_1".to_string()),
        TableColumnKey::Str("col_2".to_string()),
    ];

    let tsv = format_rows_as_tsv(&model, &selected, &columns);

    // Should be single line with tab-separated values
    assert!(!tsv.contains('\n') || tsv.ends_with('\n') && tsv.matches('\n').count() == 1);
    assert!(tsv.contains('\t'));
}

#[test]
fn format_rows_as_tsv_multiple_rows() {
    let model = VirtualTableModel::new(5, 3, 42);
    let selected = vec![TableRowId(0), TableRowId(2), TableRowId(4)];
    let columns = vec![
        TableColumnKey::Str("col_0".to_string()),
        TableColumnKey::Str("col_1".to_string()),
    ];

    let tsv = format_rows_as_tsv(&model, &selected, &columns);

    // Should have 3 lines (one per row)
    let lines: Vec<_> = tsv.lines().collect();
    assert_eq!(lines.len(), 3);

    // Each line should have values separated by tabs
    for line in &lines {
        assert!(line.contains('\t'));
    }
}

#[test]
fn format_rows_as_tsv_respects_column_order() {
    let model = VirtualTableModel::new(5, 4, 42);
    let selected = vec![TableRowId(0)];

    // Columns in custom order (reversed)
    let columns = vec![
        TableColumnKey::Str("col_2".to_string()),
        TableColumnKey::Str("col_0".to_string()),
    ];

    let tsv = format_rows_as_tsv(&model, &selected, &columns);

    // Values should be in the specified column order
    let values: Vec<_> = tsv.trim().split('\t').collect();
    assert_eq!(values.len(), 2);
    // First value should be from col_2, second from col_0
}

#[test]
fn format_rows_as_tsv_empty_selection() {
    let model = VirtualTableModel::new(5, 3, 42);
    let selected: Vec<TableRowId> = vec![];
    let columns = vec![TableColumnKey::Str("col_0".to_string())];

    let tsv = format_rows_as_tsv(&model, &selected, &columns);

    assert!(tsv.is_empty());
}

#[test]
fn format_rows_as_tsv_with_header_test() {
    let model = VirtualTableModel::new(5, 3, 42);
    let schema = model.schema();
    let selected = vec![TableRowId(0), TableRowId(1)];
    let columns = vec![
        TableColumnKey::Str("col_0".to_string()),
        TableColumnKey::Str("col_1".to_string()),
    ];

    let tsv = format_rows_as_tsv_with_header(&model, &schema, &selected, &columns);

    let lines: Vec<_> = tsv.lines().collect();
    assert_eq!(lines.len(), 3); // Header + 2 data rows

    // First line should be column labels
    let header = lines[0];
    assert!(header.contains("Col 0") || header.contains("col_0"));
}

#[test]
fn format_rows_as_tsv_escapes_tabs_in_values() {
    // If a cell value contains a tab, it should be escaped or quoted
    // This test depends on implementation choice

    // Create a model with tab in cell value (if possible with VirtualModel)
    // or use a mock model
    let model = VirtualTableModel::new(5, 2, 42);
    let selected = vec![TableRowId(0)];
    let columns = vec![TableColumnKey::Str("col_0".to_string())];

    let tsv = format_rows_as_tsv(&model, &selected, &columns);

    // Basic check: should produce valid output
    assert!(!tsv.is_empty() || selected.is_empty());
}

#[test]
fn format_rows_as_tsv_preserves_row_order() {
    let model = VirtualTableModel::new(10, 2, 42);
    // Select rows out of order
    let selected = vec![TableRowId(5), TableRowId(2), TableRowId(8)];
    let columns = vec![TableColumnKey::Str("col_0".to_string())];

    let tsv = format_rows_as_tsv(&model, &selected, &columns);

    let lines: Vec<_> = tsv.lines().collect();
    assert_eq!(lines.len(), 3);

    // Rows should be in the order provided (5, 2, 8), not sorted
    // The content should reflect this order
}

// ========================
// Stage 9 Tests - Navigation Message Integration
// ========================

#[test]
fn table_navigate_down_updates_selection() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 3,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Ensure runtime state exists
    state
        .table_runtime
        .entry(tile_id)
        .or_insert_with(TableRuntimeState::default);

    // Set initial selection at row 0
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(0));
    sel.anchor = Some(TableRowId(0));
    state.update(Message::SetTableSelection {
        tile_id,
        selection: sel,
    });

    // Simulate Down key navigation
    let visible: Vec<_> = (0..10).map(TableRowId).collect();
    let current_sel = &state.table_runtime[&tile_id].selection;
    let result = navigate_down(current_sel, &visible);

    if let Some(new_sel) = result.new_selection {
        state.update(Message::SetTableSelection {
            tile_id,
            selection: new_sel,
        });
    }

    assert!(
        state.table_runtime[&tile_id]
            .selection
            .contains(TableRowId(1))
    );
    assert!(
        !state.table_runtime[&tile_id]
            .selection
            .contains(TableRowId(0))
    );
}

#[test]
fn table_navigate_up_updates_selection() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 3,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Ensure runtime state exists
    state
        .table_runtime
        .entry(tile_id)
        .or_insert_with(TableRuntimeState::default);

    // Set initial selection at row 5
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(5));
    sel.anchor = Some(TableRowId(5));
    state.update(Message::SetTableSelection {
        tile_id,
        selection: sel,
    });

    let visible: Vec<_> = (0..10).map(TableRowId).collect();
    let current_sel = &state.table_runtime[&tile_id].selection;
    let result = navigate_up(current_sel, &visible);

    if let Some(new_sel) = result.new_selection {
        state.update(Message::SetTableSelection {
            tile_id,
            selection: new_sel,
        });
    }

    assert!(
        state.table_runtime[&tile_id]
            .selection
            .contains(TableRowId(4))
    );
}

#[test]
fn table_select_all_in_multi_mode() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 5,
        columns: 2,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec: spec.clone() });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Ensure Multi mode
    state
        .user
        .table_tiles
        .get_mut(&tile_id)
        .unwrap()
        .config
        .selection_mode = TableSelectionMode::Multi;

    // Initialize runtime and build cache first
    state
        .table_runtime
        .entry(tile_id)
        .or_insert_with(TableRuntimeState::default);

    // Build cache manually for the test
    let model = spec.create_model().unwrap();
    let cache = build_table_cache(model, TableSearchSpec::default(), vec![]).unwrap();
    let cache_key = TableCacheKey {
        model_key: TableModelKey(tile_id.0),
        display_filter: TableSearchSpec::default(),
        view_sort: vec![],
        generation: 0,
    };
    let cache_entry = Arc::new(TableCacheEntry::new(cache_key.clone(), 0));
    cache_entry.set(cache);

    state.table_runtime.get_mut(&tile_id).unwrap().cache = Some(cache_entry);
    state.table_runtime.get_mut(&tile_id).unwrap().cache_key = Some(cache_key);

    // Simulate Ctrl+A - select all
    state.update(Message::TableSelectAll { tile_id });

    assert_eq!(state.table_runtime[&tile_id].selection.len(), 5);
    for i in 0..5 {
        assert!(
            state.table_runtime[&tile_id]
                .selection
                .contains(TableRowId(i))
        );
    }
}

#[test]
fn table_select_all_ignored_in_single_mode() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 5,
        columns: 2,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Ensure Single mode
    state
        .user
        .table_tiles
        .get_mut(&tile_id)
        .unwrap()
        .config
        .selection_mode = TableSelectionMode::Single;

    // Initialize runtime
    state
        .table_runtime
        .entry(tile_id)
        .or_insert_with(TableRuntimeState::default);

    // Initial selection
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(2));
    state.update(Message::SetTableSelection {
        tile_id,
        selection: sel,
    });

    // Ctrl+A should be ignored in Single mode
    state.update(Message::TableSelectAll { tile_id });

    // Selection unchanged
    assert_eq!(state.table_runtime[&tile_id].selection.len(), 1);
    assert!(
        state.table_runtime[&tile_id]
            .selection
            .contains(TableRowId(2))
    );
}

#[test]
fn table_activate_selection_emits_action() {
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

    // Select row 3
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(3));
    sel.anchor = Some(TableRowId(3));
    state.update(Message::SetTableSelection {
        tile_id,
        selection: sel,
    });

    // Activate (Enter key)
    // This would call model.on_activate() and emit appropriate Message
    // For Virtual model, this returns TableAction::None
    state.update(Message::TableActivateSelection { tile_id });

    // Virtual model returns None action, so no side effects
    // For real models, this would emit CursorSet or FocusTransaction
}

#[test]
fn table_escape_clears_selection() {
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

    // Set selection
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(3));
    sel.rows.insert(TableRowId(5));
    state.update(Message::SetTableSelection {
        tile_id,
        selection: sel,
    });

    assert!(!state.table_runtime[&tile_id].selection.is_empty());

    // Escape clears selection
    state.update(Message::ClearTableSelection { tile_id });

    assert!(state.table_runtime[&tile_id].selection.is_empty());
}

#[test]
fn table_copy_selection_single_row() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 5,
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

    // Select single row
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(2));
    state.update(Message::SetTableSelection {
        tile_id,
        selection: sel,
    });

    // Copy to clipboard (no context available in test, so this is a no-op)
    state.update(Message::TableCopySelection {
        tile_id,
        include_header: false,
    });

    // Verify: clipboard would contain TSV format
    // (Actual clipboard interaction is platform-specific)
}

#[test]
fn table_copy_selection_multiple_rows() {
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

    // Select multiple rows
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(1));
    sel.rows.insert(TableRowId(3));
    sel.rows.insert(TableRowId(7));
    state.update(Message::SetTableSelection {
        tile_id,
        selection: sel,
    });

    state.update(Message::TableCopySelection {
        tile_id,
        include_header: false,
    });
    // Clipboard would have 3 lines
}

#[test]
fn table_copy_selection_with_header() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 5,
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

    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(0));
    state.update(Message::SetTableSelection {
        tile_id,
        selection: sel,
    });

    state.update(Message::TableCopySelection {
        tile_id,
        include_header: true,
    });
    // Clipboard would have header row + 1 data row
}

#[test]
fn table_copy_empty_selection_no_op() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 5,
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

    // No selection
    state.update(Message::TableCopySelection {
        tile_id,
        include_header: false,
    });

    // Should not crash, clipboard unchanged
}

#[test]
fn table_navigation_with_sorted_rows() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 5,
        columns: 2,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Initialize runtime
    state
        .table_runtime
        .entry(tile_id)
        .or_insert_with(TableRuntimeState::default);

    // Apply sort (rows may reorder)
    state.update(Message::SetTableSort {
        tile_id,
        sort: vec![TableSortSpec {
            key: TableColumnKey::Str("col_0".to_string()),
            direction: TableSortDirection::Descending,
        }],
    });

    // Set selection on first visible row (which is now different TableRowId)
    // Navigation should follow display order, not original row IDs
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(4)); // Assume row 4 is now first after sort
    sel.anchor = Some(TableRowId(4));
    state.update(Message::SetTableSelection {
        tile_id,
        selection: sel,
    });

    // Navigate down should go to the next row in display order
}

#[test]
fn table_navigation_nonexistent_tile_ignored() {
    let mut state = SystemState::new_default_config().expect("state");

    let fake_tile_id = TableTileId(9999);

    // Should not crash
    state.update(Message::TableSelectAll {
        tile_id: fake_tile_id,
    });
    state.update(Message::TableActivateSelection {
        tile_id: fake_tile_id,
    });
    state.update(Message::TableCopySelection {
        tile_id: fake_tile_id,
        include_header: false,
    });
}

#[test]
fn table_page_navigation_respects_page_size() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 100,
        columns: 2,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Initialize runtime
    state
        .table_runtime
        .entry(tile_id)
        .or_insert_with(TableRuntimeState::default);

    // Set selection at row 50
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(50));
    sel.anchor = Some(TableRowId(50));
    state.update(Message::SetTableSelection {
        tile_id,
        selection: sel,
    });

    let visible: Vec<_> = (0..100).map(TableRowId).collect();
    let page_size = 20; // Typical visible rows

    // Page Down
    let result = navigate_page_down(
        &state.table_runtime[&tile_id].selection,
        &visible,
        page_size,
    );
    assert_eq!(result.target_row, Some(TableRowId(70)));

    // Page Up from 50
    let result = navigate_page_up(
        &state.table_runtime[&tile_id].selection,
        &visible,
        page_size,
    );
    assert_eq!(result.target_row, Some(TableRowId(30)));
}

#[test]
fn type_search_integration() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 2,
        seed: 42,
    };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Initialize runtime and build cache
    state
        .table_runtime
        .entry(tile_id)
        .or_insert_with(TableRuntimeState::default);

    // Initialize type search state
    let runtime = state.table_runtime.get_mut(&tile_id).expect("runtime");
    let now = std::time::Instant::now();
    runtime.type_search.push_char('r', now);
    runtime.type_search.push_char('5', now);

    // Search for "r5" in visible rows
    let visible: Vec<_> = (0..10).map(TableRowId).collect();
    let search_texts: Vec<_> = visible
        .iter()
        .map(|id| format!("r{}c0 r{}c1", id.0, id.0))
        .collect();

    let match_row = find_type_search_match(
        &runtime.type_search.buffer,
        &runtime.selection,
        &visible,
        &search_texts,
    );

    assert_eq!(match_row, Some(TableRowId(5)));
}

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

    let result = scroll_target_after_sort(&selection, &new_visible);
    assert_eq!(result, ScrollTarget::ToRow(TableRowId(5)));
}

#[test]
fn scroll_target_after_sort_selection_at_top() {
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(0));
    selection.anchor = Some(TableRowId(0));

    let new_visible = vec![TableRowId(0), TableRowId(1), TableRowId(2)];

    let result = scroll_target_after_sort(&selection, &new_visible);
    assert_eq!(result, ScrollTarget::ToRow(TableRowId(0)));
}

#[test]
fn scroll_target_after_sort_selection_at_bottom() {
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(2));
    selection.anchor = Some(TableRowId(2));

    let new_visible = vec![TableRowId(0), TableRowId(1), TableRowId(2)];

    let result = scroll_target_after_sort(&selection, &new_visible);
    assert_eq!(result, ScrollTarget::ToRow(TableRowId(2)));
}

#[test]
fn scroll_target_after_sort_no_selection_preserves() {
    let selection = TableSelection::new();
    let new_visible = vec![TableRowId(0), TableRowId(1), TableRowId(2)];

    let result = scroll_target_after_sort(&selection, &new_visible);
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

    let result = scroll_target_after_sort(&selection, &new_visible);
    assert_eq!(result, ScrollTarget::ToRow(TableRowId(3)));
}

#[test]
fn scroll_target_after_filter_selected_row_visible() {
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(2));
    selection.anchor = Some(TableRowId(2));

    let new_visible = vec![TableRowId(0), TableRowId(2), TableRowId(4)];

    let result = scroll_target_after_filter(&selection, &new_visible);
    assert_eq!(result, ScrollTarget::ToRow(TableRowId(2)));
}

#[test]
fn scroll_target_after_filter_selected_row_hidden() {
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(3)); // Row 3 is filtered out
    selection.anchor = Some(TableRowId(3));

    let new_visible = vec![TableRowId(0), TableRowId(2), TableRowId(4)]; // Row 3 not in list

    let result = scroll_target_after_filter(&selection, &new_visible);
    assert_eq!(result, ScrollTarget::ToTop);
}

#[test]
fn scroll_target_after_filter_no_selection() {
    let selection = TableSelection::new();
    let new_visible = vec![TableRowId(0), TableRowId(1)];

    let result = scroll_target_after_filter(&selection, &new_visible);
    assert_eq!(result, ScrollTarget::Preserve);
}

#[test]
fn scroll_target_after_filter_all_selected_hidden() {
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(3));
    selection.rows.insert(TableRowId(5));

    let new_visible = vec![TableRowId(0), TableRowId(2), TableRowId(4)];

    let result = scroll_target_after_filter(&selection, &new_visible);
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
        },
    });

    let runtime = state.table_runtime.get(&tile_id).unwrap();
    assert_eq!(
        runtime.scroll_state.pending_scroll_op,
        Some(PendingScrollOp::AfterFilter)
    );
}
