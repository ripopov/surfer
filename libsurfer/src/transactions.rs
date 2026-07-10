use egui::{Layout, RichText, TextWrapMode, Ui};
use egui_extras::{Column, TableBody, TableBuilder};
use emath::Align;
use ftr_parser::types::Transaction;
use itertools::Itertools;
use tracing::{info, warn};

use crate::SystemState;
use crate::displayed_item::DisplayedItem;
use crate::message::Message;
use crate::source::{SourceId, SourceTransactionRef};
use crate::transaction_container::{StreamScopeRef, TransactionContainer};
use crate::transaction_container::{TransactionRef, TransactionStreamRef};
use crate::wave_data::ScopeType;
use crate::wave_data::WaveData;

// Transactions file extension
pub const TRANSACTIONS_FILE_EXTENSION: &str = "ftr";

// Constants for transaction table drawing and UI labels
const ROW_HEIGHT: f32 = 15.;
const SECTION_GAP: f32 = 5.;
const SUBHEADER_GAP: f32 = 3.;
const SUBHEADER_SIZE: f32 = 15.;

// Root stream name
const TRANSACTION_ROOT_NAME: &str = "tr";

// Header / section titles
const FOCUSED_TX_DETAILS_HDR: &str = "Focused Transaction Details";
const FOCUSED_EVENT_DETAILS_HDR: &str = "Focused Event Details";
const PROPERTIES_HDR: &str = "Properties";
const ATTRIBUTES_SECTION_TITLE: &str = "Attributes";
const INCOMING_RELATIONS_TITLE: &str = "Incoming Relations";
const OUTGOING_RELATIONS_TITLE: &str = "Outgoing Relations";
const EVENTS_SECTION_TITLE: &str = "Events";
const PARENT_SECTION_TITLE: &str = "Parent";

// FTR event card labels
const EVENT_LABEL: &str = "Event";
const EVENT_TIME_LABEL: &str = "Time";
const EVENT_DURATION_LABEL: &str = "Duration";
const GO_TO_PARENT_LABEL: &str = "Go to parent";
const OPEN_EVENTS_IN_TABLE_LABEL: &str = "Open in table";
const OUT_OF_RANGE_NOTICE: &str = "⚠ outside parent transaction range";
const MULTIPLE_PARENTS_NOTICE: &str = "⚠ multiple parents recorded; showing first";
const ORPHAN_NOTICE: &str = "orphan event (no parent_of relation)";
const RELATION_NAME_LABEL: &str = "Name";
const RELATION_LINK_LABEL: &str = "Source -> Sink";

// Column / field labels
const TX_ID_LABEL: &str = "Transaction ID";
const TX_TYPE_LABEL: &str = "Type";
const START_TIME_LABEL: &str = "Start Time";
const END_TIME_LABEL: &str = "End Time";
const ATTR_NAME_LABEL: &str = "Name";
const ATTR_VALUE_LABEL: &str = "Value";

// Information label
const STREAM_NOT_FOUND_LABEL: &str = "Stream not found";

// FTR event UI labels
const EVENT_BADGE: &str = "⚡";
const SHOW_RAW_EVENT_GENERATORS_LABEL: &str = "Show raw event generators";

impl SystemState {
    pub fn draw_transaction_detail_panel(
        &self,
        parent_ui: &mut Ui,
        max_width: f32,
        msgs: &mut Vec<Message>,
    ) {
        let Some(waves) = self.user.waves.as_ref() else {
            return;
        };
        let (Some(transaction_ref), focused_transaction) = &waves.focused_transaction else {
            return;
        };
        let Some(transactions) = waves.transactions_for_source(transaction_ref.source) else {
            return;
        };
        let Some(focused_transaction) = focused_transaction
            .as_ref()
            .or_else(|| transactions.get_transaction(&transaction_ref.inner))
        else {
            return;
        };

        let events_enabled = self.user.config.behavior.ftr_events_enabled();
        let viewport_idx = waves.last_active_viewport_idx;
        egui::SidePanel::right(parent_ui.id().with("Transaction Details"))
            .default_width(330.)
            .width_range(10.0..=max_width)
            .show_inside(parent_ui, |ui| {
                ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);
                self.handle_pointer_in_ui(ui, msgs);
                draw_focused_transaction_details(
                    ui,
                    transactions,
                    transaction_ref.source,
                    focused_transaction,
                    events_enabled,
                    viewport_idx,
                    msgs,
                );
            });
    }
}

impl WaveData {
    pub fn add_stream_or_generator_from_name(
        &mut self,
        scope: Option<StreamScopeRef>,
        name: String,
        fold_events: bool,
    ) -> Option<()> {
        self.add_stream_or_generator_from_name_from_source(
            WaveData::primary_source_id(),
            scope,
            name,
            fold_events,
        )
    }

    pub fn add_stream_or_generator_from_name_from_source(
        &mut self,
        source: SourceId,
        scope: Option<StreamScopeRef>,
        name: String,
        fold_events: bool,
    ) -> Option<()> {
        let inner = self.transactions_for_source(source)?;
        match scope {
            Some(StreamScopeRef::Root) => {
                let (stream_id, name) = inner
                    .get_stream_from_name(name)
                    .map(|s| (s.id, s.name.clone()))?;

                self.add_stream_from_source(
                    source,
                    TransactionStreamRef::new_stream(stream_id, name),
                    fold_events,
                );
            }
            Some(StreamScopeRef::Stream(stream)) => {
                let (stream_id, id, name) = inner
                    .get_generator_from_name_or_qualified(Some(stream.stream_id), &name)
                    .map(|g| (g.stream_id, g.id, g.name.clone()))?;

                self.add_generator_from_source(
                    source,
                    TransactionStreamRef::new_gen(stream_id, id, name),
                );
            }
            Some(StreamScopeRef::Empty(_)) => {}
            None => {
                let (stream_id, id, name) = inner
                    .get_generator_from_name_or_qualified(None, &name)
                    .map(|g| (g.stream_id, g.id, g.name.clone()))?;

                self.add_generator_from_source(
                    source,
                    TransactionStreamRef::new_gen(stream_id, id, name),
                );
            }
        }
        Some(())
    }

    /// Adds every generator of a stream as its own row. With `fold_events`,
    /// conforming `.events` generators are skipped: their events render
    /// overlaid on the parent generator's row instead of as duplicate rows.
    pub fn add_all_from_stream_scope(
        &mut self,
        scope_name: String,
        fold_events: bool,
    ) -> Option<()> {
        self.add_all_from_stream_scope_from_source(
            WaveData::primary_source_id(),
            scope_name,
            fold_events,
        )
    }

    pub fn add_all_from_stream_scope_from_source(
        &mut self,
        source: SourceId,
        scope_name: String,
        fold_events: bool,
    ) -> Option<()> {
        if scope_name == "tr" {
            self.add_all_streams_from_source(source, fold_events);
        } else {
            let stream_id = {
                let inner = self.transactions_for_source(source)?;
                inner.get_stream_from_name(scope_name.clone())?.id
            };
            let needs_load = self
                .transactions_for_source(source)?
                .get_stream(stream_id)
                .is_some_and(|stream| !stream.transactions_loaded);
            if needs_load {
                info!("(Stream {stream_id}) Loading transactions into memory!");
                match self
                    .transactions_for_source_mut(source)?
                    .load_stream(stream_id)
                {
                    Ok(()) => info!("(Stream {stream_id}) Finished loading transactions!"),
                    Err(e) => {
                        warn!("Failed to load transactions for stream {stream_id}: {e:?}");
                        return None;
                    }
                }
            }

            let inner = self.transactions_for_source(source)?;
            let stream = inner.get_stream(stream_id)?;
            let gens = stream
                .generators
                .iter()
                .filter(|gen_id| {
                    !(fold_events && inner.event_index().is_conforming_events_generator(**gen_id))
                })
                .map(|gen_id| inner.get_generator(*gen_id).unwrap())
                .map(|g| (g.stream_id, g.id, g.name.clone()))
                // Sort by name to get deterministic order
                .sorted_by(|a, b| numeric_sort::cmp(&a.2, &b.2))
                .collect_vec();

            for (stream_id, id, name) in gens {
                self.add_generator_from_source(
                    source,
                    TransactionStreamRef::new_gen(stream_id, id, name.clone()),
                );
            }
        }
        Some(())
    }

    pub fn move_to_transaction(&mut self, next: bool) -> Option<()> {
        let mut transactions = self
            .items_tree
            .iter_visible()
            .flat_map(|node| {
                let item = &self.displayed_items[&node.item_ref];
                match item {
                    DisplayedItem::Stream(s) => {
                        let Some(inner) = self.transactions_for_source(s.source) else {
                            return vec![];
                        };
                        let stream_ref = &s.transaction_stream_ref;
                        let stream_id = stream_ref.stream_id;
                        let transaction_refs = if let Some(gen_id) = stream_ref.gen_id {
                            inner.get_transactions_from_generator(gen_id)
                        } else {
                            inner.get_transactions_from_stream(stream_id)
                        };
                        transaction_refs
                            .into_iter()
                            .map(|id| SourceTransactionRef::new(s.source, TransactionRef { id }))
                            .collect()
                    }
                    _ => vec![],
                }
            })
            .collect_vec();
        transactions.sort_unstable_by_key(|tx| (tx.inner.id.0, tx.source.0));
        let tx = if let Some(focused_tx) = &self.focused_transaction.0 {
            let next_id = transactions
                .iter()
                .enumerate()
                .find(|(_, tx)| *tx == focused_tx)
                .map_or(
                    if next { transactions.len() - 1 } else { 0 },
                    |(vec_idx, _)| {
                        if next {
                            if vec_idx + 1 < transactions.len() {
                                vec_idx + 1
                            } else {
                                transactions.len() - 1
                            }
                        } else {
                            vec_idx.saturating_sub(1)
                        }
                    },
                );
            Some(transactions.get(next_id)?.clone())
        } else {
            transactions.first().cloned()
        };
        self.focused_transaction = (tx, self.focused_transaction.1.clone());
        Some(())
    }

    /// Moves focus between FTR events. With a parent transaction focused,
    /// focuses its first (next) or last (prev) event. With an event focused,
    /// moves to the neighboring event of the same parent, wrapping into the
    /// neighboring parents' events at the ends.
    pub fn move_to_event(&mut self, next: bool) -> Option<()> {
        let focused_ref = self.focused_transaction.0.clone()?;
        let inner = self.transactions_for_source(focused_ref.source)?;
        let focused_id = focused_ref.inner.id;
        let index = inner.event_index();

        let target = if let Some(info) = index.event_info(focused_id) {
            let siblings = index.events_of_parent(info.parent_tx);
            let pos = siblings.iter().position(|id| *id == focused_id)?;
            if next {
                siblings
                    .get(pos + 1)
                    .copied()
                    .or_else(|| neighbor_parent_event(inner, info.parent_tx, next))
            } else if pos > 0 {
                siblings.get(pos - 1).copied()
            } else {
                neighbor_parent_event(inner, info.parent_tx, next)
            }
        } else {
            let events = index.events_of_parent(focused_id);
            let own = if next {
                events.first().copied()
            } else {
                events.last().copied()
            };
            own.or_else(|| neighbor_parent_event(inner, focused_id, next))
        }?;

        self.focused_transaction = (
            Some(SourceTransactionRef::new(
                focused_ref.source,
                TransactionRef { id: target },
            )),
            None,
        );
        Some(())
    }
}

/// First (next) or last (prev) event of the nearest neighboring transaction
/// in the same generator as `parent_tx` that has events.
fn neighbor_parent_event(
    transactions: &TransactionContainer,
    parent_tx: ftr_parser::types::TransactionId,
    next: bool,
) -> Option<ftr_parser::types::TransactionId> {
    let index = transactions.event_index();
    let (gen_id, idx) = index.lookup_tx(parent_tx)?;
    let generator = transactions.get_generator(gen_id)?;
    let neighbors: Box<dyn Iterator<Item = usize>> = if next {
        Box::new(idx + 1..generator.transactions.len())
    } else {
        Box::new((0..idx).rev())
    };

    neighbors
        .map(|i| index.events_of_parent(generator.transactions[i].get_tx_id()))
        .find(|events| !events.is_empty())
        .and_then(|events| {
            if next {
                events.first().copied()
            } else {
                events.last().copied()
            }
        })
}

fn draw_focused_transaction_details(
    ui: &mut Ui,
    transactions: &TransactionContainer,
    source: SourceId,
    focused_transaction: &Transaction,
    events_enabled: bool,
    viewport_idx: usize,
    msgs: &mut Vec<Message>,
) {
    let is_event = events_enabled
        && transactions
            .event_index()
            .is_events_generator(focused_transaction.get_gen_id());
    if is_event {
        draw_focused_event_details(
            ui,
            transactions,
            source,
            focused_transaction,
            viewport_idx,
            msgs,
        );
    } else {
        draw_focused_parent_details(
            ui,
            transactions,
            source,
            focused_transaction,
            events_enabled,
            msgs,
        );
    }
}

/// Details card for a focused FTR event: the event name leads, followed by
/// its own attributes and a summary of the parent transaction.
fn draw_focused_event_details(
    ui: &mut Ui,
    transactions: &TransactionContainer,
    source: SourceId,
    focused_transaction: &Transaction,
    viewport_idx: usize,
    msgs: &mut Vec<Message>,
) {
    let index = transactions.event_index();
    let tx_id = focused_transaction.get_tx_id();
    let info = index.event_info(tx_id);
    let name = crate::transaction_events::event_name(focused_transaction)
        .unwrap_or_else(|| format!("tx#{tx_id}"));

    ui.with_layout(
        Layout::top_down(Align::LEFT).with_cross_justify(true),
        |ui| {
            ui.label(FOCUSED_EVENT_DETAILS_HDR);
            ui.heading(format!("{EVENT_LABEL}: {name}"));
            match info {
                Some(info) if info.out_of_range => {
                    ui.label(RichText::new(OUT_OF_RANGE_NOTICE).color(ui.visuals().warn_fg_color));
                }
                Some(info) if info.multiple_parents => {
                    ui.label(
                        RichText::new(MULTIPLE_PARENTS_NOTICE).color(ui.visuals().warn_fg_color),
                    );
                }
                None => {
                    ui.label(RichText::new(ORPHAN_NOTICE).color(ui.visuals().warn_fg_color));
                }
                _ => {}
            }

            let column_width = ui.available_width() / 2.;
            TableBuilder::new(ui)
                .column(Column::exact(column_width))
                .column(Column::auto())
                .body(|mut body| {
                    let start_time = focused_transaction.get_start_time();
                    let end_time = focused_transaction.get_end_time();
                    table_row(&mut body, TX_ID_LABEL, &tx_id.to_string());
                    table_row(&mut body, EVENT_TIME_LABEL, &start_time.to_string());
                    if end_time != start_time {
                        table_row(
                            &mut body,
                            EVENT_DURATION_LABEL,
                            &(end_time - start_time).to_string(),
                        );
                    }

                    // Attributes other than the promoted name
                    let attributes = focused_transaction
                        .attributes
                        .iter()
                        .filter(|attr| {
                            attr.name.as_ref() != crate::transaction_events::EVENT_NAME_ATTRIBUTE
                        })
                        .collect_vec();
                    if !attributes.is_empty() {
                        section_header(&mut body, ATTRIBUTES_SECTION_TITLE);
                        subheader(&mut body, ATTR_NAME_LABEL, ATTR_VALUE_LABEL);
                        for attr in attributes {
                            table_row(&mut body, &attr.name, &attr.value().clone());
                        }
                    }
                });

            if let Some(info) = info
                && let Some(parent) =
                    transactions.get_transaction(&TransactionRef { id: info.parent_tx })
            {
                let parent_gen_name = transactions
                    .get_generator(info.parent_gen)
                    .map_or_else(|| "unknown".to_string(), |g| g.name.clone());
                ui.add_space(SECTION_GAP);
                ui.heading(PARENT_SECTION_TITLE);
                ui.horizontal(|ui| {
                    ui.label(format!("{parent_gen_name} tx#{}", info.parent_tx));
                    if ui.button(GO_TO_PARENT_LABEL).clicked() {
                        let parent_ref = TransactionRef { id: info.parent_tx };
                        msgs.push(Message::FocusTransactionFromSource(
                            Some(SourceTransactionRef::new(source, parent_ref)),
                            None,
                        ));
                        // Bring the parent into view
                        let mid = (parent.get_start_time() + parent.get_end_time()) / 2;
                        msgs.push(Message::GoToTime(
                            Some(num::BigInt::from(mid)),
                            viewport_idx,
                        ));
                    }
                });
                let column_width = ui.available_width() / 2.;
                TableBuilder::new(ui)
                    .id_salt("event parent attributes")
                    .column(Column::exact(column_width))
                    .column(Column::auto())
                    .body(|mut body| {
                        for attr in crate::transaction_events::begin_attributes(parent) {
                            table_row(&mut body, &attr.name, &attr.value().clone());
                        }
                    });
            }
        },
    );
}

/// Details card for a focused (non-event) transaction: properties,
/// attributes, its events, and relations.
fn draw_focused_parent_details(
    ui: &mut Ui,
    transactions: &TransactionContainer,
    source: SourceId,
    focused_transaction: &Transaction,
    events_enabled: bool,
    msgs: &mut Vec<Message>,
) {
    let tx_id = focused_transaction.get_tx_id();
    let index = transactions.event_index();
    let events = if events_enabled {
        transactions.events_of_parent(tx_id)
    } else {
        &[]
    };

    ui.with_layout(
        Layout::top_down(Align::LEFT).with_cross_justify(true),
        |ui| {
            ui.label(FOCUSED_TX_DETAILS_HDR);
            let column_width = ui.available_width() * 0.5;
            TableBuilder::new(ui)
                .column(Column::exact(column_width))
                .column(Column::auto())
                .header(20.0, |mut header| {
                    header.col(|ui| {
                        ui.heading(PROPERTIES_HDR);
                    });
                })
                .body(|mut body| {
                    table_row(&mut body, TX_ID_LABEL, &tx_id.to_string());
                    table_row(&mut body, TX_TYPE_LABEL, {
                        let generator = transactions
                            .get_generator(focused_transaction.get_gen_id())
                            .unwrap();
                        &generator.name
                    });
                    table_row(
                        &mut body,
                        START_TIME_LABEL,
                        &focused_transaction.get_start_time().to_string(),
                    );
                    table_row(
                        &mut body,
                        END_TIME_LABEL,
                        &focused_transaction.get_end_time().to_string(),
                    );
                    section_header(&mut body, ATTRIBUTES_SECTION_TITLE);
                    subheader(&mut body, ATTR_NAME_LABEL, ATTR_VALUE_LABEL);

                    for attr in &focused_transaction.attributes {
                        table_row(&mut body, &attr.name, &attr.value().clone());
                    }

                    // parent_of relations to this transaction's events are
                    // represented by the Events section below instead of the
                    // generic relation tables
                    let inc_relations = focused_transaction
                        .inc_relations
                        .iter()
                        .filter_map(|idx| transactions.get_relation(*idx))
                        .filter(|rel| !is_event_link(transactions, events_enabled, rel))
                        .collect_vec();
                    if !inc_relations.is_empty() {
                        section_header(&mut body, INCOMING_RELATIONS_TITLE);
                        subheader(&mut body, RELATION_NAME_LABEL, RELATION_LINK_LABEL);

                        for rel in inc_relations {
                            table_row(
                                &mut body,
                                &rel.name,
                                &format!("#{} -> #{}", rel.source_tx_id, rel.sink_tx_id),
                            );
                        }
                    }

                    let out_relations = focused_transaction
                        .out_relations
                        .iter()
                        .filter_map(|idx| transactions.get_relation(*idx))
                        .filter(|rel| !is_event_link(transactions, events_enabled, rel))
                        .collect_vec();
                    if !out_relations.is_empty() {
                        section_header(&mut body, OUTGOING_RELATIONS_TITLE);
                        subheader(&mut body, RELATION_NAME_LABEL, RELATION_LINK_LABEL);

                        for rel in out_relations {
                            table_row(
                                &mut body,
                                &rel.name,
                                &format!("#{} -> #{}", rel.source_tx_id, rel.sink_tx_id),
                            );
                        }
                    }
                });

            if !events.is_empty() {
                ui.add_space(SECTION_GAP);
                ui.horizontal(|ui| {
                    ui.heading(format!("{EVENTS_SECTION_TITLE} ({})", events.len()));
                    if ui.button(OPEN_EVENTS_IN_TABLE_LABEL).clicked() {
                        let gen_id = focused_transaction.get_gen_id();
                        if let Some(generator) = transactions.get_generator(gen_id) {
                            msgs.push(Message::OpenEventTable {
                                source,
                                generator: TransactionStreamRef::new_gen(
                                    generator.stream_id,
                                    gen_id,
                                    generator.name.clone(),
                                ),
                            });
                        }
                    }
                });
                for event_id in events {
                    let Some(event) =
                        transactions.get_transaction(&TransactionRef { id: *event_id })
                    else {
                        continue;
                    };
                    let name = crate::transaction_events::event_name(event)
                        .unwrap_or_else(|| format!("tx#{event_id}"));
                    let out_of_range = index
                        .event_info(*event_id)
                        .is_some_and(|info| info.out_of_range);
                    let summary = event
                        .attributes
                        .iter()
                        .filter(|attr| {
                            attr.name.as_ref() != crate::transaction_events::EVENT_NAME_ATTRIBUTE
                        })
                        .map(|attr| format!("{}={}", attr.name, attr.value()))
                        .join(", ");
                    let mut label = format!("{}  {name}", event.get_start_time());
                    if out_of_range {
                        label.push_str("  ⚠");
                    }
                    if !summary.is_empty() {
                        label.push_str("  ");
                        label.push_str(&summary);
                    }
                    let response = ui.selectable_label(false, label);
                    if response.double_clicked() {
                        // Double click also moves the cursor to the event
                        msgs.push(Message::CursorSet(num::BigInt::from(
                            event.get_start_time(),
                        )));
                        msgs.push(Message::FocusTransactionFromSource(
                            Some(SourceTransactionRef::new(
                                source,
                                TransactionRef { id: *event_id },
                            )),
                            None,
                        ));
                    } else if response.clicked() {
                        msgs.push(Message::FocusTransactionFromSource(
                            Some(SourceTransactionRef::new(
                                source,
                                TransactionRef { id: *event_id },
                            )),
                            None,
                        ));
                    }
                }
            }
        },
    );
}

/// Whether a relation is the `parent_of` link of a conforming event (shown
/// through the event UI rather than the generic relation tables).
fn is_event_link(
    transactions: &TransactionContainer,
    events_enabled: bool,
    rel: &ftr_parser::types::TxRelation,
) -> bool {
    events_enabled
        && rel.name.as_ref() == crate::transaction_events::EVENT_PARENT_RELATION
        && transactions
            .event_info(rel.sink_tx_id)
            .is_some_and(|info| info.parent_tx == rel.source_tx_id)
}

pub fn calculate_rows_of_stream(
    transactions: &[Transaction],
    last_times_on_row: &mut Vec<(u64, u64)>,
) {
    for transaction in transactions {
        let mut curr_row = 0;
        let start_time = transaction.get_start_time();
        let end_time = transaction.get_end_time();

        while last_times_on_row[curr_row].1 > start_time {
            curr_row += 1;
            if last_times_on_row.len() <= curr_row {
                last_times_on_row.push((0, 0));
            }
        }
        last_times_on_row[curr_row] = (start_time, end_time);
    }
}

/// Presentation options for the transaction hierarchy sidebar.
pub struct TransactionListOptions {
    /// FTR event convention support is enabled
    pub events_enabled: bool,
    /// List raw `.events` generators instead of folding them into their
    /// parent generator's presentation
    pub show_raw_event_generators: bool,
}

pub fn draw_transaction_variable_list(
    msgs: &mut Vec<Message>,
    streams: &WaveData,
    ui: &mut Ui,
    active_stream: &StreamScopeRef,
    options: &TransactionListOptions,
) {
    draw_transaction_variable_list_for_source(
        msgs,
        streams,
        ui,
        WaveData::primary_source_id(),
        active_stream,
        options,
    );
}

pub fn draw_transaction_variable_list_for_source(
    msgs: &mut Vec<Message>,
    streams: &WaveData,
    ui: &mut Ui,
    source: SourceId,
    active_stream: &StreamScopeRef,
    options: &TransactionListOptions,
) {
    let Some(inner) = streams.transactions_for_source(source) else {
        return;
    };
    match active_stream {
        StreamScopeRef::Root => {
            draw_transaction_root_variables(msgs, ui, inner, source);
        }
        StreamScopeRef::Stream(stream_ref) => {
            draw_transaction_stream_variables(msgs, ui, inner, source, stream_ref, options);
        }
        StreamScopeRef::Empty(_) => {}
    }
}

pub fn draw_transaction_root(msgs: &mut Vec<Message>, streams: &WaveData, ui: &mut Ui) {
    draw_transaction_root_for_source(msgs, streams, WaveData::primary_source_id(), ui);
}

pub fn draw_transaction_root_for_source(
    msgs: &mut Vec<Message>,
    streams: &WaveData,
    source: SourceId,
    ui: &mut Ui,
) {
    egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        egui::Id::new(("Streams", source)),
        false,
    )
    .show_header(ui, |ui| {
        ui.with_layout(
            Layout::top_down(Align::LEFT).with_cross_justify(true),
            |ui| {
                let response = ui.selectable_label(
                    streams.active_scope_source == source
                        && streams.active_scope
                            == Some(ScopeType::StreamScope(StreamScopeRef::Root)),
                    TRANSACTION_ROOT_NAME,
                );
                if response.clicked() {
                    msgs.push(Message::SetActiveScopeFromSource(
                        source,
                        Some(ScopeType::StreamScope(StreamScopeRef::Root)),
                    ));
                }
            },
        );
    })
    .body(|ui| {
        if let Some(tx_container) = streams.transactions_for_source(source) {
            for (id, stream) in &tx_container.inner.tx_streams {
                let selected = streams.active_scope_source == source
                    && streams.active_scope.as_ref().is_some_and(|s| {
                        if let ScopeType::StreamScope(StreamScopeRef::Stream(scope_stream)) = s {
                            scope_stream.stream_id == *id
                        } else {
                            false
                        }
                    });
                let response = ui.selectable_label(selected, &stream.name);
                if response.clicked() {
                    msgs.push(Message::SetActiveScopeFromSource(
                        source,
                        Some(ScopeType::StreamScope(StreamScopeRef::Stream(
                            TransactionStreamRef::new_stream(*id, stream.name.clone()),
                        ))),
                    ));
                }
            }
        }
    });
}

fn draw_transaction_stream_variables(
    msgs: &mut Vec<Message>,
    ui: &mut Ui,
    inner: &TransactionContainer,
    source: SourceId,
    stream_ref: &TransactionStreamRef,
    options: &TransactionListOptions,
) {
    if let Some(stream) = inner.get_stream(stream_ref.stream_id) {
        let index = inner.event_index();
        let stream_loaded = stream.transactions_loaded;
        let stream_has_event_generators = options.events_enabled
            && stream
                .generators
                .iter()
                .any(|gen_id| is_folded_events_generator(index, *gen_id, stream_loaded));

        if stream_has_event_generators {
            let mut show_raw = options.show_raw_event_generators;
            if ui
                .checkbox(&mut show_raw, SHOW_RAW_EVENT_GENERATORS_LABEL)
                .changed()
            {
                msgs.push(Message::SetShowRawEventGenerators(show_raw));
            }
        }

        let sorted_generators = stream
            .generators
            .iter()
            .filter_map(|gen_id| {
                if let Some(g) = inner.get_generator(*gen_id) {
                    Some((*gen_id, g))
                } else {
                    tracing::warn!(
                        "Generator ID {} not found in stream {}",
                        gen_id,
                        stream_ref.stream_id
                    );
                    None
                }
            })
            .sorted_by(|(_, a), (_, b)| numeric_sort::cmp(&a.name, &b.name));

        for (gen_id, generator) in sorted_generators {
            let is_events_generator =
                options.events_enabled && is_folded_events_generator(index, gen_id, stream_loaded);
            // Events generators fold into their parent's presentation unless
            // the user asked for raw access
            if is_events_generator && !options.show_raw_event_generators {
                continue;
            }
            let events_generator = options
                .events_enabled
                .then(|| events_generator_for_parent(index, gen_id, stream_loaded))
                .flatten();

            ui.with_layout(
                Layout::top_down(Align::LEFT).with_cross_justify(true),
                |ui| {
                    let label = if is_events_generator {
                        RichText::new(format!("{EVENT_BADGE} {}", generator.name)).weak()
                    } else if let Some(events_gen) = events_generator {
                        RichText::new(format!(
                            "{} {}",
                            generator.name,
                            event_badge_text(index.conforming_event_count(events_gen))
                        ))
                    } else {
                        RichText::new(&generator.name)
                    };
                    let response = ui.selectable_label(false, label);
                    if response.clicked() {
                        msgs.push(Message::AddStreamOrGeneratorFromSource(
                            source,
                            TransactionStreamRef::new_gen(
                                stream_ref.stream_id,
                                gen_id,
                                generator.name.clone(),
                            ),
                        ));
                    }
                    // Context menu for generator
                    response.context_menu(|ui| {
                        let gen_ref = TransactionStreamRef::new_gen(
                            stream_ref.stream_id,
                            gen_id,
                            generator.name.clone(),
                        );
                        if !is_events_generator
                            && index.events_generator_of(gen_id).is_some()
                            && ui.button("Open in Konata view").clicked()
                        {
                            msgs.push(Message::OpenKonataView {
                                source,
                                generator: gen_ref.clone(),
                            });
                            ui.close();
                        }
                        if ui.button("Show transactions in table").clicked() {
                            msgs.push(Message::OpenTransactionTable {
                                source,
                                generator: gen_ref.clone(),
                            });
                            ui.close();
                        }
                        if let Some(events_gen) = events_generator {
                            if ui.button("Show events in table").clicked() {
                                msgs.push(Message::OpenEventTable {
                                    source,
                                    generator: gen_ref.clone(),
                                });
                                ui.close();
                            }
                            if ui.button("Add events as separate row").clicked() {
                                if let Some(events_generator) = inner.get_generator(events_gen) {
                                    msgs.push(Message::AddStreamOrGeneratorFromSource(
                                        source,
                                        TransactionStreamRef::new_gen(
                                            stream_ref.stream_id,
                                            events_gen,
                                            events_generator.name.clone(),
                                        ),
                                    ));
                                }
                                ui.close();
                            }
                        }
                        if ui.button("Add to waveform view").clicked() {
                            msgs.push(Message::AddStreamOrGeneratorFromSource(source, gen_ref));
                            ui.close();
                        }
                    });
                },
            );
        }
    } else {
        ui.label(STREAM_NOT_FOUND_LABEL);
        tracing::warn!(
            "Stream ID {} not found in transaction container",
            stream_ref.stream_id
        );
    }
}

fn is_folded_events_generator(
    index: &crate::transaction_events::EventIndex,
    gen_id: ftr_parser::types::GeneratorId,
    stream_loaded: bool,
) -> bool {
    if stream_loaded {
        index.is_conforming_events_generator(gen_id)
    } else {
        index.is_events_generator(gen_id)
    }
}

fn events_generator_for_parent(
    index: &crate::transaction_events::EventIndex,
    gen_id: ftr_parser::types::GeneratorId,
    stream_loaded: bool,
) -> Option<ftr_parser::types::GeneratorId> {
    if stream_loaded {
        index.conforming_events_generator_of(gen_id)
    } else {
        index.events_generator_of(gen_id)
    }
}

/// Sidebar badge for a parent generator with events, e.g. "⚡ 3421 events".
/// Falls back to a bare badge before the stream's transactions are loaded.
fn event_badge_text(count: usize) -> String {
    if count == 0 {
        EVENT_BADGE.to_string()
    } else {
        format!("{EVENT_BADGE} {count} events")
    }
}

fn draw_transaction_root_variables(
    msgs: &mut Vec<Message>,
    ui: &mut Ui,
    inner: &TransactionContainer,
    source: SourceId,
) {
    let streams = inner.get_streams();
    let sorted_streams = streams
        .iter()
        .sorted_by(|a, b| numeric_sort::cmp(&a.name, &b.name));
    for stream in sorted_streams {
        ui.with_layout(
            Layout::top_down(Align::LEFT).with_cross_justify(true),
            |ui| {
                let response = ui.selectable_label(false, &stream.name);
                if response.clicked() {
                    msgs.push(Message::AddStreamOrGeneratorFromSource(
                        source,
                        TransactionStreamRef::new_stream(stream.id, stream.name.clone()),
                    ));
                }
                response.context_menu(|ui| {
                    if ui.button("Add to waveform view").clicked() {
                        msgs.push(Message::AddStreamOrGeneratorFromSource(
                            source,
                            TransactionStreamRef::new_stream(stream.id, stream.name.clone()),
                        ));
                        ui.close();
                    }
                    let stream_ref =
                        TransactionStreamRef::new_stream(stream.id, stream.name.clone());
                    let generators =
                        inner.generators_in_stream(&StreamScopeRef::Stream(stream_ref));
                    let pipelines = generators
                        .iter()
                        .filter(|generator| {
                            generator.gen_id.is_some_and(|id| {
                                inner.event_index().events_generator_of(id).is_some()
                            })
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    if !pipelines.is_empty() {
                        ui.menu_button("Open in Konata view", |ui| {
                            for generator in &pipelines {
                                if ui.button(&generator.name).clicked() {
                                    msgs.push(Message::OpenKonataView {
                                        source,
                                        generator: generator.clone(),
                                    });
                                    ui.close();
                                }
                            }
                        });
                    }
                    if !generators.is_empty() && ui.button("Show transactions in table").clicked() {
                        for gen_ref in generators {
                            msgs.push(Message::OpenTransactionTable {
                                source,
                                generator: gen_ref,
                            });
                        }
                        ui.close();
                    }
                });
            },
        );
    }
}

// Helper functions for drawing transaction details table

fn table_row(body: &mut TableBody, key: &str, val: &str) {
    body.row(ROW_HEIGHT, |mut row| {
        row.col(|ui| {
            ui.label(key);
        });
        row.col(|ui| {
            ui.label(val);
        });
    });
}

fn section_header(body: &mut TableBody, title: &str) {
    body.row(ROW_HEIGHT + SECTION_GAP, |mut row| {
        row.col(|ui| {
            ui.heading(title);
        });
    });
}

fn subheader(body: &mut TableBody, left: &str, right: &str) {
    body.row(ROW_HEIGHT + SUBHEADER_GAP, |mut row| {
        row.col(|ui| {
            ui.label(RichText::new(left).size(SUBHEADER_SIZE));
        });
        row.col(|ui| {
            ui.label(RichText::new(right).size(SUBHEADER_SIZE));
        });
    });
}
