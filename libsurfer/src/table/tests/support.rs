use super::*;

/// Helper to build a row_index HashMap from a slice of row IDs.
pub(super) fn build_row_index(rows: &[TableRowId]) -> HashMap<TableRowId, usize> {
    rows.iter().enumerate().map(|(i, &id)| (id, i)).collect()
}

pub(super) fn load_counter_state() -> SystemState {
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

pub(super) fn test_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime")
}

pub(super) fn load_counter_state_with_variable(var_path: &str) -> SystemState {
    let mut state = load_counter_state();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string(var_path),
    ]));
    wait_for_waves_fully_loaded(&mut state, 10);
    state
}

pub(super) fn load_counter_state_with_variables(var_paths: &[&str]) -> SystemState {
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

pub(super) fn find_visible_index_for_variable(
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

pub(super) fn wait_for_table_cache_ready(
    state: &mut SystemState,
    tile_id: TableTileId,
    expected_model_key: Option<TableModelKey>,
) {
    let build_start = std::time::Instant::now();
    loop {
        state.handle_async_messages();

        let ready = state.table_runtime.get(&tile_id).is_some_and(|runtime| {
            runtime.model.is_some()
                && runtime.cache.as_ref().is_some_and(|entry| {
                    entry.is_ready()
                        && expected_model_key
                            .map_or(true, |model_key| entry.cache_key.model_key == model_key)
                })
        });
        if ready {
            break;
        }

        if build_start.elapsed().as_secs() > 10 {
            panic!("timed out waiting for table cache build");
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

pub(super) fn trigger_table_cache_build_for_tile(state: &mut SystemState, tile_id: TableTileId) {
    let tile_state = state.user.table_tiles.get(&tile_id).expect("tile state");
    let cache_key = TableCacheKey {
        model_key: tile_state.spec.model_key_for_tile(tile_id),
        display_filter: tile_state.config.display_filter.clone(),
        pinned_filters: tile_state.config.pinned_filters.clone(),
        view_sort: tile_state.config.sort.clone(),
        generation: state
            .user
            .waves
            .as_ref()
            .map_or(0, |waves| waves.cache_generation),
    };
    state.update(Message::BuildTableCache { tile_id, cache_key });
}
