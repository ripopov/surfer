use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use crate::{
    EGUI_CONTEXT, OUTSTANDING_TRANSACTIONS, SystemState,
    konata::{
        KonataBookmark, KonataBuildInput, KonataModel, KonataModelEntry, KonataModelKey,
        KonataModelSpec, KonataRecordProjector, KonataReloadAnchor, KonataTileId, KonataTileState,
        KonataViewport, KonataViewportMotion,
    },
    message::Message,
    source::SourceId,
    source::SourceTransactionRef,
    transaction_container::{TransactionRef, TransactionStreamRef},
};

use ftr_parser::types::{GeneratorId, StreamId, Transaction};

/// Whether a Konata tile participates in the waveform-linked synchronization group (group 0),
/// which the "Synchronize scroll" checkbox joins. Legacy state files that only set
/// `synchronize_scroll` are treated as group 0.
fn konata_waveform_sync_enabled(tile: &KonataTileState) -> bool {
    tile.config
        .sync_group
        .or(tile.config.synchronize_scroll.then_some(0))
        == Some(0)
}

fn send_konata_find_result(
    sender: &std::sync::mpsc::Sender<Message>,
    tile_id: KonataTileId,
    revision: u64,
    result: Result<crate::konata::KonataSearchHits, String>,
) {
    let _ = sender.send(Message::KonataFindFinished {
        tile_id,
        revision,
        result,
    });
    if let Ok(context) = EGUI_CONTEXT.read()
        && let Some(context) = context.as_ref()
    {
        context.request_repaint();
    }
}

struct KonataProgressivePublisher {
    entry: Arc<KonataModelEntry>,
    parent_generator: GeneratorId,
    event_generator: GeneratorId,
    stream: StreamId,
    detail_cache_budget: usize,
    next_rows: usize,
}

impl KonataProgressivePublisher {
    fn new(
        entry: Arc<KonataModelEntry>,
        parent_generator: GeneratorId,
        event_generator: GeneratorId,
        stream: StreamId,
        detail_cache_budget: usize,
    ) -> Self {
        Self {
            entry,
            parent_generator,
            event_generator,
            stream,
            detail_cache_budget,
            next_rows: 512,
        }
    }

    fn publish_if_due(&mut self, parents: &[Transaction], events: &[Transaction]) {
        if self.next_rows > 4096 || parents.len() < self.next_rows || events.is_empty() {
            return;
        }
        while self.next_rows <= parents.len() {
            self.next_rows = self.next_rows.saturating_mul(2).max(parents.len() + 1);
        }
        let model = Arc::new(KonataModel::build(KonataBuildInput {
            parent_generator: self.parent_generator,
            event_generator: self.event_generator,
            stream: self.stream,
            parents: Arc::new(parents.to_vec()),
            events: Arc::new(events.to_vec()),
            relations: Arc::new(Vec::new()),
        }));
        model.set_detail_cache_budget_bytes(self.detail_cache_budget);
        self.entry.publish(model);
    }

    fn accepts_more(&self) -> bool {
        self.next_rows <= 4096
    }
}

impl SystemState {
    pub(crate) fn handle_konata_message(&mut self, message: Message) -> Option<()> {
        match message {
            Message::OpenKonataView { source, generator } => {
                self.open_konata_tile(source, generator)?;
            }
            Message::OpenFirstKonataView => {
                let (source, generator) = self.first_konata_generator()?;
                self.open_konata_tile(source, generator)?;
            }
            Message::SetActiveKonataMinimap(show) => {
                let tile_id = self.active_konata_tile?;
                self.user
                    .konata_tiles
                    .get_mut(&tile_id)?
                    .config
                    .show_minimap = show;
                self.invalidate_draw_commands();
            }
            Message::ShowActiveKonataOnly => {
                let tile_id = self.active_konata_tile?;
                self.user.tile_tree.show_only_konata(tile_id);
                self.invalidate_draw_commands();
            }
            Message::BuildKonataModel { tile_id } => {
                self.build_konata_model(tile_id)?;
            }
            Message::KonataModelBuilt { entry, result } => {
                OUTSTANDING_TRANSACTIONS.fetch_sub(1, Ordering::SeqCst);
                let model = result.as_ref().ok().cloned();
                entry.complete(result);
                let searches_to_refresh = model.as_ref().map_or_else(Vec::new, |_| {
                    self.konata_runtime
                        .iter()
                        .filter(|(_, runtime)| {
                            runtime
                                .entry
                                .as_ref()
                                .is_some_and(|candidate| Arc::ptr_eq(candidate, &entry))
                        })
                        .filter_map(|(tile_id, runtime)| {
                            let pattern = if runtime.find_searching {
                                Some(runtime.find_query.clone())
                            } else {
                                runtime.find_valid_pattern.clone()
                            }?;
                            (!pattern.is_empty()).then_some((*tile_id, pattern))
                        })
                        .collect::<Vec<_>>()
                });
                for runtime in self.konata_runtime.values_mut().filter(|runtime| {
                    runtime
                        .entry
                        .as_ref()
                        .is_some_and(|candidate| Arc::ptr_eq(candidate, &entry))
                }) {
                    runtime.model_progress = 1.0;
                    runtime.model_phase = "Pipeline projection ready".to_string();
                }
                if let Some(model) = model {
                    let tile_ids = self
                        .konata_runtime
                        .iter()
                        .filter(|(_, runtime)| {
                            runtime
                                .entry
                                .as_ref()
                                .is_some_and(|candidate| Arc::ptr_eq(candidate, &entry))
                        })
                        .map(|(tile_id, _)| *tile_id)
                        .collect::<Vec<_>>();
                    for tile_id in tile_ids {
                        self.restore_konata_reload_anchor(tile_id, &model);
                    }
                }
                for (tile_id, pattern) in searches_to_refresh {
                    let _ = self.start_konata_find(tile_id, pattern);
                }
                self.invalidate_draw_commands();
            }
            Message::KonataModelProgress {
                entry,
                fraction,
                phase,
            } => {
                for runtime in self.konata_runtime.values_mut().filter(|runtime| {
                    runtime
                        .entry
                        .as_ref()
                        .is_some_and(|candidate| Arc::ptr_eq(candidate, &entry))
                }) {
                    runtime.model_progress = fraction.clamp(0.0, 1.0);
                    runtime.model_phase.clone_from(&phase);
                }
                self.invalidate_draw_commands();
            }
            Message::RemoveKonataTile { tile_id } => {
                self.remove_konata_tile(tile_id);
            }
            Message::KonataGotoRow(row) => {
                if self
                    .navigate_active_konata(|_, _| usize::try_from(row).ok())
                    .is_none()
                {
                    self.update(Message::Error(eyre::eyre!(
                        "Konata row {row} is unavailable"
                    )));
                }
            }
            Message::KonataGotoRid(rid) => {
                if self
                    .navigate_active_konata(|model, _| model.row_for_rid(rid))
                    .is_none()
                {
                    self.update(Message::Error(eyre::eyre!(
                        "Retire ID {rid} is missing or ambiguous; specify a thread in the table"
                    )));
                }
            }
            Message::KonataGotoThreadRid { thread, rid } => {
                if self
                    .navigate_active_konata(|model, _| model.row_for_thread_rid(Some(&thread), rid))
                    .is_none()
                {
                    self.update(Message::Error(eyre::eyre!(
                        "Retire ID {rid} for thread {thread} is unavailable"
                    )));
                }
            }
            Message::KonataGotoSid(sid) => {
                if self
                    .navigate_active_konata(|model, _| model.row_for_sid(sid))
                    .is_none()
                {
                    self.update(Message::Error(eyre::eyre!(
                        "Simulator serial ID {sid} is missing or ambiguous"
                    )));
                }
            }
            Message::KonataGotoCycle(cycle) => {
                if self.goto_active_konata_cycle(cycle).is_none() {
                    self.update(Message::Error(eyre::eyre!(
                        "A positive pipeline clock period is required for cycle navigation"
                    )));
                }
            }
            Message::KonataZoomIn => {
                self.zoom_active_konata(false)?;
            }
            Message::KonataZoomOut => {
                self.zoom_active_konata(true)?;
            }
            Message::KonataBookmarkSet(slot) => {
                self.set_konata_bookmark(slot)?;
            }
            Message::KonataBookmarkGoto(slot) => {
                self.goto_konata_bookmark(slot)?;
            }
            Message::StartKonataFind { tile_id, pattern } => {
                self.start_konata_find(tile_id, pattern)?;
            }
            Message::KonataFindNext { tile_id, reverse } => {
                self.navigate_konata_find(tile_id, reverse)?;
            }
            Message::OpenKonataFindTable { tile_id, pattern } => {
                self.open_konata_find_table(tile_id, pattern)?;
            }
            Message::OpenKonataEventTable { tile_id, parent_tx } => {
                self.open_konata_event_table(tile_id, parent_tx)?;
            }
            Message::OpenKonataStatistics { tile_id } => {
                self.open_konata_statistics(tile_id, None)?;
            }
            Message::OpenKonataRangeStatistics { tile_id, range } => {
                self.open_konata_statistics(tile_id, Some(range))?;
            }
            Message::CancelKonataFind { tile_id } => {
                self.cancel_konata_find(tile_id)?;
            }
            Message::ToggleKonataProducerChain { tile_id, row } => {
                let model = self.active_konata_model(tile_id)?;
                let runtime = self.konata_runtime.get_mut(&tile_id)?;
                if runtime.producer_chain_root == Some(row) {
                    runtime.producer_chain_cancel.store(true, Ordering::Relaxed);
                    runtime.producer_chain = None;
                    runtime.producer_chain_root = None;
                    runtime.producer_chain_revision =
                        runtime.producer_chain_revision.wrapping_add(1);
                } else {
                    runtime.producer_chain_root = Some(row);
                    runtime.producer_chain = None;
                    runtime.producer_chain_cancel.store(true, Ordering::Relaxed);
                    runtime.producer_chain_cancel = Arc::new(false.into());
                    runtime.producer_chain_revision =
                        runtime.producer_chain_revision.wrapping_add(1);
                    let revision = runtime.producer_chain_revision;
                    let cancel = runtime.producer_chain_cancel.clone();
                    let sender = self.channels.msg_sender.clone();
                    OUTSTANDING_TRANSACTIONS.fetch_add(1, Ordering::SeqCst);
                    crate::async_util::perform_async_work(async move {
                        let result = model.producer_chain_cooperative(row, &cancel).await;
                        let _ = sender.send(Message::KonataProducerChainBuilt {
                            tile_id,
                            revision,
                            row,
                            result,
                        });
                        if let Ok(context) = EGUI_CONTEXT.read()
                            && let Some(context) = context.as_ref()
                        {
                            context.request_repaint();
                        }
                    });
                }
                self.invalidate_draw_commands();
            }
            Message::KonataProducerChainBuilt {
                tile_id,
                revision,
                row,
                result,
            } => {
                OUTSTANDING_TRANSACTIONS.fetch_sub(1, Ordering::SeqCst);
                let runtime = self.konata_runtime.get_mut(&tile_id)?;
                if runtime.producer_chain_revision == revision
                    && runtime.producer_chain_root == Some(row)
                {
                    runtime.producer_chain = result.map(Arc::new);
                }
                self.invalidate_draw_commands();
            }
            Message::KonataFindFinished {
                tile_id,
                revision,
                result,
            } => {
                self.finish_konata_find(tile_id, revision, result)?;
            }
            Message::KonataFindProgress {
                tile_id,
                revision,
                processed,
                matches,
            } => {
                self.progress_konata_find(tile_id, revision, processed, &matches)?;
            }
            _ => unreachable!("non-Konata message dispatched to Konata controller"),
        }
        Some(())
    }

    fn first_konata_generator(&self) -> Option<(SourceId, TransactionStreamRef)> {
        let waves = self.user.waves.as_ref()?;
        waves
            .source_ids()
            .into_iter()
            .flat_map(|source| {
                waves
                    .transactions_for_source(source)
                    .into_iter()
                    .flat_map(move |transactions| {
                        transactions
                            .get_generators()
                            .into_iter()
                            .filter_map(move |generator| {
                                transactions
                                    .event_index()
                                    .events_generator_of(generator.id)
                                    .map(|_| {
                                        (
                                            source,
                                            TransactionStreamRef::new_gen(
                                                generator.stream_id,
                                                generator.id,
                                                generator.name.clone(),
                                            ),
                                        )
                                    })
                            })
                    })
            })
            .min_by_key(|(source, generator)| {
                (
                    source.0,
                    generator.stream_id.0,
                    generator.gen_id.map_or(u64::MAX, |id| id.0),
                    generator.name.clone(),
                )
            })
    }

    fn open_konata_tile(
        &mut self,
        source: SourceId,
        generator: TransactionStreamRef,
    ) -> Option<()> {
        let waves = self.user.waves.as_ref()?;
        let transactions = waves.transactions_for_source(source)?;
        let parent_generator = generator.gen_id?;
        transactions
            .event_index()
            .events_generator_of(parent_generator)?;

        let source_label = waves
            .source_label_for(source)
            .unwrap_or_else(|| "trace".to_string());
        let tile_id = self.user.tile_tree.next_konata_id();
        self.user.konata_tiles.insert(
            tile_id,
            KonataTileState {
                title: format!("Konata — {} ({source_label})", generator.name),
                spec: KonataModelSpec { source, generator },
                config: self.user.config.konata.view_config(),
                viewport: KonataViewport::default(),
            },
        );
        self.active_konata_tile = Some(tile_id);
        self.user.tile_tree.add_konata_tile(tile_id);
        self.invalidate_draw_commands();
        self.build_konata_model(tile_id)?;
        Some(())
    }

    fn build_konata_model(&mut self, tile_id: KonataTileId) -> Option<()> {
        let tile = self.user.konata_tiles.get(&tile_id)?.clone();

        let (key, input, file_job, remote_job) = {
            let waves = self.user.waves.as_ref()?;
            let transactions = waves.transactions_for_source(tile.spec.source)?;
            let parent_generator = tile.spec.generator.gen_id?;
            let event_generator = transactions
                .event_index()
                .events_generator_of(parent_generator)?;
            let parent = transactions.get_generator(parent_generator)?;
            let events = transactions.get_generator(event_generator)?;
            let generation = waves
                .cache_generation_for_source(tile.spec.source)
                .unwrap_or_default();
            let key = KonataModelKey {
                source: tile.spec.source,
                stream: tile.spec.generator.stream_id,
                parent_generator,
                event_generator,
                generation,
            };
            let loaded = transactions
                .get_stream(tile.spec.generator.stream_id)
                .is_some_and(|stream| stream.transactions_loaded);
            let input = loaded.then(|| KonataBuildInput {
                parent_generator,
                event_generator,
                stream: tile.spec.generator.stream_id,
                parents: parent.transactions.clone(),
                events: events.transactions.clone(),
                relations: transactions.inner.tx_relations.clone(),
            });
            let remote_job = (!loaded)
                .then(|| transactions.remote_source().cloned())
                .flatten()
                .map(|remote| {
                    (
                        remote,
                        tile.spec.generator.stream_id,
                        parent_generator,
                        event_generator,
                    )
                });
            let file_job = if loaded || remote_job.is_some() {
                None
            } else {
                transactions.inner.file_path().map(|path| {
                    (
                        path.to_path_buf(),
                        tile.spec.generator.stream_id,
                        parent_generator,
                        event_generator,
                    )
                })
            };
            (key, input, file_job, remote_job)
        };

        if file_job.is_some()
            && let Some(waves) = self.user.waves.as_mut()
            && let Some(transactions) = waves.transactions_for_source_mut(tile.spec.source)
        {
            transactions.release_relations_if_unloaded();
        }

        if input.is_none() && file_job.is_none() && remote_job.is_none() {
            let entry = Arc::new(KonataModelEntry::new(key.clone()));
            entry.complete(Err(Arc::from(
                "The transaction body is unavailable for this Konata source",
            )));
            self.konata_models.insert(key, entry.clone());
            self.konata_runtime.entry(tile_id).or_default().entry = Some(entry);
            return None;
        }

        let generation_changed = self
            .konata_runtime
            .get(&tile_id)
            .and_then(|runtime| runtime.entry.as_ref())
            .is_some_and(|entry| entry.key.generation != key.generation);
        if generation_changed {
            self.capture_konata_reload_anchor(tile_id);
        }

        if let Some(entry) = self.konata_models.get(&key).cloned() {
            let runtime = self.konata_runtime.entry(tile_id).or_default();
            runtime.requested_generation = Some(key.generation);
            runtime.entry = Some(entry);
            if let Some(model) = runtime.entry.as_ref().and_then(|entry| entry.model()) {
                self.restore_konata_reload_anchor(tile_id, &model);
            }
            return None;
        }

        let entry = Arc::new(KonataModelEntry::new(key.clone()));
        self.konata_models.insert(key.clone(), entry.clone());
        let runtime = self.konata_runtime.entry(tile_id).or_default();
        runtime.cancel_token.store(true, Ordering::Relaxed);
        runtime.cancel_token = Arc::new(false.into());
        runtime.revision = runtime.revision.wrapping_add(1);
        runtime.requested_generation = Some(key.generation);
        runtime.entry = Some(entry.clone());
        runtime.model_progress = 0.0;
        runtime.model_phase = "Preparing pipeline projection".to_string();

        let cancel = runtime.cancel_token.clone();
        let sender = self.channels.msg_sender.clone();
        let file_backed = file_job.is_some();
        let detail_cache_budget = self.user.config.konata.detail_cache_bytes();
        OUTSTANDING_TRANSACTIONS.fetch_add(1, Ordering::SeqCst);
        if let Some((remote, stream, parent_generator, event_generator)) = remote_job {
            crate::async_util::perform_async_work(async move {
                let _ = sender.send(Message::KonataModelProgress {
                    entry: entry.clone(),
                    fraction: 0.05,
                    phase: "Reading remote transaction manifest".to_string(),
                });
                let progress_sender = sender.clone();
                let progress_entry = entry.clone();
                let mut publisher = KonataProgressivePublisher::new(
                    entry.clone(),
                    parent_generator,
                    event_generator,
                    stream,
                    detail_cache_budget,
                );
                let projection_result = crate::remote::get_transaction_projection(
                    &remote,
                    stream,
                    parent_generator,
                    event_generator,
                    &cancel,
                    move |fraction, phase| {
                        let _ = progress_sender.send(Message::KonataModelProgress {
                            entry: progress_entry.clone(),
                            fraction: 0.10 + 0.65 * fraction,
                            phase: phase.to_string(),
                        });
                    },
                    move |parents, events| {
                        publisher.publish_if_due(parents, events);
                    },
                )
                .await
                .map_err(|error| Arc::<str>::from(format!("{error:#}")));
                let result = match projection_result {
                    Ok((records, projection)) => {
                        if cancel.load(Ordering::Relaxed) {
                            Err(Arc::<str>::from("Konata model build cancelled"))
                        } else {
                            let _ = sender.send(Message::KonataModelProgress {
                                entry: entry.clone(),
                                fraction: 0.80,
                                phase: "Building remote pipeline indexes".to_string(),
                            });
                            let model = Arc::new(records.finish(projection, true).await);
                            model.set_detail_cache_budget_bytes(detail_cache_budget);
                            Ok(model)
                        }
                    }
                    Err(error) => Err(error),
                };
                let _ = sender.send(Message::KonataModelBuilt {
                    entry: entry.clone(),
                    result,
                });
                if let Ok(context) = EGUI_CONTEXT.read()
                    && let Some(context) = context.as_ref()
                {
                    context.request_repaint();
                }
            });
            return Some(());
        }
        crate::async_util::perform_async_work(async move {
            let _ = sender.send(Message::KonataModelProgress {
                entry: entry.clone(),
                fraction: if file_backed { 0.05 } else { 0.25 },
                phase: if file_backed {
                    "Reading FTR directory and relations"
                } else {
                    "Normalizing loaded transactions"
                }
                .to_string(),
            });
            let result = async {
                if cancel.load(Ordering::Relaxed) {
                    return Err(Arc::<str>::from("Konata model build cancelled"));
                }
                let (input, records) = if let Some(input) = input {
                    (Some(input), None)
                } else {
                    let (path, stream, parent_generator, event_generator) =
                        file_job.expect("file job checked before worker launch");
                    let mut ftr = ftr_parser::parse::parse_ftr(path).map_err(Arc::<str>::from)?;
                    let total_bytes = ftr
                        .get_stream(stream)
                        .map(|stream| {
                            stream
                                .tx_blocks
                                .iter()
                                .map(|block| block.encoded_len)
                                .sum::<u64>()
                        })
                        .unwrap_or_default()
                        .max(1);
                    let mut processed_bytes = 0u64;
                    let mut records =
                        KonataRecordProjector::new(parent_generator, event_generator, stream, 0);
                    let mut progressive_parents = Vec::new();
                    let mut progressive_events = Vec::new();
                    let mut publisher = KonataProgressivePublisher::new(
                        entry.clone(),
                        parent_generator,
                        event_generator,
                        stream,
                        detail_cache_budget,
                    );
                    ftr.visit_stream_blocks_unlinked(stream, |block, transactions| {
                        if cancel.load(Ordering::Relaxed) {
                            return Err("Konata model build cancelled".to_string());
                        }
                        records.push(transactions);
                        if publisher.accepts_more() {
                            let remaining = 4096usize.saturating_sub(progressive_parents.len());
                            progressive_parents.extend(
                                transactions
                                    .iter()
                                    .filter(|transaction| {
                                        transaction.get_gen_id() == parent_generator
                                    })
                                    .take(remaining)
                                    .cloned(),
                            );
                            if progressive_events.is_empty()
                                && let Some(event) = transactions
                                    .iter()
                                    .find(|transaction| transaction.get_gen_id() == event_generator)
                                    .cloned()
                            {
                                progressive_events.push(event);
                            }
                            publisher.publish_if_due(&progressive_parents, &progressive_events);
                        }
                        processed_bytes = processed_bytes
                            .saturating_add(block.encoded_len)
                            .min(total_bytes);
                        let _ = sender.send(Message::KonataModelProgress {
                            entry: entry.clone(),
                            fraction: 0.25 + 0.4 * processed_bytes as f32 / total_bytes as f32,
                            phase: "Streaming transaction blocks".to_string(),
                        });
                        Ok(())
                    })
                    .map_err(Arc::<str>::from)?;
                    let mut projector = records.relation_projector();
                    ftr.visit_relation_blocks(|_, relations| {
                        if cancel.load(Ordering::Relaxed) {
                            return Err("Konata model build cancelled".to_string());
                        }
                        projector.push(relations);
                        Ok(())
                    })
                    .map_err(Arc::<str>::from)?;
                    let _ = sender.send(Message::KonataModelProgress {
                        entry: entry.clone(),
                        fraction: 0.70,
                        phase: "Building pipeline indexes".to_string(),
                    });
                    (None, Some((records, projector.finish())))
                };
                if cancel.load(Ordering::Relaxed) {
                    return Err(Arc::<str>::from("Konata model build cancelled"));
                }
                if !file_backed {
                    let _ = sender.send(Message::KonataModelProgress {
                        entry: entry.clone(),
                        fraction: 0.65,
                        phase: "Building pipeline indexes".to_string(),
                    });
                }
                let model =
                    Arc::new(match records {
                        Some((records, projection)) => records.finish(projection, true).await,
                        None => {
                            KonataModel::build_cooperative(input.expect(
                                "loaded model input exists when no record projection exists",
                            ))
                            .await
                        }
                    });
                model.set_detail_cache_budget_bytes(detail_cache_budget);
                Ok(model)
            }
            .await;
            let _ = sender.send(Message::KonataModelBuilt {
                entry: entry.clone(),
                result,
            });
            if let Ok(context) = EGUI_CONTEXT.read()
                && let Some(context) = context.as_ref()
            {
                context.request_repaint();
            }
        });
        Some(())
    }

    pub(crate) fn capture_konata_reload_anchor(&mut self, tile_id: KonataTileId) -> Option<()> {
        let model = self.active_konata_model(tile_id)?;
        let tile = self.user.konata_tiles.get(&tile_id)?;
        let visible = tile.viewport.top_visible_row.floor().max(0.0) as usize;
        let row = if tile.config.hide_flushed {
            model.visibility.select(visible)?
        } else {
            visible.min(model.row_count().checked_sub(1)?)
        };
        let left = tile.viewport.left_tick as f64 + tile.viewport.left_frac;
        self.konata_runtime.get_mut(&tile_id)?.reload_anchor = Some(KonataReloadAnchor {
            tx_id: model.rows.tx_id[row],
            fallback_tick: model.rows.begin[row],
            left_offset: left - model.rows.begin[row] as f64,
            row_offset: tile.viewport.top_visible_row - visible as f64,
            px_per_tick: tile.viewport.px_per_tick,
            row_height_px: tile.viewport.row_height_px,
        });
        Some(())
    }

    pub(crate) fn restore_konata_reload_anchor(
        &mut self,
        tile_id: KonataTileId,
        model: &KonataModel,
    ) -> Option<()> {
        let anchor = self
            .konata_runtime
            .get_mut(&tile_id)?
            .reload_anchor
            .take()?;
        let row = model
            .row_for_transaction(anchor.tx_id)
            .or_else(|| model.nearest_row_for_tick(anchor.fallback_tick))?;
        let tile = self.user.konata_tiles.get_mut(&tile_id)?;
        let visible = if tile.config.hide_flushed {
            model.visibility.rank(row)
        } else {
            row
        };
        tile.viewport.top_visible_row = visible as f64 + anchor.row_offset;
        let left = model.rows.begin[row] as f64 + anchor.left_offset;
        let integral = left.floor().clamp(i64::MIN as f64, i64::MAX as f64);
        tile.viewport.set_left_tick(integral as i64);
        tile.viewport.left_frac = left - integral;
        tile.viewport.px_per_tick = anchor.px_per_tick;
        tile.viewport.row_height_px = anchor.row_height_px;
        Some(())
    }

    fn remove_konata_tile(&mut self, tile_id: KonataTileId) {
        let removed_spec = self
            .user
            .konata_tiles
            .get(&tile_id)
            .map(|tile| tile.spec.clone());
        self.user.tile_tree.remove_konata_tile(tile_id);
        self.user.konata_tiles.remove(&tile_id);
        let removed_spec_still_available = removed_spec.as_ref().is_some_and(|removed| {
            self.user
                .konata_tiles
                .values()
                .any(|tile| &tile.spec == removed)
        });
        for tile in self.user.konata_tiles.values_mut() {
            if tile.config.overlay_tile == Some(tile_id)
                || !removed_spec_still_available
                    && tile.config.overlay.as_ref() == removed_spec.as_ref()
            {
                tile.config.overlay = None;
                tile.config.overlay_tile = None;
            }
        }
        if self.active_konata_tile == Some(tile_id) {
            self.active_konata_tile = self.user.konata_tiles.keys().next().copied();
        }
        if let Some(runtime) = self.konata_runtime.remove(&tile_id) {
            runtime.cancel_token.store(true, Ordering::Relaxed);
            runtime.producer_chain_cancel.store(true, Ordering::Relaxed);
        }
        self.retain_used_konata_models();
        self.invalidate_draw_commands();
    }

    pub(crate) fn retain_used_konata_models(&mut self) {
        self.konata_models.retain(|key, _| {
            self.user.konata_tiles.values().any(|tile| {
                tile.spec.source == key.source
                    && tile.spec.generator.stream_id == key.stream
                    && tile.spec.generator.gen_id == Some(key.parent_generator)
            }) || self.user.table_tiles.values().any(|tile| {
                matches!(
                    &tile.spec,
                    crate::table::TableModelSpec::KonataInstructions { spec }
                        | crate::table::TableModelSpec::KonataEvents { spec, .. }
                        | crate::table::TableModelSpec::KonataStatistics { spec, .. }
                        if spec.source == key.source
                            && spec.generator.stream_id == key.stream
                            && spec.generator.gen_id == Some(key.parent_generator)
                )
            })
        });
    }

    pub(crate) fn active_konata_tile(&self) -> Option<KonataTileId> {
        self.active_konata_tile
            .filter(|tile| self.user.konata_tiles.contains_key(tile))
            .or_else(|| self.user.konata_tiles.keys().next().copied())
    }

    pub(crate) fn active_konata_model(&self, tile_id: KonataTileId) -> Option<Arc<KonataModel>> {
        self.konata_runtime.get(&tile_id)?.entry.as_ref()?.model()
    }

    /// The representative Konata tile of the waveform-linked synchronization group (group 0),
    /// chosen deterministically as the lowest tile id with a drawn canvas. Its siblings follow it
    /// through [`crate::konata::synchronize_tiles`], so bridging the waveform to this one tile is
    /// enough to keep the whole group's time axis in step.
    fn waveform_sync_konata_tile(&self) -> Option<KonataTileId> {
        self.user
            .konata_tiles
            .iter()
            .filter(|(_, tile)| konata_waveform_sync_enabled(tile))
            .filter(|(id, _)| {
                self.konata_runtime
                    .get(id)
                    .is_some_and(|runtime| runtime.canvas_size.x > 0.0)
            })
            .map(|(id, _)| *id)
            .min_by_key(|id| id.0)
    }

    /// Keep the primary waveform viewport and the Konata "Synchronize scroll" group showing the
    /// same time window on the X axis. Called once per frame after messages are applied, so both
    /// the waveform viewports and the Konata tiles reflect this frame's user input.
    ///
    /// Returns `true` when a viewport was moved, so the caller can request a repaint.
    pub(crate) fn synchronize_konata_wave_viewports(&mut self) -> bool {
        use crate::viewport::Absolute;
        use crate::viewport_sync::SyncParticipant;

        // The feature is off unless at least one Konata tile opts in to group 0.
        let group_present = self
            .user
            .konata_tiles
            .values()
            .any(konata_waveform_sync_enabled);
        if !group_present {
            self.viewport_sync = crate::viewport_sync::ViewportSyncState::default();
            return false;
        }

        let Some(waves) = self.user.waves.as_ref() else {
            return false;
        };
        if waves.viewports.is_empty() {
            return false;
        }
        let num_timestamps = waves.safe_canvas_num_timestamps();

        let rep = self.waveform_sync_konata_tile();

        // Snapshot each participant's currently visible window (raw trace ticks).
        let mut participants: Vec<(SyncParticipant, (f64, f64))> = Vec::new();
        let (wave_left, wave_right) = waves.viewports[0].absolute_range(&num_timestamps);
        participants.push((
            SyncParticipant::Waveform(0),
            (wave_left.inner(), wave_right.inner()),
        ));
        if let Some(rep) = rep {
            let width = self
                .konata_runtime
                .get(&rep)
                .map_or(0.0, |runtime| runtime.canvas_size.x);
            if let Some(tile) = self.user.konata_tiles.get(&rep) {
                participants.push((
                    SyncParticipant::Konata(rep),
                    tile.viewport.visible_tick_range(width),
                ));
            }
        }

        let primary = rep.map(SyncParticipant::Konata);
        let targets = self.viewport_sync.arbitrate(&participants, primary);

        let mut waveform_changed = false;
        let mut any_changed = false;
        for (participant, (left, right)) in targets {
            any_changed = true;
            match participant {
                SyncParticipant::Waveform(idx) => {
                    let Some(waves) = self.user.waves.as_mut() else {
                        continue;
                    };
                    let Some(viewport) = waves.viewports.get_mut(idx) else {
                        continue;
                    };
                    viewport.set_absolute_range(Absolute(left), Absolute(right), &num_timestamps);
                    let (actual_left, actual_right) = viewport.absolute_range(&num_timestamps);
                    self.viewport_sync
                        .record_applied(participant, (actual_left.inner(), actual_right.inner()));
                    waveform_changed = true;
                }
                SyncParticipant::Konata(id) => {
                    let width = self
                        .konata_runtime
                        .get(&id)
                        .map_or(0.0, |runtime| runtime.canvas_size.x);
                    let Some(tile) = self.user.konata_tiles.get_mut(&id) else {
                        continue;
                    };
                    tile.viewport.set_visible_tick_range(left, right, width);
                    let actual = tile.viewport.visible_tick_range(width);
                    self.viewport_sync.record_applied(participant, actual);
                }
            }
        }

        if waveform_changed {
            self.invalidate_draw_commands();
        }
        any_changed
    }

    fn move_konata_viewport(
        &mut self,
        tile_id: KonataTileId,
        target: KonataViewport,
    ) -> Option<()> {
        let start = self.user.konata_tiles.get(&tile_id)?.viewport;
        let duration = self.user.config.animation_time.clamp(0.08, 0.10);
        let has_canvas = self
            .konata_runtime
            .get(&tile_id)
            .is_some_and(|runtime| runtime.canvas_size != egui::Vec2::ZERO);
        if self.animation_enabled() && has_canvas && self.user.config.animation_time > 0.0 {
            self.konata_runtime
                .entry(tile_id)
                .or_default()
                .viewport_motion = Some(KonataViewportMotion {
                start,
                target,
                elapsed: 0.0,
                duration,
            });
        } else {
            self.user.konata_tiles.get_mut(&tile_id)?.viewport = target;
            self.konata_runtime
                .entry(tile_id)
                .or_default()
                .viewport_motion = None;
        }
        self.invalidate_draw_commands();
        Some(())
    }

    fn navigate_active_konata(
        &mut self,
        resolve: impl FnOnce(&KonataModel, &KonataTileState) -> Option<usize>,
    ) -> Option<()> {
        let tile_id = self.active_konata_tile()?;
        let model = self.active_konata_model(tile_id)?;
        let row = resolve(&model, self.user.konata_tiles.get(&tile_id)?)?;
        if row >= model.row_count() {
            return None;
        }
        let tile = self.user.konata_tiles.get(&tile_id)?;
        let visible = if tile.config.hide_flushed {
            model.visibility.rank(row)
        } else {
            row
        };
        let mut target = tile.viewport;
        target.align_row(model.rows.begin[row], visible);
        self.move_konata_viewport(tile_id, target)
    }

    fn goto_active_konata_cycle(&mut self, cycle: i64) -> Option<()> {
        let tile_id = self.active_konata_tile()?;
        let tile = self.user.konata_tiles.get(&tile_id)?;
        let period = i128::from(
            tile.config
                .clock_period_ticks
                .filter(|period| *period > 0)?,
        );
        let tick = i128::from(tile.config.clock_origin_tick) + i128::from(cycle) * period;
        let mut target = tile.viewport;
        target.set_left_tick(tick.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64);
        self.move_konata_viewport(tile_id, target)
    }

    fn zoom_active_konata(&mut self, out: bool) -> Option<()> {
        let tile_id = self.active_konata_tile()?;
        let canvas_size = self
            .konata_runtime
            .get(&tile_id)
            .map_or(egui::Vec2::ZERO, |runtime| runtime.canvas_size);
        let tile = self.user.konata_tiles.get(&tile_id)?;
        let factor = if out {
            1.0 / tile.config.zoom_step
        } else {
            tile.config.zoom_step
        };
        let mut target = tile.viewport;
        target.zoom_at(
            factor,
            f64::from(canvas_size.x) * 0.5,
            f64::from(canvas_size.y) * 0.5,
        );
        self.move_konata_viewport(tile_id, target)
    }

    fn set_konata_bookmark(&mut self, slot: u8) -> Option<()> {
        if slot > 9 {
            return None;
        }
        let tile_id = self.active_konata_tile()?;
        let model = self.active_konata_model(tile_id)?;
        let tile = self.user.konata_tiles.get(&tile_id)?;
        let visible = tile.viewport.top_visible_row.round().max(0.0) as usize;
        let row = if tile.config.hide_flushed {
            model.visibility.select(visible)?
        } else {
            visible.min(model.row_count().checked_sub(1)?)
        };
        let bookmark = KonataBookmark {
            tx_id: model.rows.tx_id[row],
            tick: tile.viewport.left_tick,
            px_per_tick: tile.viewport.px_per_tick,
            row_height_px: tile.viewport.row_height_px,
        };
        let slots = self
            .user
            .konata_bookmarks
            .entry(tile.spec.clone())
            .or_insert_with(|| vec![None; 10]);
        slots.resize(10, None);
        slots[usize::from(slot)] = Some(bookmark);
        Some(())
    }

    fn goto_konata_bookmark(&mut self, slot: u8) -> Option<()> {
        if slot > 9 {
            return None;
        }
        let tile_id = self.active_konata_tile()?;
        let model = self.active_konata_model(tile_id)?;
        let tile_spec = self.user.konata_tiles.get(&tile_id)?.spec.clone();
        let bookmark = self
            .user
            .konata_bookmarks
            .get(&tile_spec)?
            .get(usize::from(slot))
            .copied()
            .flatten()?;
        let fallback_tick = u64::try_from(bookmark.tick).unwrap_or_default();
        let row = model
            .row_for_transaction(bookmark.tx_id)
            .or_else(|| model.nearest_row_for_tick(fallback_tick))?;
        let tile = self.user.konata_tiles.get(&tile_id)?;
        let visible = if tile.config.hide_flushed {
            model.visibility.rank(row)
        } else {
            row
        };
        let mut target = tile.viewport;
        target.top_visible_row = visible as f64;
        target.set_left_tick(bookmark.tick);
        target.px_per_tick = bookmark.px_per_tick;
        target.row_height_px = bookmark.row_height_px;
        self.move_konata_viewport(tile_id, target)
    }

    fn start_konata_find(&mut self, tile_id: KonataTileId, pattern: String) -> Option<()> {
        let model = self.active_konata_model(tile_id)?;
        let anchor_row = self
            .user
            .waves
            .as_ref()
            .and_then(|waves| waves.focused_transaction.0.as_ref())
            .filter(|focused| focused.source == self.user.konata_tiles[&tile_id].spec.source)
            .and_then(|focused| model.row_for_transaction(focused.inner.id.0))
            .or_else(|| {
                let tile = self.user.konata_tiles.get(&tile_id)?;
                let visible = tile.viewport.top_visible_row.round().max(0.0) as usize;
                if tile.config.hide_flushed {
                    model.visibility.select(visible)
                } else {
                    (visible < model.row_count()).then_some(visible)
                }
            })
            .unwrap_or_default();
        let runtime = self.konata_runtime.get_mut(&tile_id)?;
        runtime.find_cancel.store(true, Ordering::Relaxed);
        runtime.find_revision = runtime.find_revision.wrapping_add(1);
        let revision = runtime.find_revision;
        let cancel = Arc::new(AtomicBool::new(false));
        let processed = Arc::new(AtomicUsize::new(0));
        runtime.find_cancel = cancel.clone();
        runtime.find_processed = processed.clone();
        runtime.find_total = model.row_count();
        runtime.find_searching = true;
        runtime.find_partial_first = None;
        runtime.find_partial_count = 0;
        runtime.find_error = None;
        runtime.find_open = true;
        runtime.find_query.clone_from(&pattern);
        runtime.find_anchor_row = anchor_row;

        let sender = self.channels.msg_sender.clone();
        OUTSTANDING_TRANSACTIONS.fetch_add(1, Ordering::SeqCst);
        crate::async_util::perform_async_work(async move {
            let result = match regex::Regex::new(&pattern) {
                Err(error) => Err(error.to_string()),
                Ok(regex) => {
                    let mut hits = crate::konata::KonataSearchHits::empty(model.row_count());
                    let mut text = String::new();
                    let mut page_matches = Vec::new();
                    for offset in 0..model.row_count() {
                        if offset % 256 == 0 {
                            processed.store(offset, Ordering::Relaxed);
                            if cancel.load(Ordering::Relaxed) {
                                return send_konata_find_result(
                                    &sender,
                                    tile_id,
                                    revision,
                                    Err("Search cancelled".to_string()),
                                );
                            }
                            crate::async_util::sleep_ms(0).await;
                        }
                        let row = (anchor_row + 1 + offset) % model.row_count().max(1);
                        model.write_search_text(row, &mut text);
                        if regex.is_match(&text) {
                            hits.insert(row);
                            page_matches.push(row as u64);
                        }
                        if (offset + 1) % 1024 == 0 || offset + 1 == model.row_count() {
                            let _ = sender.send(Message::KonataFindProgress {
                                tile_id,
                                revision,
                                processed: offset + 1,
                                matches: std::mem::take(&mut page_matches),
                            });
                        }
                    }
                    processed.store(model.row_count(), Ordering::Relaxed);
                    hits.finish();
                    Ok(hits)
                }
            };
            send_konata_find_result(&sender, tile_id, revision, result);
        });
        Some(())
    }

    fn progress_konata_find(
        &mut self,
        tile_id: KonataTileId,
        revision: u64,
        processed: usize,
        matches: &[u64],
    ) -> Option<()> {
        let first = {
            let runtime = self.konata_runtime.get_mut(&tile_id)?;
            if runtime.find_revision != revision || !runtime.find_searching {
                return None;
            }
            runtime.find_processed.store(processed, Ordering::Relaxed);
            runtime.find_partial_count = runtime.find_partial_count.saturating_add(matches.len());
            if runtime.find_partial_first.is_none() {
                runtime.find_partial_first =
                    matches.first().and_then(|row| usize::try_from(*row).ok());
                runtime.find_partial_first
            } else {
                None
            }
        };
        if let Some(row) = first {
            self.align_konata_find_result(tile_id, row)?;
        } else {
            self.invalidate_draw_commands();
        }
        Some(())
    }

    fn cancel_konata_find(&mut self, tile_id: KonataTileId) -> Option<()> {
        let runtime = self.konata_runtime.get_mut(&tile_id)?;
        runtime.find_cancel.store(true, Ordering::Relaxed);
        runtime.find_searching = false;
        runtime.find_partial_first = None;
        runtime.find_partial_count = 0;
        if runtime.find_active_row.is_some_and(|row| {
            runtime
                .find_hits
                .as_ref()
                .is_none_or(|hits| !hits.contains(row))
        }) {
            runtime.find_active_row = runtime.find_hits.as_ref().and_then(|hits| hits.first());
        }
        runtime.find_revision = runtime.find_revision.wrapping_add(1);
        self.invalidate_draw_commands();
        Some(())
    }

    fn open_konata_find_table(&mut self, tile_id: KonataTileId, pattern: String) -> Option<()> {
        regex::Regex::new(&pattern).ok()?;
        let spec = self.user.konata_tiles.get(&tile_id)?.spec.clone();
        self.active_konata_model(tile_id)?;
        let table_id =
            self.open_table_tile(crate::table::TableModelSpec::KonataInstructions { spec });
        self.user
            .table_tiles
            .get_mut(&table_id)?
            .config
            .display_filter = crate::table::TableSearchSpec {
            mode: crate::table::TableSearchMode::Regex,
            case_sensitive: true,
            text: pattern,
            column: None,
        };
        self.trigger_table_cache_build(table_id);
        Some(())
    }

    fn open_konata_event_table(
        &mut self,
        tile_id: KonataTileId,
        parent_tx: Option<u64>,
    ) -> Option<()> {
        self.active_konata_model(tile_id)?;
        let spec = self.user.konata_tiles.get(&tile_id)?.spec.clone();
        let table_id =
            self.open_table_tile(crate::table::TableModelSpec::KonataEvents { spec, parent_tx });
        self.trigger_table_cache_build(table_id);
        Some(())
    }

    fn open_konata_statistics(
        &mut self,
        tile_id: KonataTileId,
        range: Option<(u64, u64)>,
    ) -> Option<()> {
        self.active_konata_model(tile_id)?;
        let tile = self.user.konata_tiles.get(&tile_id)?;
        let spec = crate::table::TableModelSpec::KonataStatistics {
            spec: tile.spec.clone(),
            clock_period_ticks: tile.config.clock_period_ticks,
            clock_origin_tick: tile.config.clock_origin_tick,
            range,
            stall_stages: tile.config.stall_stages.clone(),
            stall_case_sensitive: tile.config.stall_case_sensitive,
            instruction_classifier: tile.config.instruction_classifier,
            include_estimated_flush_rates: tile.config.include_estimated_flush_rates,
        };
        let table_id = self.open_table_tile(spec);
        self.trigger_table_cache_build(table_id);
        Some(())
    }

    fn navigate_konata_find(&mut self, tile_id: KonataTileId, reverse: bool) -> Option<()> {
        let runtime = self.konata_runtime.get(&tile_id)?;
        let hits = runtime.find_hits.clone()?;
        let from = runtime
            .find_active_row
            .or_else(|| hits.first())
            .unwrap_or_default();
        let row = hits.next(from, reverse)?;
        self.align_konata_find_result(tile_id, row)
    }

    fn align_konata_find_result(&mut self, tile_id: KonataTileId, row: usize) -> Option<()> {
        let model = self.active_konata_model(tile_id)?;
        let tile = self.user.konata_tiles.get(&tile_id)?;
        let visible = if tile.config.hide_flushed {
            model.visibility.rank(row)
        } else {
            row
        };
        let source = tile.spec.source;
        let mut target = tile.viewport;
        target.align_row(model.rows.begin[row], visible);
        self.move_konata_viewport(tile_id, target)?;
        let tx_id = model.rows.tx_id[row];
        self.user.waves.as_mut()?.focused_transaction = (
            Some(SourceTransactionRef::new(
                source,
                TransactionRef {
                    id: ftr_parser::types::TransactionId(tx_id),
                },
            )),
            None,
        );
        let runtime = self.konata_runtime.get_mut(&tile_id)?;
        runtime.find_active_row = Some(row);
        runtime.suppress_focus_scroll = Some(tx_id);
        self.invalidate_draw_commands();
        Some(())
    }

    fn finish_konata_find(
        &mut self,
        tile_id: KonataTileId,
        revision: u64,
        result: Result<crate::konata::KonataSearchHits, String>,
    ) -> Option<()> {
        OUTSTANDING_TRANSACTIONS.fetch_sub(1, Ordering::SeqCst);
        let runtime = self.konata_runtime.get_mut(&tile_id)?;
        if runtime.find_revision != revision {
            return None;
        }
        runtime.find_searching = false;
        runtime.find_partial_first = None;
        runtime.find_partial_count = 0;
        match result {
            Ok(hits) => {
                let active = hits
                    .next(runtime.find_anchor_row, false)
                    .or_else(|| hits.first());
                runtime.find_hits = Some(Arc::new(hits));
                runtime.find_active_row = active;
                runtime.find_valid_pattern = Some(runtime.find_query.clone());
                runtime.find_error = None;
            }
            Err(error) if error == "Search cancelled" => {}
            Err(error) => runtime.find_error = Some(error),
        }
        if let Some(row) = self
            .konata_runtime
            .get(&tile_id)
            .and_then(|runtime| runtime.find_active_row)
            && self
                .konata_runtime
                .get(&tile_id)
                .is_some_and(|runtime| runtime.find_error.is_none())
        {
            self.align_konata_find_result(tile_id, row)?;
        } else {
            self.invalidate_draw_commands();
        }
        Some(())
    }
}
