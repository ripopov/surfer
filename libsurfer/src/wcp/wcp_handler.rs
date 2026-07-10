use crate::{
    SystemState, WcpClientCapabilities,
    displayed_item::{DisplayedItem, DisplayedItemRef},
    message::{Message, MessageTarget},
    source::SourceTransactionRef,
    transaction_container::{TransactionRef, TransactionStreamRef},
    wave_container::{ScopeRefExt, VariableRef, VariableRefExt},
    wave_data::WaveData,
    wave_source::{LoadOptions, WaveSource, string_to_wavesource},
};

use ftr_parser::types::TransactionId;
use futures::executor::block_on;
use itertools::Itertools;
use std::sync::atomic::Ordering;
use surfer_translation_types::ScopeRef;
use tracing::{trace, warn};

use surfer_wcp::{ItemInfo, MarkerInfo, WcpCSMessage, WcpCommand, WcpResponse, WcpSCMessage};

impl SystemState {
    pub fn handle_wcp_commands(&mut self) {
        let Some(receiver) = &mut self.channels.wcp_c2s_receiver else {
            return;
        };

        let mut messages = vec![];
        loop {
            match receiver.try_recv() {
                Ok(command) => {
                    messages.push(command);
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    trace!("WCP Command sender disconnected");
                    break;
                }
            }
        }
        for message in messages {
            self.handle_wcp_cs_message(&message);
        }
    }

    fn handle_wcp_cs_message(&mut self, message: &WcpCSMessage) {
        if !self.wcp_greeted_signal.load(Ordering::Relaxed) {
            if let WcpCSMessage::greeting { .. } = message {
            } else {
                self.send_error("WCP server has not received greeting messages", vec![], "");
                return;
            }
        }
        match message {
            WcpCSMessage::command(command) => {
                match command {
                    WcpCommand::get_item_list => {
                        if let Some(waves) = &self.user.waves {
                            let ids: Vec<surfer_wcp::DisplayedItemRef> = self
                                .get_displayed_items(waves)
                                .iter()
                                .map(std::convert::Into::into)
                                .collect_vec();
                            self.send_response(WcpResponse::get_item_list { ids });
                        } else {
                            self.send_error("No waveform loaded", vec![], "No waveform loaded");
                        }
                    }
                    WcpCommand::get_item_info { ids } => {
                        let Some(waves) = &self.user.waves else {
                            self.send_error("remove_items", vec![], "No waveform loaded");
                            return;
                        };
                        let mut items: Vec<ItemInfo> = Vec::new();
                        for id in ids {
                            if let Some(item) = waves.displayed_items.get(&id.into()) {
                                let (name, item_type) = match item {
                                    DisplayedItem::Variable(var) => (
                                        var.manual_name.clone().unwrap_or(var.display_name.clone()),
                                        "Variable".to_string(),
                                    ),
                                    DisplayedItem::Divider(item) => (
                                        item.name.clone().unwrap_or("Name not found!".to_string()),
                                        "Divider".to_string(),
                                    ),
                                    DisplayedItem::Marker(item) => (
                                        item.name.clone().unwrap_or("Name not found!".to_string()),
                                        "Marker".to_string(),
                                    ),
                                    DisplayedItem::TimeLine(item) => (
                                        item.name.clone().unwrap_or("Name not found!".to_string()),
                                        "TimeLine".to_string(),
                                    ),
                                    DisplayedItem::Placeholder(item) => (
                                        item.manual_name
                                            .clone()
                                            .unwrap_or("Name not found!".to_string()),
                                        "Placeholder".to_string(),
                                    ),
                                    DisplayedItem::Stream(item) => (
                                        item.manual_name
                                            .clone()
                                            .unwrap_or(item.display_name.clone()),
                                        "Stream".to_string(),
                                    ),
                                    DisplayedItem::Group(item) => {
                                        (item.name.clone(), "Group".to_string())
                                    }
                                };
                                items.push(ItemInfo {
                                    name,
                                    t: item_type,
                                    id: *id,
                                });
                            } else {
                                self.send_error(
                                    "get_item_info",
                                    vec![],
                                    &format!("No item with id {id:?}"),
                                );
                                return;
                            }
                        }
                        self.send_response(WcpResponse::get_item_info { results: items });
                    }
                    WcpCommand::add_variables { variables } => {
                        if self.user.waves.is_some() {
                            self.save_current_canvas(format!("Add {} variables", variables.len()));
                        }
                        if let Some(waves) = self.user.waves.as_mut() {
                            let variable_refs = variables
                                .iter()
                                .map(|n| VariableRef::from_hierarchy_string(n))
                                .collect_vec();
                            let (cmd, ids) = waves.add_variables(
                                &self.translators,
                                variable_refs,
                                None,
                                true,
                                false,
                                None,
                            );
                            if let Some(cmd) = cmd {
                                self.load_variables(cmd);
                            }
                            self.send_response(WcpResponse::add_variables {
                                ids: ids.into_iter().map(std::convert::Into::into).collect_vec(),
                            });
                            self.invalidate_draw_commands();
                        } else {
                            self.send_error(
                                "add_variables",
                                vec![],
                                "Can't add signals. No waveform loaded",
                            );
                        }
                    }
                    WcpCommand::add_scope { scope, recursive } => {
                        if self.user.waves.is_some() {
                            self.save_current_canvas(format!("Add scope {scope}"));
                        }
                        let scope = ScopeRef::from_hierarchy_string(scope);
                        let variables = self.get_scope(&scope, *recursive);
                        if let Some(waves) = self.user.waves.as_mut() {
                            let (cmd, ids) = waves.add_variables(
                                &self.translators,
                                variables,
                                None,
                                true,
                                false,
                                None,
                            );
                            if let Some(cmd) = cmd {
                                self.load_variables(cmd);
                            }
                            self.send_response(WcpResponse::add_scope {
                                ids: ids.into_iter().map(std::convert::Into::into).collect_vec(),
                            });
                            self.invalidate_draw_commands();
                        } else {
                            self.send_error("scope_add", vec![], "No waveform loaded");
                        }
                    }
                    WcpCommand::add_items { items, recursive } => {
                        if self.user.waves.is_some() {
                            self.save_current_canvas(format!("Add {} items", items.len()));
                        }

                        let mut variables: Vec<VariableRef> = Vec::new();
                        for item in items {
                            let variable_ref = VariableRef::from_hierarchy_string(item);
                            let scope = ScopeRef::from_hierarchy_string(item);
                            let scope_variables = self.get_scope(&scope, *recursive);
                            variables.push(variable_ref);
                            variables.extend(scope_variables);
                        }

                        if let Some(waves) = self.user.waves.as_mut() {
                            let (cmd, ids) = waves.add_variables(
                                &self.translators,
                                variables,
                                None,
                                true,
                                true,
                                None,
                            );
                            if let Some(cmd) = cmd {
                                self.load_variables(cmd);
                            }
                            self.send_response(WcpResponse::add_items {
                                ids: ids.into_iter().map(std::convert::Into::into).collect_vec(),
                            });
                            self.invalidate_draw_commands();
                        } else {
                            self.send_error(
                                "add_items",
                                vec![],
                                "Can't add items. No waveform loaded",
                            );
                        }
                    }
                    WcpCommand::add_markers { markers } => {
                        if self.user.waves.is_some() {
                            self.save_current_canvas(format!("Add {} markers", markers.len()));
                        }
                        if let Some(waves) = self.user.waves.as_mut() {
                            let mut ids = vec![];
                            for marker in markers {
                                let MarkerInfo {
                                    time,
                                    name,
                                    move_focus,
                                } = marker;
                                if let Some(id) = waves.add_marker(time, name.clone(), *move_focus)
                                {
                                    ids.push(id.into());
                                } else {
                                    self.send_error("add_markers", vec![], "Cannot add marker");
                                    return;
                                }
                            }
                            self.send_response(WcpResponse::add_markers { ids });
                        } else {
                            self.send_error("add_markers", vec![], "No waveform loaded");
                        }
                    }
                    WcpCommand::reload => {
                        self.update(Message::ReloadWaveform(false));
                        self.send_response(WcpResponse::ack);
                    }
                    WcpCommand::set_viewport_to { timestamp } => {
                        self.update(Message::GoToTime(Some(timestamp.clone()), 0));
                        self.send_response(WcpResponse::ack);
                    }
                    WcpCommand::set_viewport_range { start, end } => {
                        self.update(Message::ZoomToRange {
                            start: start.clone(),
                            end: end.clone(),
                            viewport_idx: 0,
                        });
                        self.send_response(WcpResponse::ack);
                    }
                    WcpCommand::set_item_color { id, color } => {
                        let Some(waves) = &self.user.waves else {
                            self.send_error("set_item_color", vec![], "No waveform loaded");
                            return;
                        };

                        if let Some(idx) = waves.get_displayed_item_index(&id.into()) {
                            self.update(Message::ItemColorChange(
                                MessageTarget::Explicit(idx),
                                Some(color.clone()),
                            ));
                            self.send_response(WcpResponse::ack);
                        } else {
                            self.send_error(
                                "set_item_color",
                                vec![],
                                format!("Item {id:?} not found").as_str(),
                            );
                        }
                    }
                    WcpCommand::remove_items { ids } => {
                        let Some(_) = self.user.waves.as_mut() else {
                            self.send_error("remove_items", vec![], "No waveform loaded");
                            return;
                        };
                        let msgs = vec![Message::RemoveItems(
                            ids.iter().map(std::convert::Into::into).collect(),
                        )];
                        self.update(Message::Batch(msgs));

                        self.send_response(WcpResponse::ack);
                    }
                    WcpCommand::focus_item { id } => {
                        let Some(waves) = &self.user.waves else {
                            self.send_error("remove_items", vec![], "No waveform loaded");
                            return;
                        };
                        // TODO: Create a `.into` function here instead of unwrapping and wrapping
                        // it to prevent future type errors
                        if let Some(vidx) = waves.get_displayed_item_index(&id.into()) {
                            self.update(Message::FocusItem(vidx));

                            self.send_response(WcpResponse::ack);
                        } else {
                            self.send_error(
                                "focus_item",
                                vec![],
                                format!("No item with ID {id:?}").as_str(),
                            );
                        }
                    }
                    WcpCommand::clear => {
                        if let Some(wave) = &self.user.waves {
                            self.update(Message::RemoveItems(self.get_displayed_items(wave)));
                        }

                        self.send_response(WcpResponse::ack);
                    }
                    WcpCommand::load { source } => {
                        match string_to_wavesource(source) {
                            WaveSource::Url(url) => {
                                self.update(Message::LoadWaveformFileFromUrl(
                                    url,
                                    LoadOptions::Clear,
                                ));
                                self.send_response(WcpResponse::ack);
                            }
                            WaveSource::File(file) => {
                                // FIXME add support for loading transaction files via Message::LoadTransactionFile
                                let msg = Message::LoadFile(file, LoadOptions::Clear);
                                self.update(msg);
                                self.send_response(WcpResponse::ack);
                            }
                            _ => {
                                self.send_error(
                                    "load",
                                    vec![],
                                    format!("{source} is not legal wave source").as_str(),
                                );
                            }
                        }
                    }
                    WcpCommand::zoom_to_fit { viewport_idx } => {
                        self.update(Message::ZoomToFit {
                            viewport_idx: *viewport_idx,
                        });
                        self.send_response(WcpResponse::ack);
                    }
                    WcpCommand::set_cursor { timestamp } => {
                        self.update(Message::CursorSet(timestamp.to_owned()));
                        self.send_response(WcpResponse::ack);
                    }
                    WcpCommand::konata_open { generator, source } => {
                        let candidates = self.user.waves.as_ref().map_or_else(Vec::new, |waves| {
                            waves
                                .source_ids()
                                .into_iter()
                                .filter(|source_id| {
                                    source.as_ref().is_none_or(|wanted| {
                                        wanted == &format!("s{}", source_id.0)
                                            || waves.source_label_for(*source_id).as_ref()
                                                == Some(wanted)
                                    })
                                })
                                .flat_map(|source_id| {
                                    waves
                                        .transactions_for_source(source_id)
                                        .into_iter()
                                        .flat_map(move |transactions| {
                                            transactions
                                                .get_generators()
                                                .into_iter()
                                                .filter(move |candidate| {
                                                    candidate.name == *generator
                                                        && transactions
                                                            .event_index()
                                                            .events_generator_of(candidate.id)
                                                            .is_some()
                                                })
                                                .map(move |candidate| {
                                                    (
                                                        source_id,
                                                        TransactionStreamRef::new_gen(
                                                            candidate.stream_id,
                                                            candidate.id,
                                                            candidate.name.clone(),
                                                        ),
                                                    )
                                                })
                                        })
                                })
                                .collect_vec()
                        });
                        if let [candidate] = candidates.as_slice() {
                            self.update(Message::OpenKonataView {
                                source: candidate.0,
                                generator: candidate.1.clone(),
                            });
                            self.send_response(WcpResponse::ack);
                        } else {
                            self.send_error(
                                "konata_open",
                                vec![generator.clone()],
                                if candidates.is_empty() {
                                    "No matching pipeline generator"
                                } else {
                                    "Generator is ambiguous; specify source"
                                },
                            );
                        }
                    }
                    WcpCommand::konata_goto_row { row } => {
                        let valid = self
                            .active_konata_tile()
                            .and_then(|tile| self.active_konata_model(tile))
                            .is_some_and(|model| {
                                usize::try_from(*row).is_ok_and(|row| row < model.row_count())
                            });
                        if valid {
                            self.update(Message::KonataGotoRow(*row));
                            self.send_response(WcpResponse::ack);
                        } else {
                            self.send_error(
                                "konata_goto_row",
                                vec![row.to_string()],
                                "No active ready Konata tile or row is out of range",
                            );
                        }
                    }
                    WcpCommand::konata_goto_rid { rid, thread } => {
                        let row = self
                            .active_konata_tile()
                            .and_then(|tile| self.active_konata_model(tile))
                            .and_then(|model| {
                                thread.as_ref().map_or_else(
                                    || model.row_for_rid(*rid),
                                    |thread| model.row_for_thread_rid(Some(thread), *rid),
                                )
                            });
                        if row.is_some() {
                            self.update(thread.as_ref().map_or(
                                Message::KonataGotoRid(*rid),
                                |thread| Message::KonataGotoThreadRid {
                                    thread: thread.clone(),
                                    rid: *rid,
                                },
                            ));
                            self.send_response(WcpResponse::ack);
                        } else {
                            self.send_error(
                                "konata_goto_rid",
                                vec![rid.to_string()],
                                "Retire ID is missing or ambiguous for the requested thread",
                            );
                        }
                    }
                    WcpCommand::konata_goto_cycle { cycle } => {
                        let has_clock = self
                            .active_konata_tile()
                            .and_then(|tile| self.user.konata_tiles.get(&tile))
                            .is_some_and(|tile| {
                                tile.config
                                    .clock_period_ticks
                                    .is_some_and(|period| period > 0)
                            });
                        if has_clock {
                            self.update(Message::KonataGotoCycle(*cycle));
                            self.send_response(WcpResponse::ack);
                        } else {
                            self.send_error(
                                "konata_goto_cycle",
                                vec![cycle.to_string()],
                                "No active Konata tile with a configured pipeline clock",
                            );
                        }
                    }
                    WcpCommand::konata_set_bookmark { slot }
                        if *slot < 10 && self.active_konata_tile().is_some() =>
                    {
                        self.update(Message::KonataBookmarkSet(*slot));
                        self.send_response(WcpResponse::ack);
                    }
                    WcpCommand::konata_set_bookmark { slot } if *slot < 10 => {
                        self.send_error(
                            "konata_set_bookmark",
                            vec![slot.to_string()],
                            "No active Konata tile",
                        );
                    }
                    WcpCommand::konata_set_bookmark { slot } => {
                        self.send_error(
                            "konata_set_bookmark",
                            vec![slot.to_string()],
                            "Bookmark slot must be in 0..=9",
                        );
                    }
                    WcpCommand::konata_focus_instruction { transaction_id } => {
                        let target = self.active_konata_tile().and_then(|tile_id| {
                            let source = self.user.konata_tiles.get(&tile_id)?.spec.source;
                            let model = self.active_konata_model(tile_id)?;
                            model.row_for_transaction(*transaction_id).map(|_| source)
                        });
                        if let Some(source) = target {
                            self.update(Message::FocusTransactionFromSource(
                                Some(SourceTransactionRef::new(
                                    source,
                                    TransactionRef {
                                        id: TransactionId(*transaction_id),
                                    },
                                )),
                                None,
                            ));
                            self.send_response(WcpResponse::ack);
                        } else {
                            self.send_error(
                                "konata_focus_instruction",
                                vec![transaction_id.to_string()],
                                "No active ready Konata tile contains that instruction",
                            );
                        }
                    }
                    WcpCommand::shutdown => {
                        warn!("WCP Shutdown message should not reach this place");
                    }
                }
            }
            WcpCSMessage::greeting { version, commands } => {
                if version == "0" {
                    self.wcp_client_capabilities = WcpClientCapabilities::new();
                    if commands.iter().any(|s| s == "waveforms_loaded") {
                        self.wcp_client_capabilities.waveforms_loaded = true;
                    }
                    if commands.iter().any(|s| s == "goto_declaration") {
                        self.wcp_client_capabilities.goto_declaration = true;
                    }
                    if commands.iter().any(|s| s == "add_drivers") {
                        self.wcp_client_capabilities.add_drivers = true;
                    }
                    if commands.iter().any(|s| s == "add_loads") {
                        self.wcp_client_capabilities.add_loads = true;
                    }
                    self.wcp_greeted_signal.store(true, Ordering::Relaxed);
                    self.wcp_greeted_signal.store(true, Ordering::Relaxed);
                    self.send_greeting();
                } else {
                    self.send_error(
                        "greeting",
                        vec![],
                        &format!("Surfer only supports WCP version 0, client requested {version}"),
                    );
                }
            }
        }
    }

    fn send_greeting(&self) {
        let commands = vec![
            "add_variables",
            "set_viewport_to",
            "cursor_set",
            "reload",
            "add_scope",
            "add_items",
            "get_item_list",
            "set_item_color",
            "get_item_info",
            "clear_item",
            "focus_item",
            "clear",
            "load",
            "zoom_to_fit",
            "add_markers",
            "set_viewport_range_to",
            "konata_open",
            "konata_goto_row",
            "konata_goto_rid",
            "konata_goto_cycle",
            "konata_set_bookmark",
            "konata_focus_instruction",
        ]
        .into_iter()
        .map(str::to_string)
        .collect_vec();

        let greeting = WcpSCMessage::create_greeting(0, commands);

        self.channels
            .wcp_s2c_sender
            .as_ref()
            .map(|ch| block_on(ch.send(greeting)));
    }

    fn send_response(&self, result: WcpResponse) {
        self.channels
            .wcp_s2c_sender
            .as_ref()
            .map(|ch| block_on(ch.send(WcpSCMessage::response(result))));
    }

    fn send_error(&self, error: &str, arguments: Vec<String>, message: &str) {
        self.channels.wcp_s2c_sender.as_ref().map(|ch| {
            block_on(ch.send(WcpSCMessage::create_error(
                error.to_string(),
                arguments,
                message.to_string(),
            )))
        });
    }

    fn get_displayed_items(&self, waves: &WaveData) -> Vec<DisplayedItemRef> {
        // TODO check call sites since visible items may now differ from loaded items
        waves
            .items_tree
            .iter_visible()
            .map(|node| node.item_ref)
            .collect_vec()
    }
}
