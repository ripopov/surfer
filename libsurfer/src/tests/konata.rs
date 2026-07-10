use std::{sync::Arc, time::Instant};

use ftr_parser::types::{GeneratorId, StreamId};
use project_root::get_project_root;
use surfer_wcp::{WcpCSMessage, WcpCommand, WcpResponse, WcpSCMessage};

use crate::{
    Message, StartupParams, SystemState, WaveSource,
    fzcmd::parse_command,
    konata::{KonataAlignmentMode, KonataBuildInput, KonataModel, KonataTileId},
    source::SourceId,
    tiles::SurferPane,
    transaction_container::TransactionStreamRef,
};

fn loaded_sample() -> (tokio::runtime::Runtime, SystemState) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let state = {
        let _guard = runtime.enter();
        let mut state = SystemState::new_default_config()
            .unwrap()
            .with_params(StartupParams {
                waves: Some(WaveSource::File(
                    get_project_root()
                        .unwrap()
                        .join("examples/kanata-sample-2.ftr")
                        .try_into()
                        .unwrap(),
                )),
                ..Default::default()
            });
        crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);
        state
    };
    (runtime, state)
}

fn instruction_generator() -> TransactionStreamRef {
    TransactionStreamRef::new_gen(StreamId(1), GeneratorId(10), "instruction".to_string())
}

fn open_view(state: &mut SystemState) -> KonataTileId {
    state.update(Message::OpenKonataView {
        source: SourceId::default(),
        generator: instruction_generator(),
    });
    *state
        .user
        .konata_tiles
        .keys()
        .next()
        .expect("Konata tile created")
}

fn wait_for_model(state: &mut SystemState) {
    let start = Instant::now();
    while !state.konata_caches_ready() {
        state.handle_async_messages();
        if start.elapsed().as_secs() > 10 {
            panic!("timed out waiting for Konata model");
        }
        std::thread::yield_now();
    }
    state.handle_async_messages();
}

fn wait_for_find(state: &mut SystemState, tile_id: KonataTileId) {
    let start = Instant::now();
    while state.konata_runtime[&tile_id].find_searching {
        state.handle_async_messages();
        if start.elapsed().as_secs() > 10 {
            panic!("timed out waiting for Konata search");
        }
        std::thread::yield_now();
    }
    state.handle_async_messages();
}

fn wait_for_tables(state: &mut SystemState) {
    let start = Instant::now();
    while !state.table_caches_ready() {
        state.handle_async_messages();
        if start.elapsed().as_secs() > 10 {
            panic!("timed out waiting for table cache");
        }
        std::thread::yield_now();
    }
    state.handle_async_messages();
}

#[test]
fn open_build_share_persist_and_close_konata_tiles() {
    let (runtime, mut state) = loaded_sample();
    let _guard = runtime.enter();
    let first = open_view(&mut state);
    assert!(state.user.tile_tree.tree.tiles.iter().any(|(_, tile)| {
        matches!(tile, egui_tiles::Tile::Pane(SurferPane::Konata(id)) if *id == first)
    }));

    state.update(Message::BuildKonataModel { tile_id: first });
    wait_for_model(&mut state);
    let first_entry = state
        .konata_runtime
        .get(&first)
        .and_then(|runtime| runtime.entry.clone())
        .expect("first model entry");
    let model = first_entry.model().expect("built model");
    assert_eq!(model.row_count(), 4_041);
    assert_eq!(model.stage_count(), 51_961);
    if let Some(dependency) = model.dependencies.first() {
        let root = dependency.consumer_row as usize;
        state.update(Message::ToggleKonataProducerChain {
            tile_id: first,
            row: root,
        });
        wait_for_model(&mut state);
        assert!(
            state.konata_runtime[&first]
                .producer_chain
                .as_ref()
                .is_some_and(|chain| chain.contains(root))
        );
        state.update(Message::ToggleKonataProducerChain {
            tile_id: first,
            row: root,
        });
        assert!(state.konata_runtime[&first].producer_chain.is_none());
    }

    state.update(Message::KonataGotoRow(100));
    assert_eq!(
        state.user.konata_tiles[&first].viewport.top_visible_row,
        100.0
    );
    state.update(Message::KonataBookmarkSet(3));
    state.update(Message::KonataGotoSid(8));
    assert_eq!(
        state.user.konata_tiles[&first].viewport.top_visible_row,
        0.0
    );
    state.update(Message::KonataBookmarkGoto(3));
    assert_eq!(
        state.user.konata_tiles[&first].viewport.top_visible_row,
        100.0
    );
    {
        let tile = state.user.konata_tiles.get_mut(&first).unwrap();
        tile.config.clock_period_ticks = Some(2);
        tile.config.clock_origin_tick = 5;
    }
    state.update(Message::KonataGotoCycle(10));
    assert_eq!(state.user.konata_tiles[&first].viewport.left_tick, 25);

    let second = {
        state.update(Message::OpenKonataView {
            source: SourceId::default(),
            generator: instruction_generator(),
        });
        state
            .user
            .konata_tiles
            .keys()
            .copied()
            .find(|tile| *tile != first)
            .expect("second Konata tile")
    };
    state.update(Message::BuildKonataModel { tile_id: second });
    let second_entry = state
        .konata_runtime
        .get(&second)
        .and_then(|runtime| runtime.entry.clone())
        .expect("second model entry");
    assert!(Arc::ptr_eq(&first_entry, &second_entry));
    assert_eq!(state.konata_models.len(), 1);

    let second_spec = state.user.konata_tiles[&second].spec.clone();
    {
        let first_tile = state.user.konata_tiles.get_mut(&first).unwrap();
        first_tile.config.synchronize_scroll = true;
        first_tile.config.overlay = Some(second_spec.clone());
        first_tile.config.overlay_tile = Some(second);
        first_tile.config.splitter_px = 360.0;
        first_tile.viewport.top_visible_row = 100.0;
        first_tile.viewport.set_left_tick(700);
        first_tile.viewport.px_per_tick = 9.0;
        let second_tile = state.user.konata_tiles.get_mut(&second).unwrap();
        second_tile.config.synchronize_scroll = true;
    }
    let mut konata_tiles = std::mem::take(&mut state.user.konata_tiles);
    crate::konata::synchronize_tiles(&state, &mut konata_tiles, first);
    state.user.konata_tiles = konata_tiles;
    assert_eq!(
        state.user.konata_tiles[&second].viewport.top_visible_row,
        100.0
    );
    assert_eq!(state.user.konata_tiles[&second].viewport.left_tick, 700);
    assert_eq!(state.user.konata_tiles[&second].viewport.px_per_tick, 9.0);
    assert_eq!(state.user.konata_tiles[&second].config.splitter_px, 360.0);
    assert_eq!(
        state.user.konata_tiles[&second].config.alignment_mode,
        KonataAlignmentMode::ThreadRid
    );

    {
        let first_tile = state.user.konata_tiles.get_mut(&first).unwrap();
        first_tile.config.alignment_mode = KonataAlignmentMode::FetchId;
        first_tile.viewport.top_visible_row = 321.0;
    }
    let mut konata_tiles = std::mem::take(&mut state.user.konata_tiles);
    crate::konata::synchronize_tiles(&state, &mut konata_tiles, first);
    state.user.konata_tiles = konata_tiles;
    assert_eq!(
        state.user.konata_tiles[&second].viewport.top_visible_row,
        321.0
    );
    assert_eq!(
        state.user.konata_tiles[&second].config.alignment_mode,
        KonataAlignmentMode::FetchId
    );

    {
        let tile = state.user.konata_tiles.get_mut(&first).unwrap();
        tile.viewport.top_visible_row = 240.25;
        tile.viewport.set_left_tick(1_234);
        tile.viewport.px_per_tick = 7.5;
        tile.viewport.row_height_px = 13.0;
    }
    state.capture_konata_reload_anchor(first).unwrap();
    state.user.konata_tiles.get_mut(&first).unwrap().viewport = Default::default();
    state.restore_konata_reload_anchor(first, &model).unwrap();
    assert_eq!(
        state.user.konata_tiles[&first].viewport.top_visible_row,
        240.25
    );
    assert_eq!(state.user.konata_tiles[&first].viewport.left_tick, 1_234);
    assert_eq!(state.user.konata_tiles[&first].viewport.px_per_tick, 7.5);
    assert_eq!(state.user.konata_tiles[&first].viewport.row_height_px, 13.0);

    let removed_tx = model.rows.tx_id[240];
    let removed_tick = model.rows.begin[240];
    let (parents, events, relations) = {
        let mut transactions = ftr_parser::parse::parse_ftr(
            get_project_root()
                .unwrap()
                .join("examples/kanata-sample-2.ftr"),
        )
        .unwrap();
        transactions.load_stream_into_memory(StreamId(1)).unwrap();
        (
            transactions
                .get_generator(GeneratorId(10))
                .unwrap()
                .transactions
                .iter()
                .filter(|transaction| transaction.get_tx_id().0 != removed_tx)
                .cloned()
                .collect::<Vec<_>>(),
            transactions
                .get_generator(GeneratorId(11))
                .unwrap()
                .transactions
                .clone(),
            transactions.tx_relations.clone(),
        )
    };
    let replacement = KonataModel::build(KonataBuildInput {
        parent_generator: GeneratorId(10),
        event_generator: GeneratorId(11),
        stream: StreamId(1),
        parents: Arc::new(parents),
        events,
        relations,
    });
    state.capture_konata_reload_anchor(first).unwrap();
    state.user.konata_tiles.get_mut(&first).unwrap().viewport = Default::default();
    state
        .restore_konata_reload_anchor(first, &replacement)
        .unwrap();
    let fallback_row = replacement.nearest_row_for_tick(removed_tick).unwrap();
    let restored_viewport = state.user.konata_tiles[&first].viewport;
    assert_eq!(
        restored_viewport.top_visible_row,
        fallback_row as f64 + 0.25
    );
    let restored_left = restored_viewport.left_tick as f64 + restored_viewport.left_frac;
    let expected_left =
        replacement.rows.begin[fallback_row] as f64 + (1_234.0 - model.rows.begin[240] as f64);
    assert!((restored_left - expected_left).abs() < f64::EPSILON);
    assert_eq!(restored_viewport.px_per_tick, 7.5);
    assert_eq!(restored_viewport.row_height_px, 13.0);

    state
        .user
        .konata_tiles
        .get_mut(&first)
        .expect("first tile state")
        .viewport
        .top_visible_row = 123.5;
    let serialized = ron::to_string(&state.user).expect("serialize state");
    let restored: crate::state::UserState = ron::from_str(&serialized).expect("restore state");
    assert_eq!(
        restored.konata_tiles[&first].viewport.top_visible_row,
        123.5
    );
    assert_eq!(
        restored.konata_tiles[&first].config.overlay,
        Some(second_spec)
    );
    assert_eq!(
        restored.konata_tiles[&first].config.overlay_tile,
        Some(second)
    );
    assert_eq!(
        restored.konata_tiles[&first].config.alignment_mode,
        KonataAlignmentMode::FetchId
    );
    assert!(restored.konata_bookmarks[&state.user.konata_tiles[&first].spec][3].is_some());

    state.update(Message::RemoveKonataTile { tile_id: first });
    assert!(!state.user.konata_tiles.contains_key(&first));
    assert_eq!(state.konata_models.len(), 1);
    state.update(Message::RemoveKonataTile { tile_id: second });
    assert!(state.konata_models.is_empty());
}

#[test]
fn file_backed_konata_build_streams_without_repopulating_the_generic_graph() {
    let (runtime, mut state) = loaded_sample();
    let _guard = runtime.enter();
    let stream_id = StreamId(1);
    assert!(
        state
            .user
            .waves
            .as_ref()
            .unwrap()
            .transactions_for_source(SourceId::default())
            .unwrap()
            .inner
            .tx_relations
            .is_empty()
    );
    state
        .user
        .waves
        .as_mut()
        .unwrap()
        .transactions_for_source_mut(SourceId::default())
        .unwrap()
        .inner
        .drop_stream_from_memory(stream_id);

    let tile_id = open_view(&mut state);
    let entry = state.konata_runtime[&tile_id]
        .entry
        .clone()
        .expect("file-backed model entry");
    let progressive_started = Instant::now();
    let partial = loop {
        if let Some(model) = entry.model() {
            break model;
        }
        assert!(
            progressive_started.elapsed().as_secs() < 10,
            "timed out waiting for progressive Konata snapshot"
        );
        std::thread::yield_now();
    };
    assert!(!entry.is_complete());
    let partial_revision = entry.revision();
    assert!(partial_revision > 0);
    assert_eq!(partial.stage_count(), 0);
    let table_spec = crate::table::TableModelSpec::KonataInstructions {
        spec: state.user.konata_tiles[&tile_id].spec.clone(),
    };
    let partial_table_generation = table_spec.cache_generation(&state.table_model_context());
    assert!((512..=4_041).contains(&partial.row_count()));
    state.update(Message::StartKonataFind {
        tile_id,
        pattern: "^ID 4000 ".to_string(),
    });
    wait_for_model(&mut state);
    wait_for_find(&mut state, tile_id);
    assert!(entry.revision() > partial_revision);
    assert_ne!(
        table_spec.cache_generation(&state.table_model_context()),
        partial_table_generation
    );
    assert_eq!(
        state.konata_runtime[&tile_id]
            .find_hits
            .as_ref()
            .unwrap()
            .count(),
        1
    );
    assert_eq!(state.konata_runtime[&tile_id].find_active_row, Some(4_000));
    let model = state.active_konata_model(tile_id).unwrap();
    assert_eq!(model.row_count(), 4_041);
    assert_eq!(model.stage_count(), 51_961);

    let transactions = state
        .user
        .waves
        .as_ref()
        .unwrap()
        .transactions_for_source(SourceId::default())
        .unwrap();
    let stream = transactions.get_stream(stream_id).unwrap();
    assert!(!stream.transactions_loaded);
    assert!(transactions.inner.tx_relations.is_empty());
    assert!(stream.generators.iter().all(|generator| {
        transactions
            .get_generator(*generator)
            .unwrap()
            .transactions
            .is_empty()
    }));
}

#[test]
fn command_opens_named_pipeline_generator() {
    let (runtime, mut state) = loaded_sample();
    let _guard = runtime.enter();
    let message = parse_command(
        "konata_view_new kanata-sample-2.ftr:instruction",
        crate::command_parser::get_parser(&state),
    )
    .expect("parse Konata command");
    let Message::OpenKonataView { source, generator } = message else {
        panic!("expected OpenKonataView");
    };
    assert_eq!(source, SourceId::default());
    assert_eq!(generator, instruction_generator());

    state.update(Message::OpenKonataView { source, generator });
    assert_eq!(state.user.konata_tiles.len(), 1);

    assert!(matches!(
        parse_command("j 42", crate::command_parser::get_parser(&state)),
        Ok(Message::KonataGotoRow(42))
    ));
    assert!(matches!(
        parse_command(
            "konata_bookmark_set 7",
            crate::command_parser::get_parser(&state)
        ),
        Ok(Message::KonataBookmarkSet(7))
    ));
    assert!(matches!(
        parse_command("jr 0:7", crate::command_parser::get_parser(&state)),
        Ok(Message::KonataGotoThreadRid { thread, rid }) if thread == "0" && rid == 7
    ));

    assert!(matches!(
        parse_command("f ^ID 40 ", crate::command_parser::get_parser(&state)),
        Ok(Message::StartKonataFind { pattern, .. }) if pattern == "^ID 40 "
    ));
    assert!(matches!(
        parse_command("konata_stats", crate::command_parser::get_parser(&state)),
        Ok(Message::OpenKonataStatistics { .. })
    ));
}

#[test]
fn startup_commands_open_minimap_in_a_konata_only_layout() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let _guard = runtime.enter();
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples/kanata-sample-2.ftr")
                    .try_into()
                    .unwrap(),
            )),
            startup_commands: vec!["konata_view_first;konata_minimap_show;konata_only".to_string()],
            ..Default::default()
        });

    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

    let (&tile_id, tile) = state
        .user
        .konata_tiles
        .iter()
        .next()
        .expect("startup Konata tile");
    assert_eq!(state.user.konata_tiles.len(), 1);
    assert!(tile.config.show_minimap);
    let panes = state
        .user
        .tile_tree
        .tree
        .tiles
        .iter()
        .filter_map(|(_, tile)| match tile {
            egui_tiles::Tile::Pane(pane) => Some(pane),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(matches!(panes.as_slice(), [SurferPane::Konata(id)] if *id == tile_id));
}

#[test]
fn asynchronous_find_wraps_and_keeps_last_valid_results_on_error() {
    let (runtime, mut state) = loaded_sample();
    let _guard = runtime.enter();
    let tile_id = open_view(&mut state);
    wait_for_model(&mut state);

    state.update(Message::StartKonataFind {
        tile_id,
        pattern: "^ID 40 ".to_string(),
    });
    wait_for_find(&mut state, tile_id);
    let find = &state.konata_runtime[&tile_id];
    assert_eq!(find.find_hits.as_ref().unwrap().count(), 1);
    assert_eq!(find.find_active_row, Some(40));
    assert_eq!(find.find_valid_pattern.as_deref(), Some("^ID 40 "));

    state.update(Message::StartKonataFind {
        tile_id,
        pattern: ".*".to_string(),
    });
    state.update(Message::StartKonataFind {
        tile_id,
        pattern: "^ID 41 ".to_string(),
    });
    wait_for_find(&mut state, tile_id);
    let find = &state.konata_runtime[&tile_id];
    assert_eq!(find.find_hits.as_ref().unwrap().count(), 1);
    assert_eq!(find.find_active_row, Some(41));
    assert_eq!(find.find_valid_pattern.as_deref(), Some("^ID 41 "));

    state.update(Message::OpenKonataFindTable {
        tile_id,
        pattern: "^ID 41 ".to_string(),
    });
    wait_for_tables(&mut state);
    let table_id = *state.user.table_tiles.keys().next().unwrap();
    assert!(matches!(
        state.user.table_tiles[&table_id].spec,
        crate::table::TableModelSpec::KonataInstructions { .. }
    ));
    assert_eq!(
        state.table_runtime[&table_id]
            .cache
            .as_ref()
            .unwrap()
            .get()
            .unwrap()
            .row_ids
            .len(),
        1
    );
    let table_model = state.table_runtime[&table_id].model.as_ref().unwrap();
    let konata_model = state.konata_runtime[&tile_id]
        .entry
        .as_ref()
        .unwrap()
        .model()
        .unwrap();
    assert_eq!(
        table_model.search_text(crate::table::TableRowId(konata_model.rows.tx_id[41])),
        konata_model.search_text(41)
    );

    let parent_tx = konata_model.rows.tx_id[41];
    let parent_stages = konata_model.stages_for_row_blocking(41);
    let expected_stage_ids = parent_stages
        .iter()
        .map(|stage| stage.event_tx)
        .collect::<Vec<_>>();
    assert!(
        !state
            .user
            .waves
            .as_ref()
            .unwrap()
            .transactions_for_source(SourceId::default())
            .unwrap()
            .get_stream(StreamId(1))
            .unwrap()
            .transactions_loaded
    );
    state.update(Message::OpenKonataEventTable {
        tile_id,
        parent_tx: Some(parent_tx),
    });
    wait_for_tables(&mut state);
    let event_table_id = state
        .user
        .table_tiles
        .iter()
        .find_map(|(id, tile)| {
            matches!(
                tile.spec,
                crate::table::TableModelSpec::KonataEvents {
                    parent_tx: Some(candidate),
                    ..
                } if candidate == parent_tx
            )
            .then_some(*id)
        })
        .unwrap();
    let event_cache = state.table_runtime[&event_table_id]
        .cache
        .as_ref()
        .unwrap()
        .get()
        .unwrap();
    assert_eq!(
        event_cache
            .row_ids
            .iter()
            .map(|id| id.0)
            .collect::<Vec<_>>(),
        expected_stage_ids
    );
    let first_event = event_cache.row_ids[0];
    assert!(matches!(
        state.table_runtime[&event_table_id]
            .model
            .as_ref()
            .unwrap()
            .on_activate(first_event),
        crate::table::TableAction::FocusTransaction(source, transaction)
            if source == SourceId::default() && transaction.id.0 == first_event.0
    ));
    assert!(
        !state
            .user
            .waves
            .as_ref()
            .unwrap()
            .transactions_for_source(SourceId::default())
            .unwrap()
            .get_stream(StreamId(1))
            .unwrap()
            .transactions_loaded
    );

    state
        .user
        .konata_tiles
        .get_mut(&tile_id)
        .unwrap()
        .config
        .clock_period_ticks = Some(1);
    state.update(Message::OpenKonataStatistics { tile_id });
    wait_for_tables(&mut state);
    let statistics_id = state
        .user
        .table_tiles
        .iter()
        .find_map(|(id, tile)| {
            matches!(
                tile.spec,
                crate::table::TableModelSpec::KonataStatistics { .. }
            )
            .then_some(*id)
        })
        .unwrap();
    let statistics = state.table_runtime[&statistics_id].model.as_ref().unwrap();
    assert!(statistics.row_count() > konata_model.stage_names.len());
    assert!(matches!(
        statistics.cell(crate::table::TableRowId(0), 3),
        crate::table::TableCell::Text(value) if value == "4041.0000"
    ));
    state.update(Message::OpenKonataRangeStatistics {
        tile_id,
        range: (100, 200),
    });
    wait_for_tables(&mut state);
    assert!(state.user.table_tiles.values().any(|tile| {
        matches!(
            tile.spec,
            crate::table::TableModelSpec::KonataStatistics {
                range: Some((100, 200)),
                ..
            }
        ) && tile.config.title.contains("[100, 200)")
    }));

    state.update(Message::KonataFindNext {
        tile_id,
        reverse: false,
    });
    assert_eq!(state.konata_runtime[&tile_id].find_active_row, Some(41));

    let previous_hits = state.konata_runtime[&tile_id].find_hits.clone().unwrap();
    state.update(Message::StartKonataFind {
        tile_id,
        pattern: "[".to_string(),
    });
    wait_for_find(&mut state, tile_id);
    let find = &state.konata_runtime[&tile_id];
    assert!(find.find_error.is_some());
    assert!(Arc::ptr_eq(
        find.find_hits.as_ref().unwrap(),
        &previous_hits
    ));
    assert_eq!(find.find_active_row, Some(41));
}

#[test]
fn wcp_konata_commands_validate_and_drive_the_active_tile() {
    let (runtime, mut state) = loaded_sample();
    let _guard = runtime.enter();
    let (server_tx, server_rx) = tokio::sync::mpsc::channel(16);
    let (client_tx, mut client_rx) = tokio::sync::mpsc::channel(16);
    state.channels.wcp_c2s_receiver = Some(server_rx);
    state.channels.wcp_s2c_sender = Some(client_tx);

    let mut round_trip = |state: &mut SystemState, message| {
        server_tx.try_send(message).unwrap();
        state.handle_wcp_commands();
        client_rx.try_recv().expect("WCP response")
    };

    assert!(matches!(
        round_trip(&mut state, WcpCSMessage::create_greeting(0, Vec::new())),
        WcpSCMessage::greeting { .. }
    ));
    assert_eq!(
        round_trip(
            &mut state,
            WcpCSMessage::command(WcpCommand::konata_open {
                generator: "instruction".to_string(),
                source: None,
            })
        ),
        WcpSCMessage::response(WcpResponse::ack)
    );
    let tile_id = *state.user.konata_tiles.keys().next().unwrap();
    wait_for_model(&mut state);

    assert_eq!(
        round_trip(
            &mut state,
            WcpCSMessage::command(WcpCommand::konata_goto_row { row: 42 })
        ),
        WcpSCMessage::response(WcpResponse::ack)
    );
    assert_eq!(
        state.user.konata_tiles[&tile_id].viewport.top_visible_row,
        42.0
    );
    assert!(matches!(
        round_trip(
            &mut state,
            WcpCSMessage::command(WcpCommand::konata_goto_row { row: u64::MAX })
        ),
        WcpSCMessage::error { error, .. } if error == "konata_goto_row"
    ));
    assert!(matches!(
        round_trip(
            &mut state,
            WcpCSMessage::command(WcpCommand::konata_goto_cycle { cycle: 12 })
        ),
        WcpSCMessage::error { error, .. } if error == "konata_goto_cycle"
    ));

    state
        .user
        .konata_tiles
        .get_mut(&tile_id)
        .unwrap()
        .config
        .clock_period_ticks = Some(4);
    assert_eq!(
        round_trip(
            &mut state,
            WcpCSMessage::command(WcpCommand::konata_goto_cycle { cycle: 12 })
        ),
        WcpSCMessage::response(WcpResponse::ack)
    );
    assert_eq!(state.user.konata_tiles[&tile_id].viewport.left_tick, 48);

    let tx_id = state.active_konata_model(tile_id).unwrap().rows.tx_id[42];
    assert_eq!(
        round_trip(
            &mut state,
            WcpCSMessage::command(WcpCommand::konata_focus_instruction {
                transaction_id: tx_id,
            })
        ),
        WcpSCMessage::response(WcpResponse::ack)
    );
    assert_eq!(
        state
            .user
            .waves
            .as_ref()
            .unwrap()
            .focused_transaction
            .0
            .as_ref()
            .unwrap()
            .inner
            .id
            .0,
        tx_id
    );
}

#[test]
fn synchronize_scroll_couples_konata_and_waveform_time_axes() {
    use num::{BigInt, ToPrimitive as _};

    let (runtime, mut state) = loaded_sample();
    let _guard = runtime.enter();
    let tile = open_view(&mut state);
    wait_for_model(&mut state);

    let width = 800.0_f32;
    // Pretend the tile has been drawn so it reports a canvas width to the sync driver.
    state.konata_runtime.entry(tile).or_default().canvas_size = egui::Vec2::new(width, 400.0);

    let num_timestamps = state
        .user
        .waves
        .as_ref()
        .unwrap()
        .safe_canvas_num_timestamps()
        .to_f64()
        .unwrap();
    assert!(num_timestamps > 20.0, "sample must have a usable time span");

    // Focus the Konata tile on a sub-window well inside the trace and enable synchronization.
    let konata_left = num_timestamps * 0.2;
    let konata_right = num_timestamps * 0.4;
    {
        let tile_state = state.user.konata_tiles.get_mut(&tile).unwrap();
        tile_state.config.synchronize_scroll = true;
        tile_state.config.sync_group = Some(0);
        tile_state
            .viewport
            .set_visible_tick_range(konata_left, konata_right, width);
    }

    let tol = num_timestamps * 1e-3;

    // First pass seeds the shared window from the Konata tile and pulls the waveform onto it.
    assert!(state.synchronize_konata_wave_viewports());
    let (wave_left, wave_right) = state.user.waves.as_ref().unwrap().viewports[0].absolute_range(
        &state
            .user
            .waves
            .as_ref()
            .unwrap()
            .safe_canvas_num_timestamps(),
    );
    assert!(
        (wave_left.inner() - konata_left).abs() < tol
            && (wave_right.inner() - konata_right).abs() < tol,
        "waveform did not adopt Konata window: got ({}, {}), want ({konata_left}, {konata_right})",
        wave_left.inner(),
        wave_right.inner(),
    );

    // Once settled, another pass with no user input must not move anything.
    assert!(!state.synchronize_konata_wave_viewports());

    // Now zoom the waveform; the Konata tile must follow onto the new window.
    let new_left = num_timestamps * 0.5;
    let new_right = num_timestamps * 0.6;
    {
        let waves = state.user.waves.as_mut().unwrap();
        let n = waves.safe_canvas_num_timestamps();
        waves.viewports[0].zoom_to_range(
            &BigInt::from(new_left as i64),
            &BigInt::from(new_right as i64),
            &n,
        );
    }
    assert!(state.synchronize_konata_wave_viewports());
    let (konata_vis_left, konata_vis_right) = state.user.konata_tiles[&tile]
        .viewport
        .visible_tick_range(width);
    assert!(
        (konata_vis_left - new_left).abs() < tol && (konata_vis_right - new_right).abs() < tol,
        "Konata did not follow the waveform: got ({konata_vis_left}, {konata_vis_right}), \
         want ({new_left}, {new_right})",
    );

    // Disabling synchronization must release the coupling and reset the shared state.
    state
        .user
        .konata_tiles
        .get_mut(&tile)
        .unwrap()
        .config
        .synchronize_scroll = false;
    state
        .user
        .konata_tiles
        .get_mut(&tile)
        .unwrap()
        .config
        .sync_group = None;
    assert!(!state.synchronize_konata_wave_viewports());
}
