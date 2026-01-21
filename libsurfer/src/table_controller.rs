use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{
    EGUI_CONTEXT, OUTSTANDING_TRANSACTIONS, SystemState,
    displayed_item::DisplayedItem,
    message::{Message, MessageTarget},
    table,
};

enum TableBuildJob {
    Model(Arc<dyn table::TableModel>),
    SignalAnalysis(table::sources::signal_analysis::PreparedSignalAnalysisModelInput),
}

impl SystemState {
    pub(crate) fn handle_table_message(&mut self, message: Message) -> Option<()> {
        match message {
            Message::BuildTableCache { tile_id, cache_key } => {
                self.handle_build_table_cache(tile_id, cache_key)?;
            }
            Message::TableCacheBuilt {
                tile_id,
                revision,
                entry,
                model,
                result,
            } => {
                self.handle_table_cache_built(tile_id, revision, entry, model, result)?;
            }
            Message::AddTableTile { spec } => {
                self.open_table_tile(spec);
            }
            Message::OpenSignalAnalysisWizard => {
                self.user.signal_analysis_wizard_edit_target = None;
                self.user.show_signal_analysis_wizard = self.build_signal_analysis_wizard_dialog();
            }
            Message::RunSignalAnalysis { mut config } => {
                self.user.show_signal_analysis_wizard = None;
                let edit_target = self.user.signal_analysis_wizard_edit_target.take();
                self.preload_signal_analysis_variables(&config);
                let sampling_mode = self.signal_analysis_sampling_mode(&config.sampling.signal);

                if let Some(tile_id) = edit_target
                    && let Some(tile_state) = self.user.table_tiles.get_mut(&tile_id)
                    && let table::TableModelSpec::AnalysisResults {
                        kind: table::AnalysisKind::SignalAnalysisV1,
                        params:
                            table::AnalysisParams::SignalAnalysisV1 {
                                config: previous_config,
                            },
                    } = &tile_state.spec
                {
                    config.run_revision = previous_config.run_revision.wrapping_add(1);
                    tile_state.spec = table::TableModelSpec::AnalysisResults {
                        kind: table::AnalysisKind::SignalAnalysisV1,
                        params: table::AnalysisParams::SignalAnalysisV1 {
                            config: config.clone(),
                        },
                    };

                    // Keep existing view settings while refreshing title from config.
                    tile_state.config.title = table::signal_analysis_title(&config, sampling_mode);

                    self.invalidate_draw_commands();
                    self.trigger_table_cache_build(tile_id);
                    return None;
                }

                self.create_signal_analysis_tile(config);
            }
            Message::RefreshSignalAnalysis { tile_id } => {
                let mut config = {
                    let tile_state = self.user.table_tiles.get(&tile_id)?;
                    match &tile_state.spec {
                        table::TableModelSpec::AnalysisResults {
                            kind: table::AnalysisKind::SignalAnalysisV1,
                            params: table::AnalysisParams::SignalAnalysisV1 { config },
                        } => config.clone(),
                        _ => return None,
                    }
                };

                config.run_revision = config.run_revision.wrapping_add(1);
                self.preload_signal_analysis_variables(&config);

                let tile_state = self.user.table_tiles.get_mut(&tile_id)?;
                tile_state.spec = table::TableModelSpec::AnalysisResults {
                    kind: table::AnalysisKind::SignalAnalysisV1,
                    params: table::AnalysisParams::SignalAnalysisV1 {
                        config: config.clone(),
                    },
                };

                // Preserve existing view config and force cache rebuild via run revision key.
                self.invalidate_draw_commands();
                self.trigger_table_cache_build(tile_id);
            }
            Message::EditSignalAnalysis { tile_id } => {
                let config = {
                    let tile_state = self.user.table_tiles.get(&tile_id)?;
                    match &tile_state.spec {
                        table::TableModelSpec::AnalysisResults {
                            kind: table::AnalysisKind::SignalAnalysisV1,
                            params: table::AnalysisParams::SignalAnalysisV1 { config },
                        } => config.clone(),
                        _ => return None,
                    }
                };

                self.user.show_signal_analysis_wizard =
                    self.build_signal_analysis_wizard_dialog_from_config(&config);
                self.user.signal_analysis_wizard_edit_target = self
                    .user
                    .show_signal_analysis_wizard
                    .as_ref()
                    .map(|_| tile_id);
            }
            Message::OpenSignalChangeList { target } => {
                let waves = self.user.waves.as_ref()?;
                let vidx = match target {
                    MessageTarget::Explicit(vidx) => vidx,
                    MessageTarget::CurrentSelection => waves.focused_item?,
                };
                let item_ref = waves
                    .items_tree
                    .get_visible(vidx)
                    .map(|node| node.item_ref)?;
                let item = waves.displayed_items.get(&item_ref)?;
                if let DisplayedItem::Variable(variable) = item {
                    let spec = table::TableModelSpec::SignalChangeList {
                        variable: variable.variable_ref.clone(),
                        field: Vec::new(),
                    };
                    self.open_table_tile(spec);
                }
            }
            Message::OpenTransactionTable { generator } => {
                let spec = table::TableModelSpec::TransactionTrace { generator };
                self.open_table_tile(spec);
            }
            Message::RemoveTableTile { tile_id } => {
                self.user.table_tiles.remove(&tile_id);
                self.table_runtime.remove(&tile_id);
                self.invalidate_draw_commands();
            }
            Message::SetTableSort { tile_id, sort } => {
                if let Some(tile_state) = self.user.table_tiles.get_mut(&tile_id) {
                    tile_state.config.sort = sort;
                    // Set pending scroll operation to scroll to selection after sort
                    if let Some(runtime) = self.table_runtime.get_mut(&tile_id) {
                        runtime
                            .scroll_state
                            .set_pending_scroll_op(table::PendingScrollOp::AfterSort);
                    }
                    // Cache invalidation happens automatically in draw_table_tile
                    // when the cache_key (which includes view_sort) changes
                    self.invalidate_draw_commands();
                }
            }
            Message::SetTableDisplayFilter { tile_id, filter } => {
                if let Some(tile_state) = self.user.table_tiles.get_mut(&tile_id) {
                    tile_state.config.display_filter = filter.clone();
                    // Set pending scroll operation and sync draft to applied filter.
                    // This handles: Clear button, external API calls, programmatic filter changes.
                    if let Some(runtime) = self.table_runtime.get_mut(&tile_id) {
                        runtime.filter_draft = Some(table::FilterDraft::from_spec(&filter));
                        runtime
                            .scroll_state
                            .set_pending_scroll_op(table::PendingScrollOp::AfterFilter);
                    }
                    // Cache invalidation happens automatically in draw_table_tile
                    // when the cache_key (which includes display_filter) changes
                    self.invalidate_draw_commands();
                }
            }
            Message::SetTablePinnedFilters { tile_id, filters } => {
                if let Some(tile_state) = self.user.table_tiles.get_mut(&tile_id) {
                    tile_state.config.pinned_filters = table::normalize_search_specs(filters);
                    if let Some(runtime) = self.table_runtime.get_mut(&tile_id) {
                        runtime
                            .scroll_state
                            .set_pending_scroll_op(table::PendingScrollOp::AfterFilter);
                    }
                    self.invalidate_draw_commands();
                }
            }
            Message::SetTableSelection { tile_id, selection } => {
                if let Some(runtime) = self.table_runtime.get_mut(&tile_id) {
                    runtime.selection = selection.clone();
                    runtime.update_hidden_count();
                    self.invalidate_draw_commands();
                }

                // Check if we should trigger activation on selection
                if let Some(tile_state) = self.user.table_tiles.get(&tile_id)
                    && tile_state.config.activate_on_select
                    && tile_state.config.selection_mode == table::TableSelectionMode::Single
                    && let Some(anchor) = selection.anchor
                    && let Some(runtime) = self.table_runtime.get(&tile_id)
                    && let Some(model) = &runtime.model
                {
                    self.apply_table_action(model.on_activate(anchor));
                }
            }
            Message::ClearTableSelection { tile_id } => {
                if let Some(runtime) = self.table_runtime.get_mut(&tile_id) {
                    runtime.selection.clear();
                    runtime.hidden_selection_count = 0;
                    self.invalidate_draw_commands();
                }
            }
            Message::TableActivateSelection { tile_id } => {
                // Get the selected rows and call model.on_activate() for each
                let runtime = self.table_runtime.get(&tile_id)?;
                let model = runtime.model.clone()?;

                // For now, activate the anchor row if set
                if let Some(anchor) = runtime.selection.anchor {
                    self.apply_table_action(model.on_activate(anchor));
                }
            }
            Message::TableCopySelection {
                tile_id,
                include_header,
            } => {
                let Some(runtime) = self.table_runtime.get(&tile_id) else {
                    return Some(());
                };
                let Some(model) = runtime.model.clone() else {
                    return Some(());
                };
                let Some(cache_entry) = &runtime.cache else {
                    return Some(());
                };
                let Some(cache) = cache_entry.get() else {
                    return Some(());
                };
                let Some(tile_state) = self.user.table_tiles.get(&tile_id) else {
                    return Some(());
                };

                let schema = model.schema();
                let tsv = table::build_table_copy_payload(
                    model.as_ref(),
                    &schema,
                    &cache.row_ids,
                    &runtime.selection,
                    &tile_state.config.columns,
                    include_header,
                );

                if tsv.is_empty() {
                    return Some(());
                }

                // Copy to clipboard if available
                if let Some(ctx) = &self.context {
                    ctx.copy_text(tsv);
                }
            }
            Message::TableSelectAll { tile_id } => {
                let tile_state = self.user.table_tiles.get(&tile_id)?;

                // Only works in Multi mode
                if tile_state.config.selection_mode != table::TableSelectionMode::Multi {
                    return Some(());
                }

                // Get all visible rows from cache
                let runtime = self.table_runtime.get(&tile_id)?;
                let Some(cache_entry) = &runtime.cache else {
                    return Some(());
                };
                let Some(cache) = cache_entry.get() else {
                    return Some(());
                };

                // Select all visible rows
                let mut new_selection = table::TableSelection::new();
                for &row_id in &cache.row_ids {
                    new_selection.rows.insert(row_id);
                }
                if !cache.row_ids.is_empty() {
                    new_selection.anchor = Some(cache.row_ids[0]);
                }

                if let Some(runtime) = self.table_runtime.get_mut(&tile_id) {
                    runtime.selection = new_selection;
                    runtime.hidden_selection_count = 0; // All visible rows selected
                    self.invalidate_draw_commands();
                }
            }
            Message::ResizeTableColumn {
                tile_id,
                column_key,
                new_width,
            } => {
                let tile_state = self.ensure_columns_initialized(tile_id)?;

                let result = table::resize_column(
                    &tile_state.config.columns,
                    &column_key,
                    new_width,
                    table::MIN_COLUMN_WIDTH,
                );
                if result.changed {
                    tile_state.config.columns = result.columns;
                    self.invalidate_draw_commands();
                }
            }
            Message::ToggleTableColumnVisibility {
                tile_id,
                column_key,
            } => {
                let tile_state = self.ensure_columns_initialized(tile_id)?;

                tile_state.config.columns =
                    table::toggle_column_visibility(&tile_state.config.columns, &column_key);
                self.invalidate_draw_commands();
            }
            Message::SetTableColumnVisibility {
                tile_id,
                visible_columns,
            } => {
                let tile_state = self.ensure_columns_initialized(tile_id)?;

                // Update visibility based on the provided list
                for col in &mut tile_state.config.columns {
                    col.visible = visible_columns.contains(&col.key);
                }
                self.invalidate_draw_commands();
            }
            _ => unreachable!("non-table message dispatched to table controller"),
        }

        Some(())
    }

    fn handle_build_table_cache(
        &mut self,
        tile_id: table::TableTileId,
        cache_key: table::TableCacheKey,
    ) -> Option<()> {
        let reusable_model = self.table_runtime.get(&tile_id).and_then(|runtime| {
            let model = runtime.model.clone()?;
            runtime
                .cache_key
                .as_ref()
                .filter(|existing_key| {
                    existing_key.model_key == cache_key.model_key
                        && existing_key.generation == cache_key.generation
                })
                .map(|_| model)
        });

        {
            let runtime = self.table_runtime.entry(tile_id).or_default();

            if runtime.cache.as_ref().is_some_and(|entry| {
                entry.cache_key == cache_key && entry.generation == cache_key.generation
            }) {
                return None;
            }

            if let Some(entry) = self.table_inflight.get(&cache_key)
                && entry.generation == cache_key.generation
            {
                runtime.cache_key = Some(cache_key.clone());
                runtime.cache = Some(entry.clone());
                return None;
            }
        }

        // Create model from table tile spec. Signal analysis uses a prepared input that
        // lets heavy model construction happen on the worker thread.
        let build_job = if let Some(model) = reusable_model {
            TableBuildJob::Model(model)
        } else {
            match self.user.table_tiles.get(&tile_id) {
                Some(tile_state) => match &tile_state.spec {
                    table::TableModelSpec::AnalysisResults {
                        kind: table::AnalysisKind::SignalAnalysisV1,
                        params: table::AnalysisParams::SignalAnalysisV1 { config },
                    } => {
                        let model_ctx = self.table_model_context();
                        match table::sources::signal_analysis::prepare_signal_analysis_model_input(
                            config, &model_ctx,
                        ) {
                            Ok(prepared) => TableBuildJob::SignalAnalysis(prepared),
                            Err(err) => {
                                if let Some(runtime) = self.table_runtime.get_mut(&tile_id) {
                                    runtime.last_error = Some(err);
                                }
                                return None;
                            }
                        }
                    }
                    _ => {
                        let model_ctx = self.table_model_context();
                        match tile_state.spec.create_model(&model_ctx) {
                            Ok(model) => TableBuildJob::Model(model),
                            Err(err) => {
                                if let Some(runtime) = self.table_runtime.get_mut(&tile_id) {
                                    runtime.last_error = Some(err);
                                }
                                return None;
                            }
                        }
                    }
                },
                None => {
                    if let Some(runtime) = self.table_runtime.get_mut(&tile_id) {
                        runtime.last_error = Some(table::TableCacheError::ModelNotFound {
                            description: "Table tile not found".to_string(),
                        });
                    }
                    return None;
                }
            }
        };

        let runtime = self.table_runtime.get_mut(&tile_id)?;

        // Cancel any in-flight build for this tile
        runtime.cancel_token.store(true, Ordering::Relaxed);
        runtime.table_revision += 1;
        let revision = runtime.table_revision;

        let cancel_token = Arc::new(AtomicBool::new(false));
        runtime.cancel_token = cancel_token.clone();

        let entry = Arc::new(table::TableCacheEntry::new(
            cache_key.clone(),
            cache_key.generation,
            revision,
        ));

        runtime.cache_key = Some(cache_key.clone());
        runtime.cache = Some(entry.clone());
        runtime.last_error = None;
        runtime.model = match &build_job {
            TableBuildJob::Model(model) => Some(model.clone()),
            TableBuildJob::SignalAnalysis(_) => None,
        };

        self.table_inflight.insert(cache_key.clone(), entry.clone());

        let sender = self.channels.msg_sender.clone();
        let cache_key_for_build = cache_key.clone();
        crate::async_util::perform_work(move || {
            let (model, result) = match build_job {
                TableBuildJob::Model(model) => {
                    let result = table::build_table_cache_with_pinned_filters(
                        model.clone(),
                        cache_key_for_build.display_filter.clone(),
                        cache_key_for_build.pinned_filters.clone(),
                        cache_key_for_build.view_sort.clone(),
                        Some(cancel_token),
                    );
                    (Some(model), result)
                }
                TableBuildJob::SignalAnalysis(prepared) => {
                    match table::sources::SignalAnalysisResultsModel::from_prepared(prepared)
                        .map(|model| Arc::new(model) as Arc<dyn table::TableModel>)
                    {
                        Ok(model) => {
                            let result = table::build_table_cache_with_pinned_filters(
                                model.clone(),
                                cache_key_for_build.display_filter.clone(),
                                cache_key_for_build.pinned_filters.clone(),
                                cache_key_for_build.view_sort.clone(),
                                Some(cancel_token),
                            );
                            (Some(model), result)
                        }
                        Err(err) => (None, Err(err)),
                    }
                }
            };

            let msg = Message::TableCacheBuilt {
                tile_id,
                revision,
                entry: entry.clone(),
                model,
                result,
            };

            OUTSTANDING_TRANSACTIONS.fetch_add(1, Ordering::SeqCst);
            let _ = sender.send(msg);

            if let Some(ctx) = EGUI_CONTEXT.read().unwrap().as_ref() {
                ctx.request_repaint();
            }
        });

        Some(())
    }

    fn handle_table_cache_built(
        &mut self,
        tile_id: table::TableTileId,
        revision: u64,
        entry: Arc<table::TableCacheEntry>,
        model: Option<Arc<dyn table::TableModel>>,
        result: Result<table::TableCache, table::TableCacheError>,
    ) -> Option<()> {
        OUTSTANDING_TRANSACTIONS.fetch_sub(1, Ordering::SeqCst);
        let Some(runtime) = self.table_runtime.get_mut(&tile_id) else {
            self.table_inflight.remove(&entry.cache_key);
            return None;
        };

        // Discard stale results from superseded builds
        if revision != runtime.table_revision {
            return None;
        }

        // Defense-in-depth: also check cache key
        if runtime.cache_key.as_ref() != Some(&entry.cache_key) {
            self.table_inflight.remove(&entry.cache_key);
            return None;
        }

        // Defense-in-depth: message revision should match entry revision
        if entry.revision != revision {
            return None;
        }

        if let Some(model) = model {
            runtime.model = Some(model);
        }

        self.table_inflight.remove(&entry.cache_key);
        runtime.cache = Some(entry.clone());

        match result {
            Ok(cache) => {
                entry.set(cache);
                runtime.last_error = None;
            }
            Err(table::TableCacheError::Cancelled) => {
                // Silently discard cancelled builds
            }
            Err(err) => {
                runtime.last_error = Some(err);
            }
        }

        runtime.update_hidden_count();
        self.invalidate_draw_commands();

        Some(())
    }
}
