use super::*;

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
            column: None,
        },
        pinned_filters: vec![],
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
fn table_search_spec_round_trip_with_column_all_none() {
    let spec = TableSearchSpec {
        mode: TableSearchMode::Contains,
        case_sensitive: false,
        text: "needle".to_string(),
        column: None,
    };

    let encoded = ron::ser::to_string(&spec).expect("serialize TableSearchSpec");
    let decoded: TableSearchSpec =
        ron::de::from_str(&encoded).expect("deserialize TableSearchSpec");
    assert_eq!(decoded.column, None);
    assert_eq!(decoded, spec);
}

#[test]
fn table_search_spec_round_trip_with_specific_column() {
    let spec = TableSearchSpec {
        mode: TableSearchMode::Exact,
        case_sensitive: true,
        text: "READ".to_string(),
        column: Some(TableColumnKey::Str("action".to_string())),
    };

    let encoded = ron::ser::to_string(&spec).expect("serialize TableSearchSpec with column");
    let decoded: TableSearchSpec =
        ron::de::from_str(&encoded).expect("deserialize TableSearchSpec with column");
    assert_eq!(decoded, spec);
}

#[test]
fn table_view_config_deserialize_without_pinned_filters_defaults_empty() {
    let encoded = r#"
(
    title: "Example",
    columns: [],
    sort: [],
    display_filter: (
        mode: Contains,
        case_sensitive: false,
        text: "",
        column: None,
    ),
    selection_mode: Single,
    dense_rows: false,
    sticky_header: true,
    activate_on_select: false,
)
"#;

    let decoded: TableViewConfig =
        ron::de::from_str(encoded).expect("deserialize TableViewConfig without pinned filters");
    assert!(decoded.pinned_filters.is_empty());
}

#[test]
fn table_view_config_round_trip_with_pinned_filters() {
    let config = TableViewConfig {
        title: "Pinned Example".to_string(),
        columns: vec![],
        sort: vec![],
        display_filter: TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: String::new(),
            column: None,
        },
        pinned_filters: vec![
            TableSearchSpec {
                mode: TableSearchMode::Contains,
                case_sensitive: false,
                text: "Type".to_string(),
                column: Some(TableColumnKey::Str("type".to_string())),
            },
            TableSearchSpec {
                mode: TableSearchMode::Exact,
                case_sensitive: true,
                text: "READ".to_string(),
                column: Some(TableColumnKey::Str("action".to_string())),
            },
        ],
        selection_mode: TableSelectionMode::Single,
        dense_rows: false,
        sticky_header: true,
        activate_on_select: false,
    };

    let encoded =
        ron::ser::to_string(&config).expect("serialize TableViewConfig with pinned filters");
    let decoded: TableViewConfig =
        ron::de::from_str(&encoded).expect("deserialize TableViewConfig with pinned filters");
    assert_eq!(decoded, config);
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

#[derive(Clone)]
struct ColumnFilterTestModel {
    rows: Vec<(TableRowId, String, String)>,
}

impl ColumnFilterTestModel {
    fn row(&self, row: TableRowId) -> Option<&(TableRowId, String, String)> {
        self.rows.iter().find(|(id, _, _)| *id == row)
    }
}

impl TableModel for ColumnFilterTestModel {
    fn schema(&self) -> TableSchema {
        TableSchema {
            columns: vec![
                TableColumn {
                    key: TableColumnKey::Str("type".to_string()),
                    label: "Type".to_string(),
                    default_width: None,
                    default_visible: true,
                    default_resizable: true,
                },
                TableColumn {
                    key: TableColumnKey::Str("action".to_string()),
                    label: "Action".to_string(),
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
        self.rows.get(index).map(|(id, _, _)| *id)
    }

    fn cell(&self, row: TableRowId, col: usize) -> TableCell {
        match (self.row(row), col) {
            (Some((_, kind, _action)), 0) => TableCell::Text(kind.clone()),
            (Some((_, _kind, action)), 1) => TableCell::Text(action.clone()),
            _ => TableCell::Text(String::new()),
        }
    }

    fn sort_key(&self, _row: TableRowId, _col: usize) -> TableSortKey {
        TableSortKey::None
    }

    fn search_text(&self, row: TableRowId) -> String {
        self.row(row)
            .map(|(_, kind, action)| format!("{kind} {action}"))
            .unwrap_or_default()
    }

    fn on_activate(&self, _row: TableRowId) -> TableAction {
        TableAction::None
    }
}

fn build_column_filter_test_model() -> Arc<ColumnFilterTestModel> {
    Arc::new(ColumnFilterTestModel {
        rows: vec![
            (TableRowId(0), "Type".to_string(), "READ".to_string()),
            (TableRowId(1), "Type".to_string(), "write".to_string()),
            (TableRowId(2), "Event".to_string(), "READ".to_string()),
            (TableRowId(3), "Event".to_string(), "WRITE".to_string()),
        ],
    })
}

#[test]
fn table_cache_entry_ready_state() {
    let cache_key = TableCacheKey {
        model_key: TableModelKey(1),
        display_filter: TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: String::new(),
            column: None,
        },
        pinned_filters: vec![],
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
            column: None,
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
            column: None,
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
            column: None,
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
            column: None,
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
            column: None,
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
fn table_cache_builder_sorts_text_keys_naturally() {
    #[derive(Clone)]
    struct TextSortModel {
        rows: Vec<(TableRowId, String)>,
    }

    impl TableModel for TextSortModel {
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
                .map(|(_, text)| text.clone())
                .unwrap_or_default();
            TableCell::Text(text)
        }

        fn sort_key(&self, row: TableRowId, _col: usize) -> TableSortKey {
            self.rows
                .iter()
                .find(|(id, _)| *id == row)
                .map(|(_, text)| TableSortKey::Text(text.clone()))
                .unwrap_or(TableSortKey::None)
        }

        fn search_text(&self, row: TableRowId) -> String {
            self.rows
                .iter()
                .find(|(id, _)| *id == row)
                .map(|(_, text)| text.clone())
                .unwrap_or_default()
        }

        fn on_activate(&self, _row: TableRowId) -> TableAction {
            TableAction::None
        }
    }

    let model = Arc::new(TextSortModel {
        rows: vec![
            (TableRowId(0), "tx#1".to_string()),
            (TableRowId(1), "tx#11".to_string()),
            (TableRowId(2), "tx#2".to_string()),
            (TableRowId(3), "tx#10".to_string()),
            (TableRowId(4), "tx#9".to_string()),
        ],
    });

    let cache = build_table_cache(
        model.clone(),
        TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: String::new(),
            column: None,
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
        vec![
            TableRowId(0),
            TableRowId(2),
            TableRowId(4),
            TableRowId(3),
            TableRowId(1)
        ]
    );

    let cache_desc = build_table_cache(
        model,
        TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: String::new(),
            column: None,
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
        vec![
            TableRowId(1),
            TableRowId(3),
            TableRowId(4),
            TableRowId(2),
            TableRowId(0)
        ]
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
            column: None,
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
            column: None,
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
fn table_cache_builder_filters_column_contains() {
    let model = build_column_filter_test_model();
    let cache = build_table_cache_with_pinned_filters(
        model,
        TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: "Type".to_string(),
            column: Some(TableColumnKey::Str("type".to_string())),
        },
        vec![],
        vec![],
        None,
    )
    .expect("cache build should succeed");

    assert_eq!(cache.row_ids, vec![TableRowId(0), TableRowId(1)]);
}

#[test]
fn table_cache_builder_filters_column_exact_case_sensitive() {
    let model = build_column_filter_test_model();
    let cache = build_table_cache_with_pinned_filters(
        model,
        TableSearchSpec {
            mode: TableSearchMode::Exact,
            case_sensitive: true,
            text: "READ".to_string(),
            column: Some(TableColumnKey::Str("action".to_string())),
        },
        vec![],
        vec![],
        None,
    )
    .expect("cache build should succeed");

    assert_eq!(cache.row_ids, vec![TableRowId(0), TableRowId(2)]);
}

#[test]
fn table_cache_builder_filters_multiple_clauses_and_semantics() {
    let model = build_column_filter_test_model();
    let cache = build_table_cache_with_pinned_filters(
        model,
        TableSearchSpec::default(),
        vec![
            TableSearchSpec {
                mode: TableSearchMode::Exact,
                case_sensitive: true,
                text: "Type".to_string(),
                column: Some(TableColumnKey::Str("type".to_string())),
            },
            TableSearchSpec {
                mode: TableSearchMode::Contains,
                case_sensitive: false,
                text: "read".to_string(),
                column: Some(TableColumnKey::Str("action".to_string())),
            },
        ],
        vec![],
        None,
    )
    .expect("cache build should succeed");

    assert_eq!(cache.row_ids, vec![TableRowId(0)]);
}

#[test]
fn table_cache_builder_ignores_missing_column_clause() {
    let model = build_column_filter_test_model();
    let cache = build_table_cache_with_pinned_filters(
        model,
        TableSearchSpec::default(),
        vec![TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: "Type".to_string(),
            column: Some(TableColumnKey::Str("does_not_exist".to_string())),
        }],
        vec![],
        None,
    )
    .expect("cache build should succeed");

    assert_eq!(cache.row_ids.len(), 4);
}

#[test]
fn table_cache_builder_multiple_regex_invalid_fails() {
    let model = build_column_filter_test_model();
    let result = build_table_cache_with_pinned_filters(
        model,
        TableSearchSpec::default(),
        vec![
            TableSearchSpec {
                mode: TableSearchMode::Regex,
                case_sensitive: false,
                text: "READ|WRITE".to_string(),
                column: Some(TableColumnKey::Str("action".to_string())),
            },
            TableSearchSpec {
                mode: TableSearchMode::Regex,
                case_sensitive: false,
                text: "(".to_string(),
                column: Some(TableColumnKey::Str("action".to_string())),
            },
        ],
        vec![],
        None,
    );

    match result {
        Err(TableCacheError::InvalidSearch { pattern, .. }) => assert_eq!(pattern, "("),
        other => panic!("Expected invalid regex error, got {other:?}"),
    }
}

#[test]
fn table_cache_builder_duplicate_clauses_dedup_equivalent_result() {
    let spec = TableSearchSpec {
        mode: TableSearchMode::Contains,
        case_sensitive: false,
        text: "Type".to_string(),
        column: Some(TableColumnKey::Str("type".to_string())),
    };
    let base_cache = build_table_cache_with_pinned_filters(
        build_column_filter_test_model(),
        TableSearchSpec::default(),
        vec![spec.clone()],
        vec![],
        None,
    )
    .expect("cache build should succeed");
    let duplicated_cache = build_table_cache_with_pinned_filters(
        build_column_filter_test_model(),
        TableSearchSpec::default(),
        vec![spec.clone(), spec],
        vec![],
        None,
    )
    .expect("cache build should succeed");

    assert_eq!(duplicated_cache.row_ids, base_cache.row_ids);
    assert_eq!(duplicated_cache.search_texts, base_cache.search_texts);
}

#[test]
fn table_cache_builder_column_only_filter_still_populates_eager_search_texts() {
    let cache = build_table_cache_with_pinned_filters(
        build_column_filter_test_model(),
        TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: "Type".to_string(),
            column: Some(TableColumnKey::Str("type".to_string())),
        },
        vec![],
        vec![],
        None,
    )
    .expect("cache build should succeed");

    let search_texts = cache
        .search_texts
        .expect("eager model should populate search texts");
    assert_eq!(search_texts.len(), cache.row_ids.len());
    assert_eq!(
        search_texts,
        vec!["Type READ".to_string(), "Type write".to_string()]
    );
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
            column: None,
        },
        pinned_filters: vec![],
        view_sort: vec![],
        generation: 1,
    };
    let new_key = TableCacheKey {
        model_key: TableModelKey(1),
        display_filter: TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: "new".to_string(),
            column: None,
        },
        pinned_filters: vec![],
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
        model: None,
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
            pinned_filters: vec![],
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
            pinned_filters: vec![],
            view_sort: vec![],
            generation: 0,
        }),
        cache: None,
        last_error: None,
        selection: TableSelection::default(),

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
            column: None,
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
            column: None,
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
    assert!(spec.column.is_none());
}

#[test]
fn table_search_spec_is_active() {
    // Empty text means inactive
    let inactive = TableSearchSpec {
        mode: TableSearchMode::Contains,
        case_sensitive: false,
        text: String::new(),
        column: None,
    };
    assert!(inactive.text.is_empty());

    // Non-empty text means active
    let active = TableSearchSpec {
        mode: TableSearchMode::Contains,
        case_sensitive: false,
        text: "search".to_string(),
        column: None,
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
        column: None,
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
            column: None,
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
            column: None,
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

#[test]
fn set_table_pinned_filters_updates_config() {
    let mut state = SystemState::new_default_config().expect("state");
    state.update(Message::AddTableTile {
        spec: TableModelSpec::Virtual {
            rows: 10,
            columns: 3,
            seed: 42,
        },
    });
    let tile_id = *state.user.table_tiles.keys().next().expect("tile");

    let filters = vec![
        TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: "Type".to_string(),
            column: Some(TableColumnKey::Str("type".to_string())),
        },
        TableSearchSpec {
            mode: TableSearchMode::Exact,
            case_sensitive: true,
            text: "READ".to_string(),
            column: Some(TableColumnKey::Str("action".to_string())),
        },
    ];

    state.update(Message::SetTablePinnedFilters {
        tile_id,
        filters: filters.clone(),
    });

    assert_eq!(
        state.user.table_tiles[&tile_id].config.pinned_filters,
        filters
    );
}

#[test]
fn set_table_pinned_filters_nonexistent_tile_ignored() {
    let mut state = SystemState::new_default_config().expect("state");
    state.update(Message::SetTablePinnedFilters {
        tile_id: TableTileId(9999),
        filters: vec![TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: "Type".to_string(),
            column: Some(TableColumnKey::Str("type".to_string())),
        }],
    });

    assert!(state.user.table_tiles.is_empty());
}

#[test]
fn set_table_pinned_filters_dedupes_and_drops_empty() {
    let mut state = SystemState::new_default_config().expect("state");
    state.update(Message::AddTableTile {
        spec: TableModelSpec::Virtual {
            rows: 10,
            columns: 3,
            seed: 42,
        },
    });
    let tile_id = *state.user.table_tiles.keys().next().expect("tile");

    let duplicated = TableSearchSpec {
        mode: TableSearchMode::Contains,
        case_sensitive: false,
        text: "Type".to_string(),
        column: Some(TableColumnKey::Str("type".to_string())),
    };
    state.update(Message::SetTablePinnedFilters {
        tile_id,
        filters: vec![
            TableSearchSpec {
                mode: TableSearchMode::Contains,
                case_sensitive: false,
                text: String::new(),
                column: Some(TableColumnKey::Str("type".to_string())),
            },
            duplicated.clone(),
            duplicated.clone(),
        ],
    });

    assert_eq!(
        state.user.table_tiles[&tile_id].config.pinned_filters,
        vec![duplicated]
    );
}

#[test]
fn set_table_pinned_filters_sets_pending_scroll_op_after_filter() {
    let mut state = SystemState::new_default_config().expect("state");
    state.update(Message::AddTableTile {
        spec: TableModelSpec::Virtual {
            rows: 10,
            columns: 3,
            seed: 42,
        },
    });
    let tile_id = *state.user.table_tiles.keys().next().expect("tile");
    state.table_runtime.entry(tile_id).or_default();

    state.update(Message::SetTablePinnedFilters {
        tile_id,
        filters: vec![TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: "Type".to_string(),
            column: Some(TableColumnKey::Str("type".to_string())),
        }],
    });

    let runtime = state.table_runtime.get(&tile_id).expect("runtime");
    assert_eq!(
        runtime.scroll_state.pending_scroll_op,
        Some(PendingScrollOp::AfterFilter)
    );
}
