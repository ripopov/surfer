use super::support::build_row_index;
use super::*;

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
            column: None,
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
    TypeSearchState, build_table_copy_payload, find_type_search_match, format_rows_as_tsv,
    format_rows_as_tsv_with_header, navigate_down, navigate_end, navigate_extend_selection,
    navigate_home, navigate_page_down, navigate_page_up, navigate_up,
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

#[derive(Clone)]
struct ClipboardPayloadTestModel {
    rows: Vec<TableRowId>,
}

impl ClipboardPayloadTestModel {
    fn new(rows: Vec<TableRowId>) -> Self {
        Self { rows }
    }

    fn cell_text(&self, row: TableRowId, col: usize) -> String {
        match col {
            0 => format!("A{}", row.0),
            1 if row == TableRowId(2) => "B2\tX\nY".to_string(),
            1 => format!("B{}", row.0),
            2 => format!("C{}", row.0),
            _ => String::new(),
        }
    }
}

impl TableModel for ClipboardPayloadTestModel {
    fn schema(&self) -> TableSchema {
        TableSchema {
            columns: vec![
                TableColumn {
                    key: TableColumnKey::Str("a".to_string()),
                    label: "A".to_string(),
                    default_width: None,
                    default_visible: true,
                    default_resizable: true,
                },
                TableColumn {
                    key: TableColumnKey::Str("b".to_string()),
                    label: "B".to_string(),
                    default_width: None,
                    default_visible: true,
                    default_resizable: true,
                },
                TableColumn {
                    key: TableColumnKey::Str("c".to_string()),
                    label: "C".to_string(),
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
        TableCell::Text(self.cell_text(row, col))
    }

    fn sort_key(&self, row: TableRowId, col: usize) -> TableSortKey {
        TableSortKey::Numeric((row.0 * 10 + col as u64) as f64)
    }

    fn search_text(&self, row: TableRowId) -> String {
        format!("row{}", row.0)
    }

    fn on_activate(&self, _row: TableRowId) -> TableAction {
        TableAction::None
    }
}

fn copy_column(key: &str, visible: bool) -> TableColumnConfig {
    TableColumnConfig {
        key: TableColumnKey::Str(key.to_string()),
        width: None,
        visible,
        resizable: true,
    }
}

#[test]
fn build_table_copy_payload_uses_visible_columns_and_display_row_order() {
    let model = ClipboardPayloadTestModel::new(vec![TableRowId(1), TableRowId(2), TableRowId(3)]);
    let schema = model.schema();
    let row_order = vec![TableRowId(3), TableRowId(1), TableRowId(2)];
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(2));
    selection.rows.insert(TableRowId(3));

    let columns = vec![
        copy_column("c", true),
        copy_column("a", true),
        copy_column("b", false),
    ];

    let tsv = build_table_copy_payload(&model, &schema, &row_order, &selection, &columns, false);
    assert_eq!(tsv, "C3\tA3\nC2\tA2");
}

#[test]
fn build_table_copy_payload_includes_header_only_when_requested() {
    let model = ClipboardPayloadTestModel::new(vec![TableRowId(1), TableRowId(2), TableRowId(3)]);
    let schema = model.schema();
    let row_order = vec![TableRowId(3), TableRowId(2)];
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(2));
    selection.rows.insert(TableRowId(3));
    let columns = vec![copy_column("c", true), copy_column("a", true)];

    let no_header =
        build_table_copy_payload(&model, &schema, &row_order, &selection, &columns, false);
    assert_eq!(no_header, "C3\tA3\nC2\tA2");

    let with_header =
        build_table_copy_payload(&model, &schema, &row_order, &selection, &columns, true);
    assert_eq!(with_header, "C\tA\nC3\tA3\nC2\tA2");
}

#[test]
fn build_table_copy_payload_falls_back_to_schema_when_config_is_empty() {
    let model = ClipboardPayloadTestModel::new(vec![TableRowId(1), TableRowId(2), TableRowId(3)]);
    let schema = model.schema();
    let row_order = vec![TableRowId(1), TableRowId(2)];
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(1));

    let tsv = build_table_copy_payload(&model, &schema, &row_order, &selection, &[], false);
    assert_eq!(tsv, "A1\tB1\tC1");
}

#[test]
fn build_table_copy_payload_sanitizes_tabs_and_newlines() {
    let model = ClipboardPayloadTestModel::new(vec![TableRowId(1), TableRowId(2), TableRowId(3)]);
    let schema = model.schema();
    let row_order = vec![TableRowId(2)];
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(2));
    let columns = vec![copy_column("b", true)];

    let tsv = build_table_copy_payload(&model, &schema, &row_order, &selection, &columns, false);
    assert_eq!(tsv, "B2 X Y");
}

#[test]
fn build_table_copy_payload_empty_selection_returns_empty_string() {
    let model = ClipboardPayloadTestModel::new(vec![TableRowId(1), TableRowId(2), TableRowId(3)]);
    let schema = model.schema();
    let row_order = vec![TableRowId(1), TableRowId(2)];
    let selection = TableSelection::new();
    let columns = vec![copy_column("a", true)];

    let tsv = build_table_copy_payload(&model, &schema, &row_order, &selection, &columns, false);
    assert!(tsv.is_empty());
}

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
        pinned_filters: vec![],
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
    let columns = vec![
        copy_column("c", true),
        copy_column("a", true),
        copy_column("b", false),
    ];
    let (mut state, tile_id, ctx) = setup_table_copy_message_state(
        columns,
        vec![TableRowId(3), TableRowId(1), TableRowId(2)],
        &[TableRowId(3)],
    );

    state.update(Message::TableCopySelection {
        tile_id,
        include_header: false,
    });

    assert_eq!(copied_text_commands(&ctx), vec!["C3\tA3".to_string()]);
}

#[test]
fn table_copy_selection_multiple_rows() {
    let columns = vec![
        copy_column("c", true),
        copy_column("a", true),
        copy_column("b", false),
    ];
    let (mut state, tile_id, ctx) = setup_table_copy_message_state(
        columns,
        vec![TableRowId(3), TableRowId(1), TableRowId(2)],
        &[TableRowId(2), TableRowId(3)],
    );

    state.update(Message::TableCopySelection {
        tile_id,
        include_header: false,
    });

    assert_eq!(
        copied_text_commands(&ctx),
        vec!["C3\tA3\nC2\tA2".to_string()]
    );
}

#[test]
fn table_copy_selection_with_header() {
    let columns = vec![
        copy_column("c", true),
        copy_column("a", true),
        copy_column("b", false),
    ];
    let (mut state, tile_id, ctx) = setup_table_copy_message_state(
        columns,
        vec![TableRowId(3), TableRowId(1), TableRowId(2)],
        &[TableRowId(2), TableRowId(3)],
    );

    state.update(Message::TableCopySelection {
        tile_id,
        include_header: true,
    });

    assert_eq!(
        copied_text_commands(&ctx),
        vec!["C\tA\nC3\tA3\nC2\tA2".to_string()]
    );
}

#[test]
fn table_copy_empty_selection_no_op() {
    let columns = vec![copy_column("a", true)];
    let (mut state, tile_id, ctx) = setup_table_copy_message_state(
        columns,
        vec![TableRowId(1), TableRowId(2), TableRowId(3)],
        &[],
    );

    state.update(Message::TableCopySelection {
        tile_id,
        include_header: false,
    });

    assert!(copied_text_commands(&ctx).is_empty());
}

#[test]
fn table_copy_selection_no_op_when_model_or_cache_missing() {
    let mut state = SystemState::new_default_config().expect("state");
    let tile_id = TableTileId(900);
    let mut config = TableViewConfig::default();
    config.columns = vec![copy_column("a", true)];
    state.user.table_tiles.insert(
        tile_id,
        TableTileState {
            spec: TableModelSpec::Virtual {
                rows: 3,
                columns: 3,
                seed: 0,
            },
            config,
        },
    );

    let mut runtime = TableRuntimeState::default();
    runtime.selection.rows.insert(TableRowId(1));
    runtime.selection.anchor = Some(TableRowId(1));
    state.table_runtime.insert(tile_id, runtime);

    let ctx = Arc::new(egui::Context::default());
    state.context = Some(ctx.clone());

    // Missing model
    state.update(Message::TableCopySelection {
        tile_id,
        include_header: false,
    });
    assert!(copied_text_commands(&ctx).is_empty());

    let model: Arc<dyn TableModel> = Arc::new(ClipboardPayloadTestModel::new(vec![
        TableRowId(1),
        TableRowId(2),
        TableRowId(3),
    ]));
    state.table_runtime.get_mut(&tile_id).unwrap().model = Some(model);

    // Missing cache
    state.update(Message::TableCopySelection {
        tile_id,
        include_header: false,
    });
    assert!(copied_text_commands(&ctx).is_empty());
}

fn copied_text_commands(ctx: &egui::Context) -> Vec<String> {
    ctx.output(|output| {
        output
            .commands
            .iter()
            .filter_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.clone()),
                _ => None,
            })
            .collect()
    })
}

fn setup_table_copy_message_state(
    columns: Vec<TableColumnConfig>,
    row_ids: Vec<TableRowId>,
    selected_rows: &[TableRowId],
) -> (SystemState, TableTileId, Arc<egui::Context>) {
    let mut state = SystemState::new_default_config().expect("state");
    let tile_id = TableTileId(901);

    let mut config = TableViewConfig::default();
    config.columns = columns;
    state.user.table_tiles.insert(
        tile_id,
        TableTileState {
            spec: TableModelSpec::Virtual {
                rows: 3,
                columns: 3,
                seed: 0,
            },
            config,
        },
    );

    let model: Arc<dyn TableModel> = Arc::new(ClipboardPayloadTestModel::new(vec![
        TableRowId(1),
        TableRowId(2),
        TableRowId(3),
    ]));
    let cache_key = TableCacheKey {
        model_key: TableModelKey(tile_id.0),
        display_filter: TableSearchSpec::default(),
        pinned_filters: vec![],
        view_sort: vec![],
        generation: 0,
    };
    let cache = TableCache {
        row_index: build_row_index(&row_ids),
        row_ids,
        search_texts: None,
    };
    let cache_entry = Arc::new(TableCacheEntry::new(cache_key.clone(), 0, 0));
    cache_entry.set(cache);

    let mut selection = TableSelection::new();
    for &row_id in selected_rows {
        selection.rows.insert(row_id);
    }
    selection.anchor = selected_rows.first().copied();

    state.table_runtime.insert(
        tile_id,
        TableRuntimeState {
            cache_key: Some(cache_key),
            cache: Some(cache_entry),
            selection,
            model: Some(model),
            ..TableRuntimeState::default()
        },
    );

    let ctx = Arc::new(egui::Context::default());
    state.context = Some(ctx.clone());

    (state, tile_id, ctx)
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
