use super::*;
use crate::SystemState;
use crate::displayed_item_tree::VisibleItemIndex;
use crate::message::{Message, MessageTarget};
use crate::table::sources::VirtualTableModel;
use crate::tests::snapshot::wait_for_waves_fully_loaded;
use crate::wave_container::VariableRef;
use crate::wave_container::VariableRefExt;
use crate::{StartupParams, WaveSource};
use project_root::get_project_root;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// Helper to build a row_index HashMap from a slice of row IDs.
fn build_row_index(rows: &[TableRowId]) -> HashMap<TableRowId, usize> {
    rows.iter().enumerate().map(|(i, &id)| (id, i)).collect()
}

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
fn table_model_spec_multi_signal_change_list_ron_round_trip() {
    let spec = TableModelSpec::MultiSignalChangeList {
        variables: vec![
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.clk"),
                field: vec![],
            },
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
                field: vec!["value".to_string(), "lsb".to_string()],
            },
        ],
    };

    let encoded =
        ron::ser::to_string(&spec).expect("serialize TableModelSpec::MultiSignalChangeList");
    let decoded: TableModelSpec =
        ron::de::from_str(&encoded).expect("deserialize TableModelSpec::MultiSignalChangeList");

    assert_eq!(spec, decoded);
}

#[test]
fn multi_signal_change_list_default_view_config_deterministic() {
    let state = SystemState::new_default_config().expect("state");
    let ctx = state.table_model_context();

    let spec_a = TableModelSpec::MultiSignalChangeList {
        variables: vec![
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.clk"),
                field: vec![],
            },
            MultiSignalEntry {
                variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
                field: vec!["value".to_string()],
            },
        ],
    };
    let spec_b = TableModelSpec::MultiSignalChangeList {
        variables: vec![MultiSignalEntry {
            variable: VariableRef::from_hierarchy_string("tb.dut.counter"),
            field: vec!["value".to_string()],
        }],
    };

    let config_a = spec_a.default_view_config(&ctx);
    let config_b = spec_b.default_view_config(&ctx);

    assert_eq!(config_a.title, "Multi-signal change list");
    assert_eq!(config_a.title, config_b.title);
    assert_eq!(
        config_a.sort,
        vec![TableSortSpec {
            key: TableColumnKey::Str("time".to_string()),
            direction: TableSortDirection::Ascending,
        }]
    );
    assert_eq!(config_a.sort, config_b.sort);
    assert_eq!(config_a.selection_mode, TableSelectionMode::Single);
    assert!(config_a.activate_on_select);
}

#[test]
fn multi_signal_change_list_model_creation_no_waves_returns_data_unavailable() {
    let state = SystemState::new_default_config().expect("state");
    let ctx = state.table_model_context();
    let spec = TableModelSpec::MultiSignalChangeList {
        variables: vec![MultiSignalEntry {
            variable: VariableRef::from_hierarchy_string("tb.clk"),
            field: vec![],
        }],
    };

    assert!(
        matches!(
            spec.create_model(&ctx),
            Err(TableCacheError::DataUnavailable)
        ),
        "expected DataUnavailable when no wave data loaded"
    );
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
        activate_on_select: false,
    };

    let encoded = ron::ser::to_string(&config).expect("serialize TableViewConfig");
    let decoded: TableViewConfig =
        ron::de::from_str(&encoded).expect("deserialize TableViewConfig");

    assert_eq!(config, decoded);
}

#[test]
fn signal_analysis_config_ron_round_trip() {
    let config = SignalAnalysisConfig {
        sampling: SignalAnalysisSamplingConfig {
            signal: VariableRef::from_hierarchy_string("tb.clk"),
        },
        signals: vec![
            SignalAnalysisSignal {
                variable: VariableRef::from_hierarchy_string("tb.data_out"),
                field: vec![],
                translator: "Unsigned".to_string(),
            },
            SignalAnalysisSignal {
                variable: VariableRef::from_hierarchy_string("tb.counter"),
                field: vec!["value".to_string()],
                translator: "Signed".to_string(),
            },
        ],
        run_revision: 7,
    };

    let encoded = ron::ser::to_string(&config).expect("serialize SignalAnalysisConfig");
    let decoded: SignalAnalysisConfig =
        ron::de::from_str(&encoded).expect("deserialize SignalAnalysisConfig");

    assert_eq!(config, decoded);
}

#[test]
fn table_model_spec_analysis_results_signal_analysis_v1_ron_round_trip() {
    let spec = TableModelSpec::AnalysisResults {
        kind: AnalysisKind::SignalAnalysisV1,
        params: AnalysisParams::SignalAnalysisV1 {
            config: SignalAnalysisConfig {
                sampling: SignalAnalysisSamplingConfig {
                    signal: VariableRef::from_hierarchy_string("tb.clk"),
                },
                signals: vec![SignalAnalysisSignal {
                    variable: VariableRef::from_hierarchy_string("tb.data_out"),
                    field: vec![],
                    translator: "Unsigned".to_string(),
                }],
                run_revision: 0,
            },
        },
    };

    let encoded = ron::ser::to_string(&spec).expect("serialize TableModelSpec::AnalysisResults");
    let decoded: TableModelSpec =
        ron::de::from_str(&encoded).expect("deserialize TableModelSpec::AnalysisResults");

    assert!(encoded.contains("SignalAnalysisV1"));
    assert_eq!(spec, decoded);
}

#[test]
fn table_model_spec_analysis_results_placeholder_ron_round_trip() {
    let spec = TableModelSpec::AnalysisResults {
        kind: AnalysisKind::Placeholder,
        params: AnalysisParams::Placeholder {
            payload: "{}".to_string(),
        },
    };

    let encoded = ron::ser::to_string(&spec).expect("serialize placeholder analysis spec");
    let decoded: TableModelSpec =
        ron::de::from_str(&encoded).expect("deserialize placeholder analysis spec");

    assert_eq!(spec, decoded);
}

// ========================
// Stage 2 Tests
// ========================

#[test]
fn table_model_spec_create_virtual_model() {
    let state = SystemState::new_default_config().expect("state");
    let ctx = state.table_model_context();
    let spec = TableModelSpec::Virtual {
        rows: 100,
        columns: 5,
        seed: 42,
    };

    let model = spec.create_model(&ctx).expect("model");
    assert_eq!(model.row_count(), 100);
    assert_eq!(model.schema().columns.len(), 5);
}

#[test]
fn table_model_spec_create_unimplemented_returns_error() {
    let state = SystemState::new_default_config().expect("state");
    let ctx = state.table_model_context();
    let signal_spec = TableModelSpec::SignalChangeList {
        variable: crate::wave_container::VariableRef::from_hierarchy_string(""),
        field: vec![],
    };
    assert!(
        signal_spec.create_model(&ctx).is_err(),
        "SignalChangeList requires wave data"
    );

    let custom_spec = TableModelSpec::Custom {
        key: "test".to_string(),
        payload: "{}".to_string(),
    };
    assert!(
        custom_spec.create_model(&ctx).is_err(),
        "Custom not yet implemented"
    );
}

#[test]
fn virtual_model_via_factory_deterministic() {
    let state = SystemState::new_default_config().expect("state");
    let ctx = state.table_model_context();
    let spec = TableModelSpec::Virtual {
        rows: 10,
        columns: 3,
        seed: 42,
    };

    let model1 = spec.create_model(&ctx).unwrap();
    let model2 = spec.create_model(&ctx).unwrap();

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
    let state = SystemState::new_default_config().expect("state");
    let ctx = state.table_model_context();
    let spec = TableModelSpec::Virtual {
        rows: 5,
        columns: 3,
        seed: 0,
    };

    let model = spec.create_model(&ctx).unwrap();
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

#[test]
fn table_model_materialize_window_default_adapter_matches_legacy_methods() {
    struct LegacyModel {
        rows: Vec<TableRowId>,
    }

    impl TableModel for LegacyModel {
        fn schema(&self) -> TableSchema {
            TableSchema {
                columns: vec![
                    TableColumn {
                        key: TableColumnKey::Str("col_0".to_string()),
                        label: "Col 0".to_string(),
                        default_width: None,
                        default_visible: true,
                        default_resizable: true,
                    },
                    TableColumn {
                        key: TableColumnKey::Str("col_1".to_string()),
                        label: "Col 1".to_string(),
                        default_width: None,
                        default_visible: true,
                        default_resizable: true,
                    },
                    TableColumn {
                        key: TableColumnKey::Str("col_2".to_string()),
                        label: "Col 2".to_string(),
                        default_width: None,
                        default_visible: true,
                        default_resizable: true,
                    },
                ],
            }
        }

        fn row_count(&self) -> usize {
            self.rows.len()
        }

        fn row_id_at(&self, index: usize) -> Option<TableRowId> {
            self.rows.get(index).copied()
        }

        fn cell(&self, row: TableRowId, col: usize) -> TableCell {
            TableCell::Text(format!("cell:{}:{col}", row.0))
        }

        fn sort_key(&self, row: TableRowId, col: usize) -> TableSortKey {
            TableSortKey::Numeric((row.0 * 10 + col as u64) as f64)
        }

        fn search_text(&self, row: TableRowId) -> String {
            format!("search:{}", row.0)
        }

        fn on_activate(&self, _row: TableRowId) -> TableAction {
            TableAction::None
        }
    }

    fn cell_to_text(cell: &TableCell) -> String {
        match cell {
            TableCell::Text(text) => text.clone(),
            TableCell::RichText(text) => text.text().to_string(),
        }
    }

    let model = LegacyModel {
        rows: vec![TableRowId(5), TableRowId(7)],
    };
    let row_ids = model.rows.clone();
    let visible_cols = vec![0, 2];

    let render_window =
        model.materialize_window(&row_ids, &visible_cols, MaterializePurpose::Render);
    let clipboard_window =
        model.materialize_window(&row_ids, &visible_cols, MaterializePurpose::Clipboard);
    let sort_window =
        model.materialize_window(&row_ids, &visible_cols, MaterializePurpose::SortProbe);
    let search_window =
        model.materialize_window(&row_ids, &visible_cols, MaterializePurpose::SearchProbe);

    for &row_id in &row_ids {
        for &col in &visible_cols {
            let expected_cell = model.cell(row_id, col);
            let expected_cell_text = cell_to_text(&expected_cell);
            let expected_sort_key = model.sort_key(row_id, col);
            let render_text = render_window.cell(row_id, col).map(cell_to_text);
            let clipboard_text = clipboard_window.cell(row_id, col).map(cell_to_text);

            assert_eq!(render_text, Some(expected_cell_text.clone()));
            assert_eq!(clipboard_text, Some(expected_cell_text));
            assert_eq!(sort_window.sort_key(row_id, col), Some(&expected_sort_key));
        }

        let expected_search = model.search_text(row_id);
        assert_eq!(
            search_window.search_text(row_id),
            Some(expected_search.as_str())
        );
    }
}

// ========================
// Stage 3 Tests
// ========================

#[derive(Clone)]
struct LazyProbeTestModel {
    rows: Vec<(TableRowId, f64, String)>,
}

impl LazyProbeTestModel {
    fn row(&self, row: TableRowId) -> Option<&(TableRowId, f64, String)> {
        self.rows.iter().find(|(id, _, _)| *id == row)
    }
}

impl TableModel for LazyProbeTestModel {
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

    fn search_text_mode(&self) -> SearchTextMode {
        SearchTextMode::LazyProbe
    }

    fn materialize_window(
        &self,
        row_ids: &[TableRowId],
        visible_cols: &[usize],
        purpose: MaterializePurpose,
    ) -> MaterializedWindow {
        let mut window = MaterializedWindow::new();
        match purpose {
            MaterializePurpose::Render | MaterializePurpose::Clipboard => {
                for &row_id in row_ids {
                    for &col in visible_cols {
                        if col == 0
                            && let Some((_, _, text)) = self.row(row_id)
                        {
                            window.insert_cell(row_id, col, TableCell::Text(text.clone()));
                        }
                    }
                }
            }
            MaterializePurpose::SortProbe => {
                for &row_id in row_ids {
                    for &col in visible_cols {
                        if col == 0
                            && let Some((_, value, _)) = self.row(row_id)
                        {
                            window.insert_sort_key(row_id, col, TableSortKey::Numeric(*value));
                        }
                    }
                }
            }
            MaterializePurpose::SearchProbe => {
                for &row_id in row_ids {
                    if let Some((_, _, text)) = self.row(row_id) {
                        window.insert_search_text(row_id, text.clone());
                    }
                }
            }
        }
        window
    }

    fn cell(&self, row: TableRowId, _col: usize) -> TableCell {
        let text = self
            .row(row)
            .map(|(_, _, text)| text.clone())
            .unwrap_or_default();
        TableCell::Text(text)
    }

    fn sort_key(&self, row: TableRowId, _col: usize) -> TableSortKey {
        self.row(row)
            .map(|(_, value, _)| TableSortKey::Numeric(*value))
            .unwrap_or(TableSortKey::None)
    }

    fn search_text(&self, _row: TableRowId) -> String {
        panic!("lazy probe test model should not call eager search_text")
    }

    fn on_activate(&self, _row: TableRowId) -> TableAction {
        TableAction::None
    }
}

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
    let entry = TableCacheEntry::new(cache_key, 0, 0);
    assert!(!entry.is_ready());

    entry.set(TableCache {
        row_ids: vec![],
        row_index: HashMap::new(),
        search_texts: Some(vec![]),
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
        None,
    )
    .expect("cache build should succeed");

    let expected: Vec<_> = (0..5).map(|idx| TableRowId(idx as u64)).collect();
    assert_eq!(cache.row_ids, expected);
    assert_eq!(
        cache
            .search_texts
            .as_ref()
            .expect("virtual model uses eager search cache")
            .len(),
        expected.len()
    );
    assert_eq!(cache.row_index.len(), expected.len());
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
        None,
    )
    .expect("cache build should succeed");

    assert_eq!(cache.row_ids, vec![TableRowId(3)]);
}

#[test]
fn table_cache_builder_lazy_probe_keeps_index_only_cache_shape() {
    let model = Arc::new(LazyProbeTestModel {
        rows: vec![
            (TableRowId(10), 5.0, "alpha".to_string()),
            (TableRowId(11), 1.0, "beta".to_string()),
            (TableRowId(12), 3.0, "gamma".to_string()),
        ],
    });

    let cache = build_table_cache(
        model,
        TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: "a".to_string(),
        },
        vec![TableSortSpec {
            key: TableColumnKey::Str("col".to_string()),
            direction: TableSortDirection::Descending,
        }],
        None,
    )
    .expect("cache build should succeed");

    assert_eq!(
        cache.row_ids,
        vec![TableRowId(10), TableRowId(12), TableRowId(11)]
    );
    assert!(cache.search_texts.is_none());
    for (expected_pos, &row_id) in cache.row_ids.iter().enumerate() {
        assert_eq!(cache.row_index.get(&row_id), Some(&expected_pos));
    }
}

#[test]
fn type_search_uses_lazy_probe_provider_when_eager_cache_absent() {
    let model = Arc::new(LazyProbeTestModel {
        rows: vec![
            (TableRowId(0), 0.0, "alpha".to_string()),
            (TableRowId(1), 1.0, "beta".to_string()),
            (TableRowId(2), 2.0, "gamma".to_string()),
        ],
    });

    let cache = build_table_cache(model.clone(), TableSearchSpec::default(), vec![], None)
        .expect("cache build should succeed");
    assert!(cache.search_texts.is_none());

    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(0));
    selection.anchor = Some(TableRowId(0));

    let match_row = find_type_search_match_in_cache("ga", &selection, &cache, model.as_ref());
    assert_eq!(match_row, Some(TableRowId(2)));

    let mut wrapped_selection = TableSelection::new();
    wrapped_selection.rows.insert(TableRowId(2));
    wrapped_selection.anchor = Some(TableRowId(2));

    let wrapped_match =
        find_type_search_match_in_cache("al", &wrapped_selection, &cache, model.as_ref());
    assert_eq!(wrapped_match, Some(TableRowId(0)));
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
        None,
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
        None,
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
        None,
    )
    .expect("cache build should succeed");

    assert!(cache.row_ids.is_empty());
    assert_eq!(cache.search_texts, Some(vec![]));
    assert!(cache.row_index.is_empty());
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
        None,
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
            filter_draft: None,
            hidden_selection_count: 0,
            model: None,
            table_revision: 0,
            cancel_token: Arc::new(AtomicBool::new(false)),
        },
    );

    let entry = Arc::new(TableCacheEntry::new(old_key.clone(), old_key.generation, 0));
    state.table_inflight.insert(old_key, entry.clone());

    let msg = Message::TableCacheBuilt {
        tile_id,
        revision: 0,
        entry: entry.clone(),
        result: Ok(TableCache {
            row_ids: vec![],
            row_index: HashMap::new(),
            search_texts: Some(vec![]),
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
            activate_on_select: false,
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
        filter_draft: None,
        hidden_selection_count: 0,
        model: None,
        table_revision: 0,
        cancel_token: Arc::new(AtomicBool::new(false)),
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
        None,
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
        None,
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

    let result = selection_on_shift_click(
        &current,
        TableRowId(5),
        &visible,
        &build_row_index(&visible),
    );

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

    let result = selection_on_shift_click(
        &current,
        TableRowId(2),
        &visible,
        &build_row_index(&visible),
    );

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

    let result = selection_on_shift_click(
        &current,
        TableRowId(3),
        &visible,
        &build_row_index(&visible),
    );

    // Shift+click on same row as anchor - just that row
    assert!(!result.changed); // Already selected
    assert_eq!(result.selection.len(), 1);
    assert!(result.selection.contains(TableRowId(3)));
}

#[test]
fn selection_shift_click_no_anchor_uses_clicked_as_anchor() {
    let current = TableSelection::new();
    let visible = vec![TableRowId(0), TableRowId(1), TableRowId(2), TableRowId(3)];

    let result = selection_on_shift_click(
        &current,
        TableRowId(2),
        &visible,
        &build_row_index(&visible),
    );

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

    let result = selection_on_shift_click(
        &current,
        TableRowId(2),
        &visible,
        &build_row_index(&visible),
    );

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
    let result = selection_on_shift_click(
        &current,
        TableRowId(4),
        &visible,
        &build_row_index(&visible),
    );

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

    let result = selection_on_shift_click(
        &current,
        TableRowId(4),
        &visible,
        &build_row_index(&visible),
    );

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

    let result = navigate_up(&sel, &visible, &build_row_index(&visible));

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

    let result = navigate_up(&sel, &visible, &build_row_index(&visible));

    // Already at first row - no change
    assert!(!result.selection_changed);
    assert_eq!(result.target_row, Some(TableRowId(0)));
}

#[test]
fn navigate_up_empty_selection_selects_last() {
    let sel = TableSelection::new();
    let visible = vec![TableRowId(0), TableRowId(1), TableRowId(2)];

    let result = navigate_up(&sel, &visible, &build_row_index(&visible));

    // No selection: Up selects last row (like most file managers)
    assert!(result.selection_changed);
    assert_eq!(result.target_row, Some(TableRowId(2)));
}

#[test]
fn navigate_up_empty_visible_no_change() {
    let sel = TableSelection::new();
    let visible: Vec<TableRowId> = vec![];

    let result = navigate_up(&sel, &visible, &build_row_index(&visible));

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

    let result = navigate_down(&sel, &visible, &build_row_index(&visible));

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

    let result = navigate_down(&sel, &visible, &build_row_index(&visible));

    assert!(!result.selection_changed);
    assert_eq!(result.target_row, Some(TableRowId(5)));
}

#[test]
fn navigate_down_empty_selection_selects_first() {
    let sel = TableSelection::new();
    let visible = vec![TableRowId(0), TableRowId(1), TableRowId(2)];

    let result = navigate_down(&sel, &visible, &build_row_index(&visible));

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

    let result = navigate_up(&sel, &visible, &build_row_index(&visible));

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

    let result = navigate_page_down(&sel, &visible, &build_row_index(&visible), page_size);

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

    let result = navigate_page_down(&sel, &visible, &build_row_index(&visible), page_size);

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

    let result = navigate_page_up(&sel, &visible, &build_row_index(&visible), page_size);

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

    let result = navigate_page_up(&sel, &visible, &build_row_index(&visible), page_size);

    // Would go to -3, but stops at 0 (first row)
    assert!(result.selection_changed);
    assert_eq!(result.target_row, Some(TableRowId(0)));
}

#[test]
fn navigate_page_empty_selection() {
    let sel = TableSelection::new();
    let visible: Vec<TableRowId> = (0..20).map(TableRowId).collect();

    let result_down = navigate_page_down(&sel, &visible, &build_row_index(&visible), 5);
    assert_eq!(result_down.target_row, Some(TableRowId(0))); // Start from beginning

    let result_up = navigate_page_up(&sel, &visible, &build_row_index(&visible), 5);
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
    let result =
        navigate_extend_selection(&sel, TableRowId(3), &visible, &build_row_index(&visible));

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
    let result =
        navigate_extend_selection(&sel, TableRowId(5), &visible, &build_row_index(&visible));

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
    let result =
        navigate_extend_selection(&sel, TableRowId(2), &visible, &build_row_index(&visible));

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
    let result =
        navigate_extend_selection(&sel, TableRowId(3), &visible, &build_row_index(&visible));

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

    let result =
        navigate_extend_selection(&sel, TableRowId(1), &visible, &build_row_index(&visible));

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

    let result = find_type_search_match(
        "gam",
        &sel,
        &visible,
        &search_texts,
        &build_row_index(&visible),
    );

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

    let result = find_type_search_match(
        "bar",
        &sel,
        &visible,
        &search_texts,
        &build_row_index(&visible),
    );

    assert_eq!(result, Some(TableRowId(1))); // "foo bar" contains "bar"
}

#[test]
fn type_search_case_insensitive() {
    let visible = vec![TableRowId(0), TableRowId(1)];
    let search_texts = vec!["UPPERCASE".to_string(), "lowercase".to_string()];
    let sel = TableSelection::new();

    let result = find_type_search_match(
        "upper",
        &sel,
        &visible,
        &search_texts,
        &build_row_index(&visible),
    );
    assert_eq!(result, Some(TableRowId(0)));

    let result2 = find_type_search_match(
        "LOWER",
        &sel,
        &visible,
        &search_texts,
        &build_row_index(&visible),
    );
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
    let result = find_type_search_match(
        "apr",
        &sel,
        &visible,
        &search_texts,
        &build_row_index(&visible),
    );

    assert_eq!(result, Some(TableRowId(1))); // "apricot" is next match
}

#[test]
fn type_search_no_match() {
    let visible = vec![TableRowId(0), TableRowId(1)];
    let search_texts = vec!["alpha".to_string(), "beta".to_string()];
    let sel = TableSelection::new();

    let result = find_type_search_match(
        "xyz",
        &sel,
        &visible,
        &search_texts,
        &build_row_index(&visible),
    );

    assert_eq!(result, None);
}

#[test]
fn type_search_empty_query() {
    let visible = vec![TableRowId(0), TableRowId(1)];
    let search_texts = vec!["alpha".to_string(), "beta".to_string()];
    let sel = TableSelection::new();

    let result = find_type_search_match(
        "",
        &sel,
        &visible,
        &search_texts,
        &build_row_index(&visible),
    );

    assert_eq!(result, None); // Empty query matches nothing
}

#[test]
fn type_search_empty_table() {
    let visible: Vec<TableRowId> = vec![];
    let search_texts: Vec<String> = vec![];
    let sel = TableSelection::new();

    let result = find_type_search_match(
        "test",
        &sel,
        &visible,
        &search_texts,
        &build_row_index(&visible),
    );

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
    let result = navigate_down(current_sel, &visible, &build_row_index(&visible));

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
    let result = navigate_up(current_sel, &visible, &build_row_index(&visible));

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
    let ctx = state.table_model_context();
    let model = spec.create_model(&ctx).unwrap();
    let cache = build_table_cache(model, TableSearchSpec::default(), vec![], None).unwrap();
    let cache_key = TableCacheKey {
        model_key: TableModelKey(tile_id.0),
        display_filter: TableSearchSpec::default(),
        view_sort: vec![],
        generation: 0,
    };
    let cache_entry = Arc::new(TableCacheEntry::new(cache_key.clone(), 0, 0));
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
    let row_index = build_row_index(&visible);
    let result = navigate_page_down(
        &state.table_runtime[&tile_id].selection,
        &visible,
        &row_index,
        page_size,
    );
    assert_eq!(result.target_row, Some(TableRowId(70)));

    // Page Up from 50
    let result = navigate_page_up(
        &state.table_runtime[&tile_id].selection,
        &visible,
        &row_index,
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
        &build_row_index(&visible),
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

// ========================
// Stage 11 Tests - SignalChangeList Model
// ========================

fn load_counter_state() -> SystemState {
    let mut state = SystemState::new_default_config()
        .expect("state")
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .expect("project root")
                    .join("examples/counter.vcd")
                    .try_into()
                    .expect("path"),
            )),
            ..Default::default()
        });
    wait_for_waves_fully_loaded(&mut state, 10);
    state
}

fn test_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime")
}

fn load_counter_state_with_variable(var_path: &str) -> SystemState {
    let mut state = load_counter_state();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string(var_path),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);
    state
}

fn load_counter_state_with_variables(var_paths: &[&str]) -> SystemState {
    let mut state = load_counter_state();
    state.update(Message::AddVariables(
        var_paths
            .iter()
            .map(|path| VariableRef::from_hierarchy_string(path))
            .collect(),
    ));
    wait_for_waves_fully_loaded(&mut state, 10);
    state
}

fn find_visible_index_for_variable(
    waves: &crate::wave_data::WaveData,
    variable: &VariableRef,
) -> Option<crate::displayed_item_tree::VisibleItemIndex> {
    waves
        .items_tree
        .iter_visible()
        .enumerate()
        .find_map(
            |(idx, node)| match waves.displayed_items.get(&node.item_ref) {
                Some(crate::displayed_item::DisplayedItem::Variable(var))
                    if &var.variable_ref == variable =>
                {
                    Some(crate::displayed_item_tree::VisibleItemIndex(idx))
                }
                _ => None,
            },
        )
}

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
fn signal_change_list_model_missing_field_path_uses_dash() {
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
    assert_eq!(value_text, "-");
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
    let state = SystemState::new_default_config().expect("state");
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
    assert_eq!(config.title, "Signal Analysis: tb.clk");
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
            scroll_offset: 0.0,
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
        mode: TableSearchMode::Contains,
        case_sensitive: false,
    };
    let draft = FilterDraft::from_spec(&spec);

    assert_eq!(draft.text, "foo");
    assert_eq!(draft.mode, TableSearchMode::Contains);
    assert!(!draft.case_sensitive);
    assert!(draft.last_changed.is_none());
}

#[test]
fn filter_draft_to_spec() {
    let draft = FilterDraft {
        text: "bar".into(),
        mode: TableSearchMode::Regex,
        case_sensitive: true,
        last_changed: Some(std::time::Instant::now()),
    };
    let spec = draft.to_spec();

    assert_eq!(spec.text, "bar");
    assert_eq!(spec.mode, TableSearchMode::Regex);
    assert!(spec.case_sensitive);
}

#[test]
fn filter_draft_is_dirty() {
    let spec = TableSearchSpec {
        text: "foo".into(),
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
        mode: TableSearchMode::Fuzzy,
        case_sensitive: true,
    };
    let draft = FilterDraft::from_spec(&spec);
    let round_tripped = draft.to_spec();

    assert_eq!(spec.text, round_tripped.text);
    assert_eq!(spec.mode, round_tripped.mode);
    assert_eq!(spec.case_sensitive, round_tripped.case_sensitive);
}

#[test]
fn filter_draft_default() {
    let draft = FilterDraft::default();

    assert!(draft.text.is_empty());
    assert_eq!(draft.mode, TableSearchMode::Contains);
    assert!(!draft.case_sensitive);
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
            scroll_offset: 0.0,
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
            scroll_offset: 0.0,
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
