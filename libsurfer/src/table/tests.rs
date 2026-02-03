use super::*;
use crate::wave_container::VariableRefExt;

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
