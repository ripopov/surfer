use crate::tiles::{ItemListId, commands::SplitMode};

use crate::tile_kinds::waveform::{WaveformMessage, WaveformTileFile};
use crate::tiles::kind::*;
use crate::tiles::runtime::WorkspaceRuntime;
use crate::tiles::serde::TileFile;
use crate::{
    tile_kinds::waveform::{WaveformView, WaveformViewFile},
    viewport::Viewport,
};

fn two_linked_tiles() -> (
    crate::tiles::workspace::Workspace,
    WorkspaceRuntime,
    crate::tiles::TileId,
    crate::tiles::TileId,
) {
    use crate::tiles::{
        commands::WorkspaceCommand,
        layout::{Direction, Placement},
        workspace::Workspace,
    };
    let mut workspace = Workspace::default();
    let mut runtime = WorkspaceRuntime::default();
    workspace
        .apply_command(
            &mut runtime,
            WorkspaceCommand::CreateTile {
                kind: WAVEFORM.name.into(),
                placement: Placement::Root,
                focus: true,
            },
        )
        .unwrap();
    let first = workspace.layout.focused().unwrap();
    workspace
        .apply_command(
            &mut runtime,
            WorkspaceCommand::SplitTile {
                tile: first,
                dir: Direction::Right,
                mode: SplitMode::Linked,
            },
        )
        .unwrap();
    let second = workspace.layout.focused().unwrap();
    for (id, height) in [(first, 100.0), (second, 300.0)] {
        let TileKind::Waveform(tile) = &mut workspace.tiles.get_mut(&id).unwrap().kind else {
            unreachable!()
        };
        tile.view.viewport_height = height;
        tile.link_vertical_scroll = true;
    }
    let list = workspace.tiles[&first].kind.waveform_list().unwrap();
    let mut layout = workspace.item_lists[&list].layout_cache.borrow_mut();
    layout.signature = Some(1);
    layout.total_height = 500.0;
    drop(layout);
    (workspace, runtime, first, second)
}

fn offset(workspace: &crate::tiles::workspace::Workspace, id: crate::tiles::TileId) -> f32 {
    let TileKind::Waveform(tile) = &workspace.tiles[&id].kind else {
        unreachable!()
    };
    tile.view.scroll_offset
}

#[test]
fn linked_scroll_uses_visible_bounds_and_reclamps_when_a_tab_is_revealed() {
    use crate::tiles::{commands::WorkspaceCommand, layout::Placement};
    let (mut workspace, mut runtime, first, second) = two_linked_tiles();
    workspace
        .apply_tile_message(
            first,
            TileMessage::Waveform(WaveformMessage::ScrollTo(350.0)),
            None,
        )
        .unwrap();
    assert_eq!(
        (offset(&workspace, first), offset(&workspace, second)),
        (200.0, 200.0)
    );
    workspace
        .apply_command(
            &mut runtime,
            WorkspaceCommand::CreateTile {
                kind: WAVEFORM.name.into(),
                placement: Placement::TabAfter(second),
                focus: true,
            },
        )
        .unwrap();
    let independent = workspace.layout.focused().unwrap();
    workspace
        .apply_tile_message(
            first,
            TileMessage::Waveform(WaveformMessage::ScrollTo(350.0)),
            None,
        )
        .unwrap();
    assert_eq!(
        (offset(&workspace, first), offset(&workspace, second)),
        (350.0, 350.0)
    );
    assert_eq!(offset(&workspace, independent), 0.0);
    workspace
        .apply_command(&mut runtime, WorkspaceCommand::FocusTile(second))
        .unwrap();
    assert_eq!(
        (offset(&workspace, first), offset(&workspace, second)),
        (200.0, 200.0)
    );
}

#[test]
fn joining_adopts_group_offset_and_independent_split_leaves_group() {
    use crate::tiles::{commands::WorkspaceCommand, layout::Direction};
    let (mut workspace, mut runtime, first, second) = two_linked_tiles();
    workspace
        .apply_tile_message(
            second,
            TileMessage::Waveform(WaveformMessage::LinkVerticalScroll(false)),
            None,
        )
        .unwrap();
    workspace
        .apply_tile_message(
            first,
            TileMessage::Waveform(WaveformMessage::ScrollTo(300.0)),
            None,
        )
        .unwrap();
    assert_eq!(offset(&workspace, second), 0.0);
    workspace
        .apply_tile_message(
            second,
            TileMessage::Waveform(WaveformMessage::LinkVerticalScroll(true)),
            None,
        )
        .unwrap();
    assert_eq!(
        (offset(&workspace, first), offset(&workspace, second)),
        (200.0, 200.0)
    );
    workspace
        .apply_command(
            &mut runtime,
            WorkspaceCommand::SplitTile {
                tile: first,
                dir: Direction::Down,
                mode: SplitMode::Independent,
            },
        )
        .unwrap();
    let copy = workspace.layout.focused().unwrap();
    let TileKind::Waveform(tile) = &workspace.tiles[&copy].kind else {
        unreachable!()
    };
    assert!(!tile.link_vertical_scroll);
    assert_ne!(
        tile.items,
        workspace.tiles[&first].kind.waveform_list().unwrap()
    );
    workspace
        .apply_tile_message(
            first,
            TileMessage::Waveform(WaveformMessage::ScrollTo(50.0)),
            None,
        )
        .unwrap();
    assert_eq!(offset(&workspace, copy), 200.0);
}

#[test]
fn tile_navigation_rejects_invalid_values_without_mutating_any_peer() {
    let (mut workspace, _, first, _) = two_linked_tiles();
    let before = ron::to_string(&workspace.to_file().unwrap()).unwrap();
    for command in [
        WaveformMessage::ScrollTo(f32::NAN),
        WaveformMessage::ColumnWidths {
            names: 200.0,
            values: f32::INFINITY,
        },
    ] {
        assert!(
            workspace
                .apply_tile_message(first, TileMessage::Waveform(command), None)
                .is_err()
        );
        assert_eq!(
            ron::to_string(&workspace.to_file().unwrap()).unwrap(),
            before
        );
    }
    assert!(
        !workspace
            .apply_tile_message(
                crate::tiles::TileId(999),
                TileMessage::Waveform(WaveformMessage::ScrollTo(10.0)),
                None
            )
            .unwrap()
    );
    assert_eq!(
        ron::to_string(&workspace.to_file().unwrap()).unwrap(),
        before
    );
}

fn waveform_file() -> WaveformTileFile {
    WaveformTileFile {
        items: ItemListId(7),
        view: WaveformViewFile::from(&WaveformView::from(Viewport::new())),
        link_vertical_scroll: false,
        show_name_column: true,
        show_value_column: false,
        name_column_width: 220.0,
        value_column_width: 100.0,
    }
}

fn envelope(payload: &WaveformTileFile) -> TileFile {
    TileFile::encode(
        Some("My wave".into()),
        WAVEFORM.name,
        WAVEFORM.payload_version,
        payload,
    )
    .unwrap()
}

#[test]
fn waveform_registry_round_trip_and_linked_copy_reset_runtime() {
    let mut entry = TileEntry::from_file(envelope(&waveform_file())).unwrap();
    let TileKind::Waveform(tile) = &mut entry.kind else {
        panic!("wrong kind");
    };
    tile.view.scroll_offset = 70.0;
    tile.view.draw_cache.borrow_mut().builds = 4;
    tile.view.interaction.measure_start_location = Some(egui::Pos2::ZERO);
    let copy = entry.kind.split_clone().unwrap();
    assert_eq!(copy.waveform_list(), Some(ItemListId(7)));
    let TileKind::Waveform(copy) = copy else {
        panic!("wrong kind");
    };
    assert_eq!(copy.view.scroll_offset, 70.0);
    assert_eq!(copy.view.draw_cache.borrow().builds, 0);
    assert!(copy.view.interaction.measure_start_location.is_none());
    let encoded = ron::to_string(&entry.to_file().unwrap()).unwrap();
    let restored = TileEntry::from_file(crate::tiles::serde::decode(&encoded).unwrap()).unwrap();
    assert_eq!(restored.display_title(), "My wave");
    assert_eq!(restored.kind.kind_name(), WAVEFORM.name);
    let TileKind::Waveform(restored) = restored.kind else {
        panic!("wrong kind");
    };
    assert_eq!(restored.view.scroll_offset, 70.0);
    assert!(!restored.show_value_column);
    assert_eq!(restored.view.draw_cache.borrow().builds, 0);
}

#[test]
fn unavailable_kind_or_version_preserves_raw_payload_when_renamed() {
    for (name, version) in [("future.pipeline", 17), (WAVEFORM.name, 2)] {
        let mut file: TileFile =
            crate::tiles::serde::decode(include_str!("../fixtures/future-tile.ron")).unwrap();
        file.kind = name.into();
        file.kind_version = version;
        let raw = file.payload.get_ron().to_owned();
        let mut entry = TileEntry::from_file(file).unwrap();
        assert!(entry.kind.dependencies().is_opaque());
        assert!(entry.kind.split_clone().is_none());
        entry.title = Some("Renamed".into());
        let saved = entry.to_file().unwrap();
        assert_eq!(saved.payload.get_ron(), raw);
        assert_eq!(saved.kind, name);
        assert_eq!(saved.kind_version, version);
        assert_eq!(saved.title.as_deref(), Some("Renamed"));
    }
}

#[test]
fn linked_rows_do_not_link_time_navigation_or_focus() {
    use crate::tile_kinds::waveform::WaveformNavigation;
    let (mut workspace, _, first, second) = two_linked_tiles();
    let TileKind::Waveform(peer) = &workspace.tiles[&second].kind else {
        unreachable!()
    };
    let original = peer.view.viewport;
    workspace
        .apply_tile_message(
            first,
            TileMessage::Waveform(WaveformMessage::Navigate(WaveformNavigation::Pan(50.0))),
            None,
        )
        .unwrap();
    let TileKind::Waveform(tile) = &workspace.tiles[&first].kind else {
        unreachable!()
    };
    assert_ne!(tile.view.viewport, original);
    let TileKind::Waveform(peer) = &workspace.tiles[&second].kind else {
        unreachable!()
    };
    assert_eq!(peer.view.viewport, original);
    assert!(
        workspace
            .apply_tile_message(
                first,
                TileMessage::Waveform(WaveformMessage::FocusItem(Some(
                    crate::displayed_item::DisplayedItemRef(999)
                ))),
                None
            )
            .is_err()
    );
    assert_eq!(workspace.layout.focused(), Some(second));
}

#[test]
fn transaction_navigation_is_local_and_deduplicates_displayed_generators() {
    use crate::{
        SystemState,
        displayed_item::{DisplayedItem, DisplayedStream},
        transaction_container::{TransactionContainer, TransactionRef, TransactionStreamRef},
        wave_source::{LoadOptions, WaveFormat, WaveSource},
    };
    let mut inner = ftr_parser::parse::parse_ftr(
        project_root::get_project_root()
            .unwrap()
            .join("examples/my_db.ftr"),
    )
    .unwrap();
    let streams = inner.tx_streams.keys().copied().collect::<Vec<_>>();
    for stream in streams {
        inner.load_stream_into_memory(stream).unwrap();
    }
    let mut state = SystemState::new_default_config().unwrap();
    state.on_transaction_streams_loaded(
        WaveSource::Data,
        WaveFormat::Ftr,
        TransactionContainer {
            locations: Default::default(),
            indexes: Default::default(),
            #[cfg(not(target_arch = "wasm32"))]
            native: None,
            inner,
            vtr_details: None,
        },
        LoadOptions::Clear,
    );
    let document = state.user.waves.as_ref().unwrap();
    let container = document.inner.as_transactions().unwrap();
    let generator = container
        .get_generators()
        .into_iter()
        .find(|generator| generator.transactions.len() >= 2)
        .unwrap();
    let mut ids = container.get_transactions_from_generator(generator.id);
    ids.sort_unstable_by_key(|id| id.0);
    let (mut workspace, _, first, second) = two_linked_tiles();
    let list_id = workspace.tiles[&first].kind.waveform_list().unwrap();
    let list = workspace.item_lists.get_mut(&list_id).unwrap();
    for _ in 0..2 {
        list.insert_item(
            DisplayedItem::Stream(DisplayedStream {
                transaction_stream_ref: TransactionStreamRef::new_gen(
                    generator.stream_id,
                    generator.id,
                    generator.name.clone(),
                ),
                color: None,
                background_color: None,
                display_name: generator.name.clone(),
                manual_name: None,
                rows: 1,
            }),
            list.end_insert_position(),
        )
        .unwrap();
    }
    let focus = |workspace: &crate::tiles::workspace::Workspace, id| {
        let TileKind::Waveform(tile) = &workspace.tiles[&id].kind else {
            panic!()
        };
        tile.view.focused_transaction.clone()
    };
    for expected in &ids[..2] {
        assert!(
            workspace
                .apply_tile_message(
                    first,
                    TileMessage::Waveform(WaveformMessage::MoveTransaction { next: true }),
                    Some(document)
                )
                .unwrap()
        );
        assert_eq!(
            focus(&workspace, first),
            Some(TransactionRef { id: *expected })
        );
        assert_eq!(focus(&workspace, second), None);
    }
    assert!(
        !workspace
            .apply_tile_message(
                first,
                TileMessage::Waveform(WaveformMessage::MoveTransaction { next: true }),
                None
            )
            .unwrap()
    );
    assert_eq!(
        focus(&workspace, first),
        Some(TransactionRef { id: ids[1] })
    );
    workspace
        .apply_tile_message(
            first,
            TileMessage::Waveform(WaveformMessage::MoveTransaction { next: false }),
            Some(document),
        )
        .unwrap();
    assert_eq!(
        focus(&workspace, first),
        Some(TransactionRef { id: ids[0] })
    );
    assert!(
        container
            .get_transactions_from_generator(ftr_parser::types::GeneratorId(999999))
            .is_empty()
    );
    assert!(
        container
            .get_transactions_from_stream(ftr_parser::types::StreamId(999999))
            .is_empty()
    );
    workspace
        .apply_tile_message(
            first,
            TileMessage::Waveform(WaveformMessage::FocusTransaction(None)),
            None,
        )
        .unwrap();
    assert_eq!(focus(&workspace, first), None);
    assert_eq!(workspace.layout.focused(), Some(second));
}

#[test]
fn selection_is_shared_only_by_linked_lists_and_preserves_view_focus() {
    use crate::tiles::{commands::WorkspaceCommand, layout::Direction};
    use crate::{
        displayed_item::{DisplayedDivider, DisplayedItem},
        item_list::ItemSelection,
    };
    let (mut workspace, mut runtime, first, second) = two_linked_tiles();
    let list_id = workspace.tiles[&first].kind.waveform_list().unwrap();
    let list = workspace.item_lists.get_mut(&list_id).unwrap();
    let item = list
        .insert_item(
            DisplayedItem::Divider(DisplayedDivider {
                name: None,
                color: None,
                background_color: None,
            }),
            list.end_insert_position(),
        )
        .unwrap();
    workspace
        .apply_tile_message(
            first,
            TileMessage::Waveform(WaveformMessage::FocusItem(Some(item))),
            None,
        )
        .unwrap();
    workspace
        .apply_command(
            &mut runtime,
            WorkspaceCommand::SplitTile {
                tile: first,
                dir: Direction::Down,
                mode: SplitMode::Independent,
            },
        )
        .unwrap();
    let copy = workspace.layout.focused().unwrap();
    let copy_list = workspace.tiles[&copy].kind.waveform_list().unwrap();
    assert!(
        workspace
            .apply_tile_message(
                second,
                TileMessage::Waveform(WaveformMessage::Selection(ItemSelection::Toggle(item))),
                None
            )
            .unwrap()
    );
    assert_eq!(workspace.tiles[&second].kind.waveform_list(), Some(list_id));
    assert_eq!(
        workspace.item_lists[&list_id]
            .items_tree
            .iter_visible_selected()
            .count(),
        1
    );
    assert_eq!(
        workspace.item_lists[&copy_list]
            .items_tree
            .iter_visible_selected()
            .count(),
        0
    );
    let TileKind::Waveform(first_tile) = &workspace.tiles[&first].kind else {
        panic!()
    };
    let TileKind::Waveform(second_tile) = &workspace.tiles[&second].kind else {
        panic!()
    };
    assert_eq!(first_tile.view.focused_item, Some(item));
    assert_eq!(second_tile.view.focused_item, None);
    assert_eq!(workspace.layout.focused(), Some(copy));
}

#[test]
fn list_deletion_reconciles_linked_views_without_touching_independent_copies() {
    use crate::displayed_item::{DisplayedDivider, DisplayedItem};
    use crate::tiles::{commands::WorkspaceCommand, layout::Direction};
    let (mut workspace, mut runtime, first, second) = two_linked_tiles();
    let list_id = workspace.tiles[&first].kind.waveform_list().unwrap();
    let list = workspace.item_lists.get_mut(&list_id).unwrap();
    let item = list.next_displayed_item_ref();
    list.items_tree
        .insert_item(item, list.end_insert_position())
        .unwrap();
    list.displayed_items.insert(
        item,
        DisplayedItem::Divider(DisplayedDivider {
            name: Some("row".into()),
            color: None,
            background_color: None,
        }),
    );
    for id in [first, second] {
        workspace
            .apply_tile_message(
                id,
                TileMessage::Waveform(WaveformMessage::FocusItem(Some(item))),
                None,
            )
            .unwrap();
    }
    workspace
        .apply_command(
            &mut runtime,
            WorkspaceCommand::SplitTile {
                tile: first,
                dir: Direction::Down,
                mode: SplitMode::Independent,
            },
        )
        .unwrap();
    let copy = workspace.layout.focused().unwrap();
    let copy_list = workspace.tiles[&copy].kind.waveform_list().unwrap();
    workspace
        .apply_tile_message(
            first,
            TileMessage::Waveform(WaveformMessage::RemoveItems(vec![item, item])),
            None,
        )
        .unwrap();
    assert!(workspace.item_lists[&list_id].displayed_items.is_empty());
    assert!(
        workspace.item_lists[&list_id]
            .layout_cache
            .borrow()
            .signature
            .is_none()
    );
    assert!(
        workspace.item_lists[&copy_list]
            .displayed_items
            .contains_key(&item)
    );
    for (id, expected) in [(first, None), (second, None), (copy, Some(item))] {
        let TileKind::Waveform(tile) = &workspace.tiles[&id].kind else {
            unreachable!()
        };
        assert_eq!(tile.view.focused_item, expected);
    }
    assert!(
        !workspace
            .apply_tile_message(
                first,
                TileMessage::Waveform(WaveformMessage::RemoveItems(vec![item])),
                None
            )
            .unwrap()
    );
}

#[test]
fn invalid_known_payloads_fail_instead_of_becoming_unknown() {
    let malformed = TileFile::encode(None, WAVEFORM.name, 1, &vec![1, 2, 3]).unwrap();
    assert!(TileEntry::from_file(malformed).is_err());
    for mutate in [
        (|file: &mut WaveformTileFile| file.items = ItemListId(0)) as fn(&mut WaveformTileFile),
        |file| file.view.scroll_offset = f32::NAN,
        |file| file.view.scroll_offset = -1.0,
        |file| file.view.viewport.curr_right = file.view.viewport.curr_left,
        |file| file.view.viewport.curr_left.0 = f64::NEG_INFINITY,
        |file| {
            file.view.viewport.move_strategy =
                crate::viewport::ViewportStrategy::EaseInOut { duration: f32::NAN }
        },
        |file| file.name_column_width = f32::INFINITY,
        |file| file.value_column_width = 0.0,
    ] {
        let mut file = waveform_file();
        mutate(&mut file);
        assert!(TileEntry::from_file(envelope(&file)).is_err());
    }
}
