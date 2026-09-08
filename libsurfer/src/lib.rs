#![deny(unused_crate_dependencies)]

pub mod analog_renderer;
pub mod analog_signal_cache;
pub mod annotation;
pub mod annotation_list;
mod appearance;
pub mod arrow;
pub mod async_util;
pub mod batch_commands;
#[cfg(feature = "performance_plot")]
pub mod benchmark;
mod channels;
#[cfg(not(target_arch = "wasm32"))]
mod chrome;
pub mod clock_highlighting;
pub mod command_parser;
pub mod command_prompt;
pub mod comment;
pub mod config;
pub mod cxxrtl;
pub mod cxxrtl_container;
pub mod data_container;
pub mod dialog;
pub mod displayed_item;
pub mod displayed_item_tree;
pub mod drawing_canvas;
pub mod file_dialog;
pub mod file_history;
pub mod file_watcher;
pub mod frame_buffer;
#[cfg(not(target_arch = "wasm32"))]
pub mod fst_export;
pub mod fzcmd;
pub mod graphics;
pub mod help;
pub mod hierarchy;
pub mod item_drawing_info;
pub mod item_list;
pub mod keyboard_shortcuts;
pub mod keys;
pub mod logs;
pub mod marker;
pub mod memory_viewer;
pub mod menus;
pub mod message;
pub mod mousegestures;
pub mod overview;
pub mod rectangle;
pub mod remote;
pub(crate) mod schematic;
pub mod server_file_window;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod source_code;
pub(crate) mod source_index;
pub mod state;
pub mod state_file_io;
pub mod state_util;
pub mod statusbar;
pub mod system_state;
#[cfg(test)]
pub mod tests;
pub mod tile_kinds;
pub mod tiles;
pub mod time;
pub mod toolbar;
pub mod tooltips;
pub mod trace_style;
pub mod transaction_container;
mod transaction_index;
pub mod transactions;
pub mod translation;
pub mod util;
pub mod variable_direction;
pub mod variable_filter;
mod variable_index;
pub mod variable_meta;
pub mod variable_name_type;
pub mod view;
pub mod viewport;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod vtr_adapter;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod vtr_transactions;
#[cfg(target_arch = "wasm32")]
pub mod wasm_api;
#[cfg(target_arch = "wasm32")]
pub mod wasm_panic;
pub mod wave_container;
pub mod wave_data;
pub mod wave_source;
pub mod wcp;
pub mod wellen;

use crate::annotation::Annotatable;
use crate::annotation::Annotation;
use crate::annotation_list::AnnotationGroup;
use crate::annotation_list::DEFAULT_GROUP_NAME;
use crate::arrow::ArrowAnnotation;
#[cfg(all(not(target_arch = "wasm32"), feature = "wasm_plugins"))]
use crate::channels::checked_send;
use crate::comment::CommentMessage;
use crate::config::AutoLoad;
use crate::displayed_item_tree::ItemIndex;
use crate::displayed_item_tree::TargetPosition;
use crate::rectangle::RectAnnotation;
use crate::remote::get_time_table_from_server;
use crate::tiles::commands::DocumentCommand;
use crate::variable_name_type::VariableNameType;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, LazyLock, RwLock};

use batch_commands::read_command_bytes;
use batch_commands::read_command_file;
#[cfg(target_arch = "wasm32")]
use channels::{GlobalChannelTx, IngressHandler, IngressReceiver};
use derive_more::Display;
use displayed_item::DisplayedVariable;
use displayed_item_tree::DisplayedItemTree;
use eframe::{App, CreationContext};
use egui::{FontData, FontDefinitions, FontFamily};
use eyre::{Result, WrapErr as _};
use futures::executor::block_on;
use itertools::Itertools;
use message::MessageTarget;
use serde::Deserialize;
use surfer_translation_types::Translator;
use surfer_wcp::{WcpCSMessage, WcpEvent, WcpSCMessage};
pub use system_state::SystemState;
#[cfg(target_arch = "wasm32")]
use tokio_stream as _;
use tracing::{error, info, warn};
#[cfg(all(not(target_arch = "wasm32"), feature = "wasm_plugins"))]
use translation::wasm_translator::PluginTranslator;
use wave_container::ScopeRef;

#[cfg(all(not(target_arch = "wasm32"), feature = "wasm_plugins"))]
use crate::async_util::perform_work;
use crate::config::{SurferConfig, SurferTheme};
use crate::dialog::{OpenSiblingStateFileDialog, ReloadWaveformDialog};
use crate::displayed_item::{
    AnalogVarState, DisplayedFieldRef, DisplayedItem, DisplayedItemRef, FieldFormat,
};
use crate::displayed_item_tree::VisibleItemIndex;
use crate::drawing_canvas::TxDrawingCommands;
use crate::frame_buffer::{FrameBufferContent, build_frame_buffer_content};
use crate::message::Message;
use crate::transaction_container::{TransactionRef, TransactionStreamRef};
use crate::translation::{AnyTranslator, all_translators};
use crate::variable_filter::{VariableIOFilterType, VariableNameFilterType};
use crate::viewport::Viewport;
use crate::wave_container::{ScopeRefExt, VariableRef, VariableRefExt, WaveContainer};

use crate::wave_source::{LoadOptions, WaveFormat, WaveSource};
use crate::wellen::{HeaderResult, convert_format};

/// A number that is non-zero if there are asynchronously triggered operations that
/// have been triggered but not successfully completed yet. In practice, if this is
/// non-zero, we will re-run the egui update function in order to ensure that we deal
/// with the outstanding transactions eventually.
/// When incrementing this, it is important to make sure that it gets decremented
/// whenever the asynchronous transaction is completed, otherwise we will re-render
/// things until program exit
pub(crate) static OUTSTANDING_TRANSACTIONS: AtomicUsize = AtomicUsize::new(0);

pub static EGUI_CONTEXT: LazyLock<RwLock<Option<Arc<egui::Context>>>> =
    LazyLock::new(|| RwLock::new(None));

#[cfg(target_arch = "wasm32")]
pub(crate) static WCP_CS_HANDLER: LazyLock<IngressHandler<WcpCSMessage>> =
    LazyLock::new(IngressHandler::new);

#[cfg(target_arch = "wasm32")]
pub(crate) static WCP_SC_HANDLER: LazyLock<GlobalChannelTx<WcpSCMessage>> =
    LazyLock::new(GlobalChannelTx::new);

#[derive(Default)]
pub struct StartupParams {
    pub waves: Option<WaveSource>,
    pub wcp_initiate: Option<u16>,
    pub startup_commands: Vec<String>,
}

fn setup_custom_font(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    fonts.font_data.insert(
        "remix_icons".to_owned(),
        FontData::from_static(egui_remixicon::FONT).into(),
    );

    fonts
        .families
        .get_mut(&FontFamily::Proportional)
        .unwrap()
        .push("remix_icons".to_owned());

    fonts
        .families
        .get_mut(&FontFamily::Monospace)
        .unwrap()
        .push("remix_icons".to_owned());

    ctx.set_fonts(fonts);
}

pub fn run_egui(cc: &CreationContext, mut state: SystemState) -> Result<Box<dyn App>> {
    let ctx_arc = Arc::new(cc.egui_ctx.clone());
    *EGUI_CONTEXT.write().unwrap() = Some(ctx_arc.clone());
    state.context = Some(ctx_arc.clone());
    #[cfg(target_os = "linux")]
    {
        state.window_frame = crate::chrome::WindowFrame::new(cc);
    }
    state.apply_theme_visuals();
    cc.egui_ctx.all_styles_mut(|style| {
        if state.user.config.animation_time == 0.0 {
            info!("With animation_time set to 0.0, animations cannot be enabled.");
        }
        crate::appearance::configure(style);
        style.animation_time = if state.user.config.animation_enabled() {
            state.user.config.animation_time
        } else {
            0.0
        };
    });
    #[cfg(not(target_arch = "wasm32"))]
    if state.user.config.wcp.autostart {
        state.start_wcp_server(Some(state.user.config.wcp.address.clone()), false);
    }
    setup_custom_font(&cc.egui_ctx);
    Ok(Box::new(state))
}

#[derive(Debug, Clone, Copy, Deserialize, Display, PartialEq, Eq)]
pub enum MoveDir {
    #[display("up")]
    Up,

    #[display("down")]
    Down,
}

pub enum ColorSpecifier {
    Index(usize),
    Name(String),
}

enum CachedDrawData {
    Waves(CachedWaveDrawData),
    Transactions(CachedTransactionDrawData),
    Combined(CachedCombinedDrawData),
}

struct CachedWaveDrawData {
    pub draw_commands: HashMap<DisplayedFieldRef, drawing_canvas::DrawingCommands>,
    pub clock_edges: crate::clock_highlighting::ClockHighlightData,
    pub ticks: Vec<(String, f32, i64)>,
}

struct CachedTransactionDrawData {
    pub draw_commands: HashMap<(TransactionStreamRef, TransactionRef), TxDrawingCommands>,
    pub stream_to_displayed_txs: HashMap<TransactionStreamRef, Vec<TransactionRef>>,
    pub inc_relation_tx_ids: Vec<TransactionRef>,
    pub out_relation_tx_ids: Vec<TransactionRef>,
}

struct CachedCombinedDrawData {
    pub wave: CachedWaveDrawData,
    pub transaction: CachedTransactionDrawData,
}

pub struct Channels {
    pub msg_sender: Sender<Message>,
    pub msg_receiver: Receiver<Message>,
    #[cfg(target_arch = "wasm32")]
    wcp_c2s_receiver: Option<IngressReceiver<WcpCSMessage>>,
    #[cfg(not(target_arch = "wasm32"))]
    wcp_c2s_receiver: Option<tokio::sync::mpsc::Receiver<WcpCSMessage>>,
    wcp_s2c_sender: Option<tokio::sync::mpsc::Sender<WcpSCMessage>>,
}
impl Channels {
    fn new() -> Self {
        let (msg_sender, msg_receiver) = mpsc::channel();
        Self {
            msg_sender,
            msg_receiver,
            wcp_c2s_receiver: None,
            wcp_s2c_sender: None,
        }
    }
}

pub struct WcpClientCapabilities {
    pub waveforms_loaded: bool,
    pub goto_declaration: bool,
    pub add_drivers: bool,
    pub add_loads: bool,
}
impl WcpClientCapabilities {
    fn new() -> Self {
        Self {
            waveforms_loaded: false,
            goto_declaration: false,
            add_drivers: false,
            add_loads: false,
        }
    }
}

/// Stores the current canvas state to enable undo/redo operations
struct CanvasState {
    message: String,
    list: crate::tiles::ItemListId,
    items_tree: DisplayedItemTree,
    displayed_items: HashMap<DisplayedItemRef, DisplayedItem>,
    graphics: HashMap<crate::graphics::GraphicId, crate::graphics::Graphic>,
    default_variable_name_type: VariableNameType,
    annotations: Vec<Annotation>,
    annotation_group: Vec<AnnotationGroup>,
    annotation_counter: i32,
}

impl SystemState {
    pub(crate) fn scroll_rows_message(&self, down: bool, count: usize) -> Option<Message> {
        let target = self
            .user
            .workspace
            .resolve_waveform(crate::tiles::TileTarget::Focused)?;
        Some(Message::ToTile(
            target,
            crate::tiles::kind::TileMessage::Waveform(
                crate::tile_kinds::waveform::WaveformMessage::ScrollRows { down, count },
            ),
        ))
    }

    fn navigate_waveform(
        &mut self,
        tile_id: crate::tiles::TileId,
        command: crate::tile_kinds::waveform::WaveformNavigation,
    ) -> Option<()> {
        let waves = self.user.waveform_edit_at(tile_id)?;
        let view = waves.view;
        if let Err(error) = view.apply_navigation(command, Some(waves.document)) {
            tracing::warn!(%error, "invalid waveform navigation");
            return None;
        }
        Some(())
    }

    pub fn update(&mut self, message: Message) -> Option<()> {
        let result = self.apply_message(message);
        if self
            .user
            .waves
            .as_ref()
            .is_some_and(|w| w.format == WaveFormat::Vtr)
        {
            let retained = self.user.workspace.evict_hidden_native_runtime();
            if let Some(document) = &mut self.user.waves {
                document
                    .inflight_caches
                    .retain(|key, _| retained.contains(key));
            }
        }
        self.reconcile_native_signals();
        #[cfg(not(target_arch = "wasm32"))]
        self.reconcile_native_transactions();
        result
    }

    /// Recorded signals the visible source tiles show values for.
    fn source_tile_signals(&self) -> Vec<VariableRef> {
        let workspace = &self.user.workspace;
        workspace
            .layout()
            .visible_tiles()
            .into_iter()
            .filter_map(|id| workspace.tiles().get(&id))
            .filter_map(|tile| match &tile.kind {
                crate::tiles::kind::TileKind::SourceCode(tile) => Some(tile.demanded_signals(self)),
                _ => None,
            })
            .flatten()
            .collect()
    }

    fn reconcile_native_signals(&mut self) {
        let source_signals = self.source_tile_signals();
        let Some(document) = self.user.waves.as_mut() else {
            return;
        };
        let Some(container) = document.inner.as_waves_mut() else {
            return;
        };
        if !matches!(container, WaveContainer::Wellen(waves) if waves.native_backend) {
            // Other recordings keep everything they load; ask once per new set.
            if source_signals != self.source_signals_requested {
                let load = container.load_variables(source_signals.iter());
                self.source_signals_requested = source_signals;
                if let Ok(Some(load)) = load {
                    self.load_variables(load);
                }
            }
            return;
        }
        let workspace = &self.user.workspace;
        let mut variables = source_signals;
        for id in workspace.layout().visible_tiles() {
            let Some(tile) = workspace.tiles().get(&id) else {
                continue;
            };
            if let Some(list) = tile
                .kind
                .waveform_list()
                .and_then(|id| workspace.item_lists().get(&id))
            {
                variables.extend(list.items_tree.iter_visible().filter_map(|node| {
                    match list.displayed_items.get(&node.item_ref) {
                        Some(crate::displayed_item::DisplayedItem::Variable(v)) => {
                            Some(v.variable_ref.clone())
                        }
                        _ => None,
                    }
                }));
            }
            match &tile.kind {
                crate::tiles::kind::TileKind::Memory(tile) => {
                    if let Some(scope) = &tile.settings.scope {
                        variables.extend(container.variables_in_scope(scope));
                    }
                }
                crate::tiles::kind::TileKind::FrameBuffer(tile) => match &tile.state.content {
                    Some(crate::frame_buffer::FrameBufferContent::Variable(variable)) => {
                        variables.push(variable.clone())
                    }
                    Some(crate::frame_buffer::FrameBufferContent::Array { scope_ref, levels })
                        if !levels.is_empty() =>
                    {
                        if let Some(refs) = crate::frame_buffer::resolve_leaf_scopes_and_variables(
                            container, scope_ref, levels,
                        ) {
                            variables.extend(refs);
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        let WaveContainer::Wellen(waves) = container else {
            return;
        };
        if let Some(cmd) = waves.retain_native_variables(&variables) {
            self.load_variables(cmd);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn reconcile_native_transactions(&mut self) {
        use crate::tiles::kind::TileKind;
        use num::ToPrimitive;
        let Some(document) = self.user.waves.as_ref() else {
            return;
        };
        let Some(transactions) = document.inner.as_transactions() else {
            return;
        };
        if !transactions.is_native() {
            return;
        }
        let mut demand = crate::vtr_transactions::TransactionDemand::default();
        let workspace = &self.user.workspace;
        for id in workspace.layout().visible_tiles() {
            let Some(tile) = workspace.tiles().get(&id) else {
                continue;
            };
            match &tile.kind {
                TileKind::Waveform(tile) => {
                    demand
                        .payloads
                        .extend(tile.view.draw_cache.borrow().payloads.iter().copied());
                    let Some(list) = workspace.item_lists().get(&tile.items) else {
                        continue;
                    };
                    let start = tile
                        .view
                        .viewport
                        .left_edge_time(document.time_range())
                        .to_u64()
                        .unwrap_or(0);
                    let end = tile
                        .view
                        .viewport
                        .right_edge_time(document.time_range())
                        .to_u64()
                        .unwrap_or(0);
                    for node in list.items_tree.iter_visible() {
                        if let Some(crate::displayed_item::DisplayedItem::Stream(stream)) =
                            list.displayed_items.get(&node.item_ref)
                        {
                            let reference = &stream.transaction_stream_ref;
                            if let Some(generator) = reference.gen_id {
                                demand.windows.push((generator.0 as u32, start, end));
                            } else if let Some(stream) =
                                transactions.get_stream(reference.stream_id)
                            {
                                demand.windows.extend(
                                    stream.generators.iter().map(|g| (g.0 as u32, start, end)),
                                );
                            }
                        }
                    }
                }
                TileKind::TransactionDetails(_) => {
                    if let Some(reference) = self
                        .user
                        .waveform_read()
                        .and_then(|w| w.view.focused_transaction.as_ref())
                    {
                        demand.pinned.push(reference.id.0 as u64);
                    }
                }
                _ => {}
            }
        }
        let transactions = self
            .user
            .waves
            .as_mut()
            .unwrap()
            .inner
            .as_transactions_mut()
            .unwrap();
        demand.normalize();
        let changed = transactions
            .native
            .as_ref()
            .is_some_and(|native| native.desired != demand);
        let load = transactions.retain_native_transactions(demand);
        if changed {
            self.user.workspace.refresh_transaction_rows(transactions);
            self.invalidate_draw_commands();
        }
        if let Some(load) = load {
            let sender = self.channels.msg_sender.clone();
            let context = self.context.clone();
            crate::async_util::perform_work(move || {
                crate::channels::checked_send(
                    &sender,
                    Message::NativeTransactionsLoaded(load.run()),
                );
                if let Some(context) = context {
                    context.request_repaint();
                }
            });
        }
    }

    fn apply_message(&mut self, message: Message) -> Option<()> {
        if tracing::enabled!(tracing::Level::TRACE)
            && !matches!(message, Message::CommandPromptUpdate { .. })
        {
            tracing::trace!("{message:?}");
        }
        match message {
            Message::DocumentLoadFailed(request) => {
                if request == self.document_load_request {
                    self.begin_document_load();
                }
            }
            Message::DocumentLoadResult(request, message) => {
                if request == self.document_load_request {
                    self.update(*message)?;
                }
            }
            Message::ApplyLayoutProposal(edit) => {
                let movement = if edit.structural {
                    let tile = edit.moved_tile?;
                    crate::tiles::history::move_record(
                        tile,
                        self.user.workspace.layout().root()?,
                        edit.root.as_ref()?,
                    )
                } else {
                    None
                };
                self.user
                    .workspace
                    .apply_layout_edit(edit)
                    .map_err(|error| warn!("Layout proposal rejected: {error}"))
                    .ok()?;
                if let Some(record) = movement {
                    self.record_edit(record);
                }
            }
            Message::Workspace(command) => {
                let move_before = match &command {
                    crate::tiles::commands::WorkspaceCommand::MoveTile { tile, .. } => self
                        .user
                        .workspace
                        .layout()
                        .root()
                        .cloned()
                        .map(|root| (*tile, root)),
                    _ => None,
                };
                let resources_before = crate::tiles::history::ResourceEditStart::capture(
                    &self.user.workspace,
                    &command,
                );
                let layout_before = matches!(
                    &command,
                    crate::tiles::commands::WorkspaceCommand::SetLayout(_)
                )
                .then(|| self.user.workspace.layout().root().cloned());
                let title_before = match &command {
                    crate::tiles::commands::WorkspaceCommand::RenameTile { tile, .. } => self
                        .user
                        .workspace
                        .tiles()
                        .get(tile)
                        .map(|entry| (*tile, entry.title.clone())),
                    _ => None,
                };
                self.user
                    .workspace
                    .apply_command(&mut self.workspace_runtime, command)
                    .map_err(|error| warn!("Workspace command rejected: {error}"))
                    .ok()?;
                if let Some((tile, before)) = move_before
                    && let Some(after) = self.user.workspace.layout().root()
                    && let Some(record) = crate::tiles::history::move_record(tile, &before, after)
                {
                    self.record_edit(record);
                }
                if let Some(record) =
                    resources_before.and_then(|before| before.finish(&self.user.workspace))
                {
                    self.record_edit(record);
                }
                if let Some(before) = layout_before {
                    let after = self.user.workspace.layout().root().cloned();
                    if !crate::tiles::layout::same_topology(before.as_ref(), after.as_ref()) {
                        self.record_edit(crate::tiles::history::UndoRecord::SetLayout {
                            before,
                            after,
                        });
                    }
                }
                if let Some((tile, before)) = title_before {
                    let after = self.user.workspace.tiles()[&tile].title.clone();
                    if before != after {
                        self.record_edit(crate::tiles::history::UndoRecord::Title {
                            tile,
                            before,
                            after,
                        });
                    }
                }
            }
            Message::ToTile(target, message) => {
                let target = self
                    .user
                    .workspace
                    .validate_message_target(target, &message)?;
                let inspect_transaction = matches!(
                    &message,
                    crate::tiles::kind::TileMessage::Waveform(
                        crate::tile_kinds::waveform::WaveformMessage::FocusTransaction(Some(_))
                            | crate::tile_kinds::waveform::WaveformMessage::MoveTransaction { .. }
                    )
                );
                let before = message.item_edit_label().and_then(|label| {
                    let list = self.user.workspace.tiles()[&target].kind.waveform_list()?;
                    Some(Self::current_canvas_state(
                        list,
                        self.user.workspace.item_lists().get(&list)?,
                        label.into(),
                    ))
                });
                let settings_before =
                    message.settings_before(&self.user.workspace.tiles()[&target].kind);
                let changed = self
                    .user
                    .workspace
                    .apply_tile_message(target, message, self.user.waves.as_ref())
                    .map_err(|error| warn!("Tile command rejected: {error}"))
                    .ok()?;
                if changed && let Some(before) = before {
                    self.record_canvas_edit(before);
                }
                if changed && let Some(before) = settings_before {
                    if before.source_changed(&self.user.workspace.tiles()[&target].kind) {
                        self.attach_tile_source(target);
                    }
                    let after = before
                        .capture_like(&self.user.workspace.tiles()[&target].kind)
                        .expect("tile update preserves kind");
                    self.record_edit(crate::tiles::history::UndoRecord::Settings {
                        tile: target,
                        before,
                        after,
                    });
                }
                if inspect_transaction
                    && self
                        .user
                        .workspace
                        .waveform_resources(target)
                        .is_some_and(|(_, view)| view.focused_transaction.is_some())
                {
                    self.update(Message::Workspace(
                        crate::tiles::commands::WorkspaceCommand::OpenTile {
                            kind: "transaction_details".into(),
                            placement: crate::tiles::layout::Placement::Edge(
                                crate::tiles::layout::Direction::Right,
                            ),
                            focus: false,
                        },
                    ))?;
                }
            }
            Message::WcpVariableAction { action, variable } => {
                use crate::tiles::commands::WcpVariableAction;
                if !self
                    .wcp_greeted_signal
                    .load(std::sync::atomic::Ordering::Relaxed)
                {
                    return None;
                }
                let event = match action {
                    WcpVariableAction::GoToDeclaration
                        if self.wcp_client_capabilities.goto_declaration =>
                    {
                        WcpEvent::goto_declaration { variable }
                    }
                    WcpVariableAction::AddDrivers if self.wcp_client_capabilities.add_drivers => {
                        WcpEvent::add_drivers { variable }
                    }
                    WcpVariableAction::AddLoads if self.wcp_client_capabilities.add_loads => {
                        WcpEvent::add_loads { variable }
                    }
                    _ => return None,
                };
                if let Some(channel) = &self.channels.wcp_s2c_sender {
                    let _ = futures::executor::block_on(channel.send(WcpSCMessage::event(event)));
                }
            }
            Message::OpenSimulationLogs(stream, generator) => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let source = &self
                        .user
                        .waves
                        .as_ref()?
                        .inner
                        .as_transactions()?
                        .native
                        .as_ref()?
                        .logs;
                    if !source.streams.iter().any(|s| s.0 == stream)
                        || generator.is_some_and(|id| {
                            !source
                                .generators
                                .iter()
                                .any(|g| g.id == id && g.stream == stream)
                        })
                    {
                        return None;
                    }
                    self.user
                        .workspace
                        .apply_command(
                            &mut self.workspace_runtime,
                            crate::tiles::commands::WorkspaceCommand::OpenTile {
                                kind: "simulation_logs".into(),
                                placement: crate::tiles::layout::Placement::Edge(
                                    crate::tiles::layout::Direction::Down,
                                ),
                                focus: true,
                            },
                        )
                        .ok()?;
                    let id = self.user.workspace.layout().focused()?;
                    let crate::tiles::kind::TileKind::SimulationLogs(tile) =
                        &mut self.user.workspace.tiles_mut().get_mut(&id)?.kind
                    else {
                        return None;
                    };
                    tile.query = crate::tile_kinds::simulation_logs::Query {
                        stream: Some(stream),
                        generator,
                        ..Default::default()
                    };
                }
                #[cfg(target_arch = "wasm32")]
                let _ = (stream, generator);
            }
            Message::OpenSchematic(instance, highlight) => {
                let schematic_tile = self.user.workspace.tiles().iter().find_map(|(id, entry)| {
                    (entry.kind.kind_name() == crate::tiles::kind::SCHEMATIC.name).then_some(*id)
                });
                let tile = if let Some(tile) = schematic_tile {
                    self.user
                        .workspace
                        .apply_command(
                            &mut self.workspace_runtime,
                            crate::tiles::commands::WorkspaceCommand::FocusTile(tile),
                        )
                        .ok()?;
                    tile
                } else {
                    self.user
                        .workspace
                        .apply_command(
                            &mut self.workspace_runtime,
                            crate::tiles::commands::WorkspaceCommand::OpenTile {
                                kind: crate::tiles::kind::SCHEMATIC.name.into(),
                                placement: crate::tiles::layout::Placement::Edge(
                                    crate::tiles::layout::Direction::Right,
                                ),
                                focus: true,
                            },
                        )
                        .ok()?;
                    self.user.workspace.layout().focused().filter(|id| {
                        self.user.workspace.tiles()[id].kind.kind_name()
                            == crate::tiles::kind::SCHEMATIC.name
                    })?
                };
                let crate::tiles::kind::TileKind::Schematic(schematic) =
                    &mut self.user.workspace.tiles_mut().get_mut(&tile)?.kind
                else {
                    return None;
                };
                schematic.open(instance, highlight);
            }
            Message::RevealSchematicHierarchy(path) => {
                let scope = ScopeRef::from_strs(&path.split('.').collect::<Vec<_>>());
                self.user.show_hierarchy = Some(true);
                self.user
                    .waves
                    .as_mut()?
                    .set_active_scope(Some(crate::wave_data::ScopeType::WaveScope(scope.clone())));
                *self.scope_ref_to_expand.borrow_mut() =
                    Some(crate::hierarchy::ScopeExpandType::ExpandSpecific(scope));
            }
            Message::SourceInstance(instance) => {
                let tile =
                    self.user.workspace.tiles_mut().values_mut().find_map(
                        |entry| match &mut entry.kind {
                            crate::tiles::kind::TileKind::SourceCode(source) => Some(source),
                            _ => None,
                        },
                    )?;
                tile.set_instance(instance);
            }
            Message::OpenSource {
                file,
                line,
                column,
                instance,
            } => {
                let source_tile = self.user.workspace.tiles().iter().find_map(|(id, entry)| {
                    (entry.kind.kind_name() == crate::tiles::kind::SOURCE_CODE.name).then_some(*id)
                });
                let tile = if let Some(tile) = source_tile {
                    self.user
                        .workspace
                        .apply_command(
                            &mut self.workspace_runtime,
                            crate::tiles::commands::WorkspaceCommand::FocusTile(tile),
                        )
                        .ok()?;
                    tile
                } else {
                    self.user
                        .workspace
                        .apply_command(
                            &mut self.workspace_runtime,
                            crate::tiles::commands::WorkspaceCommand::OpenTile {
                                kind: crate::tiles::kind::SOURCE_CODE.name.into(),
                                placement: crate::tiles::layout::Placement::Edge(
                                    crate::tiles::layout::Direction::Right,
                                ),
                                focus: true,
                            },
                        )
                        .ok()?;
                    self.user.workspace.layout().focused().filter(|id| {
                        self.user.workspace.tiles()[id].kind.kind_name()
                            == crate::tiles::kind::SOURCE_CODE.name
                    })?
                };
                let crate::tiles::kind::TileKind::SourceCode(source) =
                    &mut self.user.workspace.tiles_mut().get_mut(&tile)?.kind
                else {
                    return None;
                };
                source.open(
                    crate::source_index::SourceLocation { file, line, column },
                    instance,
                );
            }
            Message::ToDocument(command) => {
                self.user.waves.as_mut()?.apply_command(command)?;
            }

            Message::ExpandScope(scope_ref) => {
                *self.scope_ref_to_expand.borrow_mut() = Some(scope_ref);
            }
            Message::AddVariables(vars) => {
                if vars.is_empty() {
                    return Some(());
                }
                let undo_msg = if vars.len() == 1 {
                    format!("Add variable {}", vars[0].name)
                } else {
                    format!("Add {} variables", vars.len())
                };
                // Reject unavailable paths before changing a list or creating a tile.
                let container = self.user.waves.as_ref()?.inner.as_waves()?;
                for variable in &vars {
                    container.variable_meta(variable).ok()?;
                }
                let target = self
                    .user
                    .workspace
                    .resolve_waveform(crate::tiles::TileTarget::Focused);
                let load = if let Some(target) = target {
                    let list = self.user.workspace.tiles()[&target].kind.waveform_list()?;
                    let mut waves = self.user.waveform_edit_at(target)?;
                    let before = Self::current_canvas_state(list, waves.items, undo_msg);
                    let (load, inserted) =
                        waves.add_variables(&self.translators, vars, None, true, false, None, true);
                    if !inserted.is_empty() {
                        self.record_canvas_edit(before);
                    }
                    load
                } else {
                    use crate::tiles::{
                        commands::WorkspaceCommand,
                        kind::{TileEntry, TileKind},
                        layout::{Direction, Placement},
                    };
                    let placement = self
                        .user
                        .workspace
                        .layout()
                        .focused()
                        .map(|id| Placement::Beside(id, Direction::Right))
                        .unwrap_or(Placement::Root);
                    let command = WorkspaceCommand::CreateTile {
                        kind: "waveform".into(),
                        placement,
                        focus: true,
                    };
                    let before = crate::tiles::history::ResourceEditStart::capture(
                        &self.user.workspace,
                        &command,
                    )?
                    .with_label("Add variables");
                    // Stage only the new resources. Neither the empty tile nor a partial
                    // insertion becomes visible if initialization fails.
                    let (mut kind, mut lists) =
                        TileKind::create("waveform", &mut self.workspace_runtime).ok()?;
                    let TileKind::Waveform(tile) = &mut kind else {
                        unreachable!()
                    };
                    let items = lists.get_mut(&tile.items)?;
                    items.default_variable_name_type = self.user.config.default_variable_name_type;
                    let mut waves = crate::wave_data::WaveformEdit {
                        document: self.user.waves.as_mut()?,
                        items,
                        view: &mut tile.view,
                        peers: vec![],
                    };
                    let count = vars.len();
                    let (load, inserted) =
                        waves.add_variables(&self.translators, vars, None, true, false, None, true);
                    if inserted.len() != count {
                        if let Some(load) = load {
                            self.load_variables(load);
                        }
                        return None;
                    }
                    self.user
                        .workspace
                        .insert_prepared(
                            &mut self.workspace_runtime,
                            TileEntry { title: None, kind },
                            lists,
                            placement,
                            true,
                        )
                        .ok()?;
                    if let Some(record) = before.finish(&self.user.workspace) {
                        self.record_edit(record);
                    }
                    load
                };
                if let Some(load) = load {
                    self.load_variables(load);
                }
                self.invalidate_draw_commands();
            }

            Message::DownloadDefaultConfig => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    if let Err(e) = crate::config::write_default_config() {
                        tracing::error!("Failed to write default config: {}", e);
                    }
                }

                #[cfg(target_arch = "wasm32")]
                {
                    tracing::warn!("Download default config is not supported on WASM");
                }
            }

            Message::AddDivider(name, vidx) => {
                let target = self
                    .user
                    .workspace
                    .resolve_waveform(crate::tiles::TileTarget::Focused)?;
                let list = self.user.workspace.tiles()[&target].kind.waveform_list()?;
                let mut waves = self.user.waveform_edit_at(target)?;
                let before = Self::current_canvas_state(list, waves.items, "Add divider".into());
                waves.add_divider(name, vidx).ok()?;
                self.record_canvas_edit(before);
            }
            Message::AddTimeLine(vidx) => {
                let target = self
                    .user
                    .workspace
                    .resolve_waveform(crate::tiles::TileTarget::Focused)?;
                let list = self.user.workspace.tiles()[&target].kind.waveform_list()?;
                let mut waves = self.user.waveform_edit_at(target)?;
                let before = Self::current_canvas_state(list, waves.items, "Add timeline".into());
                waves.add_timeline(vidx).ok()?;
                self.record_canvas_edit(before);
            }
            Message::AddScope(scope, recursive) => {
                self.save_current_canvas(format!("Add scope {}", scope.name()));

                let vars = self.get_scope(&scope, recursive);
                let mut waves = self.user.waveform_edit()?;

                if let (Some(cmd), _) =
                    waves.add_variables(&self.translators, vars, None, true, false, None, true)
                {
                    self.load_variables(cmd);
                }

                self.invalidate_draw_commands();
            }
            Message::AddScopeAsGroup(scope, recursive) => {
                self.save_current_canvas(format!("Add scope {} as group", scope.name()));
                let waves = self.user.waveform_edit()?;
                let passed_or_focused = waves
                    .items
                    .insert_position(waves.view.focused_index(waves.items));
                let target = passed_or_focused.unwrap_or_else(|| waves.items.end_insert_position());

                self.add_scope_as_group(&scope, target, recursive, None);
                self.invalidate_draw_commands();

                let waves = self.user.waveform_edit()?;
                waves.items.compute_variable_display_names(
                    &waves.document.inner,
                    waves.document.display_variable_indices,
                );
            }
            Message::AddCount(digit) => {
                if let Some(count) = &mut self.user.count {
                    count.push(digit);
                } else {
                    self.user.count = Some(digit.to_string());
                }
            }
            Message::AddStreamOrGenerator(s) => {
                let undo_msg = if let Some(gen_id) = s.gen_id {
                    format!("Add generator(id: {gen_id})")
                } else {
                    format!("Add stream(id: {})", s.stream_id)
                };
                self.save_current_canvas(undo_msg);

                let mut waves = self.user.waveform_edit()?;
                if s.gen_id.is_some() {
                    waves.add_generator(s).ok()?;
                } else {
                    waves.add_stream(s).ok()?;
                }
                self.invalidate_draw_commands();
            }
            Message::AddStreamOrGeneratorFromName(scope, name) => {
                self.save_current_canvas(format!("Add Stream/Generator from name: {name}"));
                let mut waves = self.user.waveform_edit()?;
                waves.add_stream_or_generator_from_name(scope, name)?;
                self.invalidate_draw_commands();
            }
            Message::AddAllFromStreamScope(scope_name) => {
                self.save_current_canvas(format!("Add all from scope {}", scope_name.clone()));
                let mut waves = self.user.waveform_edit()?;
                waves.add_all_from_stream_scope(scope_name)?;
                self.invalidate_draw_commands();
            }
            Message::InvalidateCount => self.user.count = None,
            Message::SetNameAlignRight(align_right) => {
                self.user.align_names_right = Some(align_right);
            }
            Message::FocusItem(idx) => {
                let waves = self.user.waveform_edit()?;

                if let Some(node) = waves.items.items_tree.get_visible(idx) {
                    waves.view.focused_item = Some(node.item_ref);
                } else {
                    error!("Cannot focus missing visible item {}", idx.0);
                }
            }
            Message::ItemSelectRange(select_to) => {
                let waves = self.user.waveform_edit()?;
                let from = waves.view.focused_item?;
                let to = waves.items.items_tree.get_visible(select_to)?.item_ref;
                if waves
                    .items
                    .apply_selection(crate::item_list::ItemSelection::Range {
                        from,
                        to,
                        selected: true,
                    })
                    .ok()?
                {
                    self.invalidate_draw_commands();
                }
            }
            Message::ItemSelectAll => {
                let waves = self.user.waveform_edit()?;
                if waves
                    .items
                    .apply_selection(crate::item_list::ItemSelection::AllVisible(true))
                    .ok()?
                {
                    self.invalidate_draw_commands();
                }
            }
            Message::SetItemSelected(vidx, selected) => {
                let waves = self.user.waveform_edit()?;
                let item = waves.items.items_tree.get_visible(vidx)?.item_ref;
                if waves
                    .items
                    .apply_selection(crate::item_list::ItemSelection::Set { item, selected })
                    .ok()?
                {
                    self.invalidate_draw_commands();
                }
            }
            Message::ToggleItemSelected(vidx) => {
                let waves = self.user.waveform_edit()?;
                let item = vidx
                    .or(waves.view.focused_index(waves.items))
                    .and_then(|index| waves.items.items_tree.get_visible(index))?
                    .item_ref;
                if waves
                    .items
                    .apply_selection(crate::item_list::ItemSelection::Toggle(item))
                    .ok()?
                {
                    self.invalidate_draw_commands();
                }
            }
            Message::SetDefaultTimeline(v) => {
                self.user.show_default_timeline = Some(v);
            }
            Message::UnfocusItem => {
                let waves = self.user.waveform_edit()?;
                waves.view.focused_item = None;
            }
            Message::MoveFocus(direction, count, select) => {
                let waves = self.user.waveform_edit()?;
                let visible_item_cnt = waves.items.items_tree.iter_visible().count();
                if visible_item_cnt == 0 {
                    return None;
                }

                let new_focus_vidx = VisibleItemIndex(match direction {
                    MoveDir::Up => waves
                        .view
                        .focused_index(waves.items)
                        .map_or(visible_item_cnt, |vidx| vidx.0)
                        .saturating_sub(count),
                    MoveDir::Down => waves
                        .view
                        .focused_index(waves.items)
                        .map_or(usize::MAX, |vidx| vidx.0)
                        .wrapping_add(count)
                        .clamp(0, visible_item_cnt - 1),
                });

                if select {
                    if let Some(vidx) = waves.view.focused_index(waves.items) {
                        waves.items.items_tree.xselect(vidx, true);
                    }
                    waves.items.items_tree.xselect(new_focus_vidx, true);
                }
                waves.view.focused_item = waves
                    .items
                    .items_tree
                    .get_visible(new_focus_vidx)
                    .map(|node| node.item_ref);
            }
            Message::FocusTransaction(tx_ref, tile_id) => {
                self.update(Message::ToTile(
                    tile_id,
                    crate::tiles::kind::TileMessage::Waveform(
                        crate::tile_kinds::waveform::WaveformMessage::FocusTransaction(tx_ref),
                    ),
                ))?;
            }
            Message::WaveformBodyMeasured {
                tile_id,
                height,
                scroll_offset,
            } => {
                self.user
                    .workspace
                    .measure_waveform(tile_id, height, scroll_offset)
                    .ok()?;
            }
            Message::SetFrameBufferVariable(variable) => {
                let variable = variable.map_ids(|_| Default::default(), |_| Default::default());
                self.open_framebuffer(Some(FrameBufferContent::Variable(variable)))?;
            }
            Message::SetFrameBufferVisibleVariable(None) => {
                if let Some(tile) = self.framebuffer_target() {
                    self.update(Message::Workspace(
                        crate::tiles::commands::WorkspaceCommand::CloseTile(tile),
                    ))?;
                }
            }
            Message::SetFrameBufferVisibleVariable(Some(vidx)) => {
                let waves = self.user.waveform_read()?;
                let variable = waves
                    .items
                    .items_tree
                    .get_visible(vidx)
                    .and_then(|node| waves.items.displayed_items.get(&node.item_ref))
                    .and_then(|item| match item {
                        DisplayedItem::Variable(variable) => Some(variable.variable_ref.clone()),
                        _ => None,
                    })?;
                self.update(Message::SetFrameBufferVariable(variable))?;
            }
            Message::SetFrameBufferArray(scope_ref) => {
                let scope_ref = crate::wave_container::ScopeRef {
                    strs: scope_ref.strs,
                    id: Default::default(),
                };
                let levels = self
                    .user
                    .waves
                    .as_ref()
                    .and_then(|waves| waves.inner.as_waves())
                    .and_then(|container| build_frame_buffer_content(container, &scope_ref))
                    .map_or_else(Vec::new, |(levels, _)| levels);
                self.open_framebuffer(Some(FrameBufferContent::Array { scope_ref, levels }))?;
            }
            Message::SetFrameBufferMode(mode, a, b, c) => {
                self.update_framebuffer(
                    crate::tile_kinds::frame_buffer::FrameBufferMessage::Mode(mode, a, b, c),
                )?;
            }
            Message::SetFrameBufferWidth(width) => {
                self.update_framebuffer(
                    crate::tile_kinds::frame_buffer::FrameBufferMessage::Width(width),
                )?;
            }
            Message::SetFrameBufferRange(ranges) => {
                self.update_framebuffer(
                    crate::tile_kinds::frame_buffer::FrameBufferMessage::Range(ranges),
                )?;
            }
            Message::OpenMemoryViewer {
                scope,
                name,
                placement,
            } => {
                use crate::tiles::{
                    commands::WorkspaceCommand,
                    kind::{TileKind, TileMessage},
                    layout::{Direction, Placement},
                };
                let placement = placement
                    .filter(|placement| match placement {
                        Placement::TabAfter(anchor) | Placement::Beside(anchor, _) => {
                            self.user.workspace.tiles().contains_key(anchor)
                        }
                        Placement::Edge(_) | Placement::Root => true,
                    })
                    .or_else(|| {
                        self.user
                            .workspace
                            .layout()
                            .focused()
                            .map(|id| Placement::Beside(id, Direction::Right))
                    })
                    .unwrap_or(Placement::Root);
                let command = WorkspaceCommand::CreateTile {
                    kind: "memory".into(),
                    placement,
                    focus: true,
                };
                let before = crate::tiles::history::ResourceEditStart::capture(
                    &self.user.workspace,
                    &command,
                );
                self.user
                    .workspace
                    .apply_command(&mut self.workspace_runtime, command)
                    .ok()?;
                let id = self.user.workspace.layout().focused()?;
                let scope = crate::wave_container::ScopeRef {
                    strs: scope.strs,
                    id: Default::default(),
                };
                let TileKind::Memory(tile) = &self.user.workspace.tiles()[&id].kind else {
                    unreachable!()
                };
                let mut settings = tile.settings.clone();
                settings.scope = Some(scope.clone());
                settings.name = name;
                self.user
                    .workspace
                    .apply_tile_message(
                        id,
                        TileMessage::Memory(crate::tile_kinds::memory::MemoryMessage::Settings(
                            Box::new(settings),
                        )),
                        self.user.waves.as_ref(),
                    )
                    .ok()?;
                if let Some(record) = before.and_then(|before| before.finish(&self.user.workspace))
                {
                    self.record_edit(record);
                }
                if let Some(container) = self
                    .user
                    .waves
                    .as_mut()
                    .and_then(|waves| waves.inner.as_waves_mut())
                {
                    let variables = container.variables_in_scope(&scope);
                    if let Some(cmd) = container
                        .load_variables(variables.iter())
                        .map_err(|e| error!("{e:#?}"))
                        .ok()
                        .flatten()
                    {
                        self.load_variables(cmd);
                    }
                }
            }
            Message::SetSurverFileWindowVisible(visibility) => {
                self.user.show_server_file_window = visibility;
            }
            Message::LoadSurverFileByIndex(file_index, load_options) => {
                // Disable file window in case executing from command/test
                self.user.show_server_file_window = false;
                let force_switch = self.user.selected_server_file_index != file_index;
                if let Some(url) = self.user.surver_url.as_ref() {
                    self.load_wave_from_url(url.clone(), load_options, force_switch, file_index);
                }
            }
            Message::LoadSurverFileByName(file_name, load_options) => {
                // Disable file window in case executing from command/test
                self.user.show_server_file_window = false;
                let file_index = self
                    .user
                    .surver_file_infos
                    .as_ref()?
                    .iter()
                    .position(|fi| fi.filename == file_name);
                let force_switch = self.user.selected_server_file_index != file_index;

                if let Some(url) = self.user.surver_url.as_ref() {
                    self.load_wave_from_url(url.clone(), load_options, force_switch, file_index);
                }
            }
            Message::RemoveVisibleItems(target) => match target {
                MessageTarget::Explicit(vidx) => {
                    let waves = self.user.waveform_read();
                    let item_ref = waves
                        .and_then(|waves| waves.items.items_tree.get_visible(vidx))
                        .map(|node| node.item_ref);
                    let undo_msg = item_ref
                        .and_then(|item_ref| {
                            waves.and_then(|waves| waves.items.displayed_items.get(&item_ref))
                        })
                        .map(displayed_item::DisplayedItem::name)
                        .map_or("Remove one item".to_string(), |name| {
                            format!("Remove item {name}")
                        });
                    self.save_current_canvas(undo_msg);

                    if let Some(mut waves) = self.user.waveform_edit()
                        && let Some(item_ref) = item_ref
                    {
                        waves.remove_displayed_items(&[item_ref]);
                        waves.items.compute_variable_display_names(
                            &waves.document.inner,
                            waves.document.display_variable_indices,
                        );
                    }
                }
                MessageTarget::CurrentSelection => {
                    self.save_current_canvas("Remove selected items".to_owned());
                    let mut waves = self.user.waveform_edit()?;

                    let mut remove_ids: Vec<_> = waves
                        .items
                        .items_tree
                        .iter_visible_selected()
                        .map(|node| node.item_ref)
                        .collect();
                    if let Some(node) = waves
                        .view
                        .focused_index(waves.items)
                        .and_then(|focus| waves.items.items_tree.get_visible(focus))
                    {
                        remove_ids.push(node.item_ref);
                    }
                    waves.remove_displayed_items(&remove_ids);
                    waves.items.compute_variable_display_names(
                        &waves.document.inner,
                        waves.document.display_variable_indices,
                    );
                }
            },
            Message::RemoveItems(items) => {
                if !items.iter().any(|id| {
                    self.user
                        .waveform_read()
                        .is_some_and(|waves| waves.items.displayed_items.contains_key(id))
                }) {
                    return None;
                }
                let undo_msg = self
                    .user
                    .waveform_read()
                    .and_then(|waves| {
                        if items.len() == 1 {
                            items.first().and_then(|item_ref| {
                                waves
                                    .items
                                    .displayed_items
                                    .get(item_ref)
                                    .map(|item| format!("Remove item {}", item.name()))
                            })
                        } else {
                            Some(format!("Remove {} items", items.len()))
                        }
                    })
                    .unwrap_or_default();
                self.save_current_canvas(undo_msg);

                let mut waves = self.user.waveform_edit()?;
                waves.remove_displayed_items(&items);
            }
            Message::MoveFocusedItem(direction, count) => {
                self.save_current_canvas(format!("Move item {direction}, {count}"));
                self.invalidate_draw_commands();
                let waves = self.user.waveform_edit()?;
                let mut vidx = waves.view.focused_index(waves.items)?;
                for _ in 0..count {
                    vidx = waves
                        .items
                        .items_tree
                        .move_item(vidx, direction, |node| {
                            matches!(
                                waves.items.displayed_items.get(&node.item_ref),
                                Some(DisplayedItem::Group(..))
                            )
                        })
                        .expect("move failed for unknown reason");
                }
            }
            Message::CanvasScroll { delta, tile_id } => {
                self.navigate_waveform(
                    tile_id,
                    crate::tile_kinds::waveform::WaveformNavigation::Pan(
                        f64::from(delta.y) + f64::from(delta.x),
                    ),
                )?;
            }
            Message::CanvasZoom {
                delta,
                mouse_ptr,
                tile_id,
            } => {
                self.navigate_waveform(
                    tile_id,
                    crate::tile_kinds::waveform::WaveformNavigation::Zoom {
                        factor: f64::from(delta),
                        anchor: mouse_ptr,
                    },
                )?;
            }
            Message::ZoomToCursor { delta, tile_id } => {
                self.navigate_waveform(
                    tile_id,
                    crate::tile_kinds::waveform::WaveformNavigation::ZoomToCursor {
                        factor: f64::from(delta),
                    },
                )?;
            }
            Message::ZoomToFit { tile_id } => {
                self.navigate_waveform(
                    tile_id,
                    crate::tile_kinds::waveform::WaveformNavigation::ZoomToFit,
                )?;
            }
            Message::GoToEnd { tile_id } => {
                self.navigate_waveform(
                    tile_id,
                    crate::tile_kinds::waveform::WaveformNavigation::GoToEnd,
                )?;
            }
            Message::GoToStart { tile_id } => {
                self.navigate_waveform(
                    tile_id,
                    crate::tile_kinds::waveform::WaveformNavigation::GoToStart,
                )?;
            }
            Message::GoToTime(time, tile_id) => {
                self.navigate_waveform(
                    tile_id,
                    crate::tile_kinds::waveform::WaveformNavigation::GoToTime(time?),
                )?;
            }
            Message::SetTimeUnit(timeunit) => {
                self.user.wanted_timeunit = timeunit;
                self.invalidate_draw_commands();
            }
            Message::SetTimeStringFormatting(format) => {
                self.user.time_string_format = format;
                self.invalidate_draw_commands();
            }
            Message::ZoomToRange {
                start,
                end,
                tile_id,
            } => {
                self.navigate_waveform(
                    tile_id,
                    crate::tile_kinds::waveform::WaveformNavigation::ZoomToRange { start, end },
                )?;
            }
            Message::VariableFormatChange(displayed_field_ref, format) => {
                let waves = self.user.waveform_edit()?;
                if !self
                    .translators
                    .all_translator_names()
                    .contains(&format.as_str())
                {
                    warn!("No translator {format}");
                    return None;
                }

                let update_format =
                    |variable: &mut DisplayedVariable, field_ref: DisplayedFieldRef| {
                        if field_ref.field.is_empty() {
                            let Ok(meta) = waves
                                .document
                                .inner
                                .as_waves()
                                .unwrap()
                                .variable_meta(&variable.variable_ref)
                                .map_err(|e| {
                                    warn!("Error trying to get variable metadata: {e:#?}");
                                })
                            else {
                                return;
                            };
                            let translator = self.translators.get_translator(&format);
                            let new_info = translator.variable_info(&meta).unwrap();

                            variable.format = Some(format.clone());
                            variable.info = new_info;

                            variable.downgrade_type_limits_if_unsupported(translator, &meta);
                        } else {
                            variable
                                .field_formats
                                .retain(|ff| ff.field != field_ref.field);
                            variable.field_formats.push(FieldFormat {
                                field: field_ref.field,
                                format: format.clone(),
                            });
                        }
                    };

                // convert focused item index to item ref
                let focused = waves
                    .view
                    .focused_index(waves.items)
                    .and_then(|vidx| waves.items.items_tree.get_visible(vidx))
                    .map(|node| node.item_ref);

                let mut redraw = false;

                match displayed_field_ref {
                    MessageTarget::Explicit(field_ref) => {
                        if let Some(DisplayedItem::Variable(displayed_variable)) =
                            waves.items.displayed_items.get_mut(&field_ref.item)
                        {
                            update_format(displayed_variable, field_ref);
                            redraw = true;
                        }
                    }
                    MessageTarget::CurrentSelection => {
                        //If an item is focused, update its format too
                        if let Some(focused) = focused
                            && let Some(DisplayedItem::Variable(displayed_variable)) =
                                waves.items.displayed_items.get_mut(&focused)
                        {
                            update_format(displayed_variable, DisplayedFieldRef::from(focused));
                            redraw = true;
                        }
                        for item in waves
                            .items
                            .items_tree
                            .iter_visible_selected()
                            .map(|node| node.item_ref)
                        {
                            //Update format for all selected
                            let field_ref = DisplayedFieldRef::from(item);
                            if let Some(DisplayedItem::Variable(variable)) =
                                waves.items.displayed_items.get_mut(&item)
                            {
                                update_format(variable, field_ref);
                            }
                            redraw = true;
                        }
                    }
                }

                if redraw {
                    self.invalidate_draw_commands();
                }
            }
            Message::ItemSelectionClear => {
                let waves = self.user.waveform_edit()?;
                if waves
                    .items
                    .apply_selection(crate::item_list::ItemSelection::AllVisible(false))
                    .ok()?
                {
                    self.invalidate_draw_commands();
                }
            }
            Message::ItemColorChange(vidx, color_name) => {
                self.save_current_canvas(format!(
                    "Change item color to {}",
                    color_name.clone().unwrap_or("default".into())
                ));
                self.invalidate_draw_commands();
                let waves = self.user.waveform_edit()?;

                match vidx {
                    MessageTarget::Explicit(vidx) => {
                        let node = waves.items.items_tree.get_visible(vidx)?;
                        waves
                            .items
                            .displayed_items
                            .entry(node.item_ref)
                            .and_modify(|item| item.set_color(&color_name));
                    }
                    MessageTarget::CurrentSelection => {
                        if let Some(focused) = waves.view.focused_index(waves.items) {
                            let node = waves.items.items_tree.get_visible(focused)?;
                            waves
                                .items
                                .displayed_items
                                .entry(node.item_ref)
                                .and_modify(|item| item.set_color(&color_name));
                        }

                        for node in waves.items.items_tree.iter_visible_selected() {
                            waves
                                .items
                                .displayed_items
                                .entry(node.item_ref)
                                .and_modify(|item| item.set_color(&color_name));
                        }
                    }
                }
            }
            Message::ItemNameChange(vidx, name) => {
                self.save_current_canvas(format!(
                    "Change item name to {}",
                    name.clone().unwrap_or("default".into())
                ));
                let waves = self.user.waveform_edit()?;
                let vidx = vidx.or(waves.view.focused_index(waves.items))?;
                let node = waves.items.items_tree.get_visible(vidx)?;
                waves
                    .items
                    .displayed_items
                    .entry(node.item_ref)
                    .and_modify(|item| item.set_name(name));
            }
            Message::ItemNameReset(target) => {
                self.save_current_canvas("Resetting item name(s)".to_owned());
                let waves = self.user.waveform_edit()?;
                match target {
                    MessageTarget::Explicit(vidx) => {
                        let node = waves.items.items_tree.get_visible(vidx)?;
                        waves
                            .items
                            .displayed_items
                            .entry(node.item_ref)
                            .and_modify(|item| item.set_name(None));
                    }
                    MessageTarget::CurrentSelection => {
                        for node in waves.items.items_tree.iter_visible_selected() {
                            waves
                                .items
                                .displayed_items
                                .entry(node.item_ref)
                                .and_modify(|item| item.set_name(None));
                        }
                    }
                }
            }
            Message::ItemBackgroundColorChange(vidx, color_name) => {
                self.save_current_canvas(format!(
                    "Change item background color to {}",
                    color_name.clone().unwrap_or("default".into())
                ));
                let waves = self.user.waveform_edit()?;

                match vidx {
                    MessageTarget::Explicit(vidx) => {
                        let node = waves.items.items_tree.get_visible(vidx)?;
                        waves
                            .items
                            .displayed_items
                            .entry(node.item_ref)
                            .and_modify(|item| item.set_background_color(&color_name));
                    }
                    MessageTarget::CurrentSelection => {
                        if let Some(focused) = waves.view.focused_index(waves.items) {
                            let node = waves.items.items_tree.get_visible(focused)?;
                            waves
                                .items
                                .displayed_items
                                .entry(node.item_ref)
                                .and_modify(|item| item.set_background_color(&color_name));
                        }

                        for node in waves.items.items_tree.iter_visible_selected() {
                            waves
                                .items
                                .displayed_items
                                .entry(node.item_ref)
                                .and_modify(|item| item.set_background_color(&color_name));
                        }
                    }
                }
            }
            Message::ItemHeightScalingFactorChange(vidx, scale) => {
                self.save_current_canvas(format!("Change item height scaling factor to {scale}"));
                let waves = self.user.waveform_edit()?;

                match vidx {
                    MessageTarget::Explicit(vidx) => {
                        let node = waves.items.items_tree.get_visible(vidx)?;
                        waves
                            .items
                            .displayed_items
                            .entry(node.item_ref)
                            .and_modify(|item| item.set_height_scaling_factor(scale));
                    }
                    MessageTarget::CurrentSelection => {
                        if let Some(focused) = waves.view.focused_index(waves.items) {
                            let node = waves.items.items_tree.get_visible(focused)?;
                            waves
                                .items
                                .displayed_items
                                .entry(node.item_ref)
                                .and_modify(|item| item.set_height_scaling_factor(scale));
                        }

                        for node in waves.items.items_tree.iter_visible_selected() {
                            waves
                                .items
                                .displayed_items
                                .entry(node.item_ref)
                                .and_modify(|item| item.set_height_scaling_factor(scale));
                        }
                    }
                }
            }
            Message::SetAnalogSettings(vidx, new_settings) => {
                self.save_current_canvas("Set analog state".into());
                self.invalidate_draw_commands();
                let analog_waveform_multiplier = self.user.config.layout.analog_waveform_multiplier;
                let waves = self.user.waveform_edit()?;

                // Update settings while preserving existing cache
                let update = |item: &mut DisplayedItem| {
                    if let DisplayedItem::Variable(var) = item {
                        match (&mut var.analog, new_settings) {
                            (Some(s), Some(new)) => s.settings = new,
                            (None, Some(new)) => {
                                var.analog = Some(AnalogVarState::new(new));
                                var.height_scaling_factor = Some(analog_waveform_multiplier);
                            }
                            (_, None) => var.analog = None,
                        }
                    }
                };

                match vidx {
                    MessageTarget::Explicit(vidx) => {
                        let node = waves.items.items_tree.get_visible(vidx)?;
                        waves
                            .items
                            .displayed_items
                            .entry(node.item_ref)
                            .and_modify(update);
                    }
                    MessageTarget::CurrentSelection => {
                        if let Some(focused) = waves.view.focused_index(waves.items) {
                            let node = waves.items.items_tree.get_visible(focused)?;
                            waves
                                .items
                                .displayed_items
                                .entry(node.item_ref)
                                .and_modify(update);
                        }
                        for node in waves.items.items_tree.iter_visible_selected() {
                            waves
                                .items
                                .displayed_items
                                .entry(node.item_ref)
                                .and_modify(update);
                        }
                    }
                }
            }
            Message::MoveCursorToTransition {
                next,
                variable,
                skip_zero,
            } => {
                let mut waves = self.user.waveform_edit()?;
                // If there are no timestamps, the file is not fully loaded
                if waves.max_timestamp().is_some() {
                    // if no cursor is set, move it to
                    // start of visible area transition for next transition
                    // end of visible area for previous transition
                    let cursor = waves.document.cursor.clone().or_else(|| {
                        waves
                            .view
                            .focused_index(waves.items)
                            .map(|_| &waves.view.viewport)
                            .map(|vp| {
                                if next {
                                    vp.left_edge_time(waves.time_range())
                                } else {
                                    vp.right_edge_time(waves.time_range())
                                }
                            })
                    });
                    if let Some(time) =
                        waves.cursor_at_transition(cursor.as_ref(), next, variable, skip_zero)
                    {
                        waves
                            .document
                            .apply_command(DocumentCommand::CursorSet(time))?;
                    }
                    let moved = waves.go_to_cursor_if_not_in_view();
                    if moved {
                        self.invalidate_draw_commands();
                    }
                } else {
                    warn!(
                        "Move cursor to transition: No timestamps count, even though waveforms should be loaded"
                    );
                }
            }
            Message::MoveTransaction { next } => {
                let tile = self
                    .user
                    .workspace
                    .resolve_waveform(crate::tiles::TileTarget::Focused)?;
                self.update(Message::ToTile(
                    tile,
                    crate::tiles::kind::TileMessage::Waveform(
                        crate::tile_kinds::waveform::WaveformMessage::MoveTransaction { next },
                    ),
                ))?;
            }
            Message::ResetVariableFormat(displayed_field_ref) => {
                let waves = self.user.waveform_edit()?;
                if let Some(DisplayedItem::Variable(displayed_variable)) = waves
                    .items
                    .displayed_items
                    .get_mut(&displayed_field_ref.item)
                {
                    if displayed_field_ref.field.is_empty() {
                        displayed_variable.format = None;
                    } else {
                        displayed_variable
                            .field_formats
                            .retain(|ff| ff.field != displayed_field_ref.field);
                    }
                    self.invalidate_draw_commands();
                }
            }
            Message::ExpandParameterSection => {
                self.expand_parameter_section = true;
            }
            Message::LoadFile(filename, load_options) => {
                self.user.selected_server_file_index = None;
                *self.surver_selected_file.borrow_mut() = None;
                #[cfg(not(target_arch = "wasm32"))]
                self.load_from_file(filename, load_options).ok();
                #[cfg(target_arch = "wasm32")]
                error!("Cannot load file from path in WASM");
            }
            Message::LoadWaveformFileFromUrl(url, load_options) => {
                self.user.selected_server_file_index = None;
                *self.surver_selected_file.borrow_mut() = None;
                // If we provide URL at command line and it is a Surver URL, we want to force a switch
                self.load_wave_from_url(url, load_options, true, None);
            }
            Message::LoadFromData(data, load_options) => {
                self.user.selected_server_file_index = None;
                *self.surver_selected_file.borrow_mut() = None;
                self.load_from_data(data, load_options).ok();
            }
            #[cfg(feature = "python")]
            Message::LoadPythonTranslator(filename) => {
                try_log_error!(
                    self.translators.load_python_translator(filename),
                    "Error loading Python translator",
                )
            }
            #[cfg(all(not(target_arch = "wasm32"), feature = "wasm_plugins"))]
            Message::LoadWasmTranslator(path) => {
                let sender = self.channels.msg_sender.clone();
                let max_memory_mib = self.user.config.plugin.max_memory_mib;
                perform_work(move || {
                    match PluginTranslator::new(path.into_std_path_buf(), max_memory_mib) {
                        Ok(t) => {
                            checked_send(&sender, Message::TranslatorLoaded(Arc::new(t)));
                        }
                        Err(e) => {
                            error!("Failed to load wasm translator {e:#}");
                        }
                    }
                });
            }
            Message::LoadCommandFile(path) => {
                self.add_batch_commands(read_command_file(&path));
            }
            Message::LoadCommandFileFromUrl(url) => {
                self.load_commands_from_url(url);
            }
            Message::LoadCommandFromData(bytes) => {
                self.add_batch_commands(read_command_bytes(bytes));
            }
            Message::ExecuteBatchCommand { line, command } => {
                let target = self.user.workspace.command_target();
                match crate::fzcmd::parse_command(
                    &command,
                    command_parser::get_parser(self, target),
                ) {
                    Ok(message) => {
                        self.update(message);
                    }
                    Err(error) => {
                        error!("Error on batch commands line {line}: {error:#?}");
                    }
                }
            }
            Message::SetupCxxrtl(kind) => self.connect_to_cxxrtl(kind, false),
            Message::SetSurverStatus(_start, server, status) => {
                self.user.surver_file_infos = Some(status.file_infos.clone());
                info!(
                    "Received surfer server status from {server}. {} files available.",
                    status.file_infos.len()
                );
                self.user.surver_url = Some(server.clone());
                if status.file_infos.is_empty() {
                    warn!("Received surfer server status with no file infos");
                    return None;
                }
                if self.user.selected_server_file_index.is_none() {
                    if status.file_infos.len() == 1 {
                        // if only one file is available, select it automatically
                        info!(
                            "Only one file available on server {}, loading it automatically",
                            server
                        );
                        self.load_wave_from_url(server.clone(), LoadOptions::Clear, false, Some(0));
                    } else {
                        // if no file is selected, show the server file selection window
                        self.user.show_server_file_window = true;
                        self.progress_tracker = None;
                    }
                }

                if let Some(file_index) = self.user.selected_server_file_index {
                    if file_index >= status.file_infos.len() {
                        warn!(
                            "Selected server file index {file_index} is out of bounds ({} files available)",
                            status.file_infos.len()
                        );
                        return None;
                    }
                    self.server_status_to_progress(&server, &status.file_infos[file_index]);
                }
            }
            Message::FileDropped(dropped_file) => {
                self.load_from_dropped(dropped_file)
                    .map_err(|e| error!("{e:#?}"))
                    .ok();
            }
            Message::DroppedFileBytesLoaded(path, bytes) => {
                self.load_from_dropped_bytes(path, bytes)
                    .map_err(|e| error!("{e:#?}"))
                    .ok();
            }
            Message::StopProgressTracker => {
                self.progress_tracker = None;
            }
            Message::WaveHeaderLoaded(start, source, load_options, header) => {
                // for files using the `wellen` backend, we load the header before parsing the body
                info!(
                    "Loaded the hierarchy and meta-data of {source} in {:?}",
                    start.elapsed()
                );
                match header {
                    #[cfg(not(target_arch = "wasm32"))]
                    HeaderResult::Vtr(loaded) => {
                        let mut new_waves = WaveContainer::new_waveform(Arc::new(loaded.hierarchy));
                        new_waves.attach_source_index(loaded.source_index);
                        self.pending_document = Some(crate::wave_source::PendingDocument {
                            source: source.clone(),
                            format: WaveFormat::Vtr,
                            waves: new_waves,
                            transactions: loaded.transactions,
                            options: load_options,
                        });
                        self.update(Message::WaveBodyLoaded(
                            start,
                            source,
                            crate::wellen::BodyResult::Vtr(loaded.body),
                        ));
                    }
                    HeaderResult::LocalFile(header) => {
                        // Stage the hierarchy until its matching body succeeds.
                        let shared_hierarchy = Arc::new(header.hierarchy);
                        let new_waves = WaveContainer::new_waveform(shared_hierarchy.clone());
                        self.pending_document = Some(crate::wave_source::PendingDocument {
                            source: source.clone(),
                            format: convert_format(header.file_format),
                            waves: new_waves,
                            transactions: None,
                            options: load_options,
                        });
                        // start parsing of the body
                        self.load_wave_body(source, header.body, header.body_len, shared_hierarchy);
                    }
                    HeaderResult::LocalBytes(header) => {
                        // Stage the hierarchy until its matching body succeeds.
                        let shared_hierarchy = Arc::new(header.hierarchy);
                        let new_waves = WaveContainer::new_waveform(shared_hierarchy.clone());
                        self.pending_document = Some(crate::wave_source::PendingDocument {
                            source: source.clone(),
                            format: convert_format(header.file_format),
                            waves: new_waves,
                            transactions: None,
                            options: load_options,
                        });
                        // start parsing of the body
                        self.load_wave_body(source, header.body, header.body_len, shared_hierarchy);
                    }
                    HeaderResult::Remote(hierarchy, file_format, server, file_index) => {
                        // Stage the hierarchy until its matching body succeeds.
                        let new_waves = WaveContainer::new_remote_waveform(
                            &server,
                            hierarchy.clone(),
                            file_index,
                        );
                        self.pending_document = Some(crate::wave_source::PendingDocument {
                            source: source.clone(),
                            format: convert_format(file_format),
                            waves: new_waves,
                            transactions: None,
                            options: load_options,
                        });
                        // body is already being parsed on the server, we need to request the time table though
                        get_time_table_from_server(
                            self.channels.msg_sender.clone(),
                            server,
                            file_index,
                            self.document_load_request,
                        );
                    }
                }
            }
            Message::WaveBodyLoaded(start, source, body) => {
                // for files using the `wellen` backend, parse the body in a second step
                info!("Loaded the body of {source} in {:?}", start.elapsed());
                self.progress_tracker = None;
                let mut pending = self.pending_document.take()?;
                if pending.source != source {
                    self.pending_document = Some(pending);
                    return None;
                }
                let maybe_cmd = pending
                    .waves
                    .wellen_add_body(body)
                    .map_err(|error| error!("Failed to attach waveform body: {error:?}"))
                    .ok()?;
                let param_cmd = pending
                    .waves
                    .load_parameters()
                    .map_err(|error| error!("Failed to request waveform parameters: {error:?}"))
                    .ok()?;
                self.on_waves_loaded(
                    pending.source,
                    pending.format,
                    pending.waves,
                    pending.transactions,
                    pending.options,
                );

                if self.wcp_greeted_signal.load(Ordering::Relaxed)
                    && self.wcp_client_capabilities.waveforms_loaded
                {
                    let source = match source {
                        WaveSource::File(path) => path.to_string(),
                        WaveSource::Url(url) => url,
                        _ => String::new(),
                    };
                    self.channels.wcp_s2c_sender.as_ref().map(|ch| {
                        block_on(
                            ch.send(WcpSCMessage::event(WcpEvent::waveforms_loaded { source })),
                        )
                    });
                }

                // make sure we redraw
                self.invalidate_draw_commands();
                // start loading parameters
                if let Some(cmd) = param_cmd {
                    self.load_variables(cmd);
                }
                // start loading variables
                if let Some(cmd) = maybe_cmd {
                    self.load_variables(cmd);
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            Message::NativeTransactionsLoaded(result) => {
                if let Some(transactions) = self
                    .user
                    .waves
                    .as_mut()
                    .and_then(|w| w.inner.as_transactions_mut())
                    && transactions.on_native_transactions_loaded(result)
                {
                    self.user.workspace.refresh_transaction_rows(transactions);
                    self.invalidate_draw_commands();
                }
                if self.pending_document.is_none()
                    && self
                        .user
                        .waves
                        .as_ref()
                        .is_some_and(|w| w.inner.is_fully_loaded())
                    && self.progress_tracker.as_ref().is_some_and(|progress| {
                        matches!(
                            progress.progress,
                            crate::wave_source::LoadProgressStatus::LoadingVariables(_)
                        )
                    })
                {
                    self.progress_tracker = None;
                }
            }
            Message::SignalsLoaded(start, res) => {
                info!("Loaded {} variables in {:?}", res.len(), start.elapsed());
                let waves = self.user.waves.as_mut()?;
                match waves.inner.as_waves_mut()?.on_signals_loaded(res) {
                    Err(err) => error!("{err:?}"),
                    Ok(Some(cmd)) => self.load_variables(cmd),
                    _ => {}
                }
                if self.pending_document.is_none()
                    && self
                        .user
                        .waves
                        .as_ref()
                        .is_some_and(|waves| waves.inner.is_fully_loaded())
                    && self.progress_tracker.as_ref().is_some_and(|progress| {
                        matches!(
                            progress.progress,
                            crate::wave_source::LoadProgressStatus::LoadingVariables(_)
                        )
                    })
                {
                    self.progress_tracker = None;
                }
                // make sure we redraw since now more variable data is available
                self.invalidate_draw_commands();
            }
            Message::WavesLoaded(filename, format, new_waves, load_options) => {
                self.on_waves_loaded(filename, format, *new_waves, None, load_options);
                // here, the body and thus the number of timestamps is already loaded!
                let enable_time_offset = self.enable_time_offset();
                let waves = self
                    .user
                    .waves
                    .as_mut()
                    .expect("Waves should be loaded at this point!");
                waves.refresh_time_range(enable_time_offset);
                self.user.workspace.update_viewports(waves);
                self.progress_tracker = None;
            }
            Message::TransactionStreamsLoaded(filename, format, new_ftr, loaded_options) => {
                self.on_transaction_streams_loaded(filename, format, new_ftr, loaded_options);
                let enable_time_offset = self.enable_time_offset();
                let waves = self
                    .user
                    .waves
                    .as_mut()
                    .expect("Waves should be loaded at this point!");
                waves.refresh_time_range(enable_time_offset);
                self.user.workspace.update_viewports(waves);
            }
            Message::BlacklistTranslator(idx, translator) => {
                self.user.blacklisted_translators.insert((idx, translator));
            }
            Message::TranslatorLoaded(t) => {
                info!("Translator {} loaded", t.name());
                t.set_wave_source(
                    self.user
                        .waves
                        .as_ref()
                        .map(|waves| waves.source.into_translation_type()),
                );

                self.translators.add_or_replace(AnyTranslator::Full(t));
            }
            Message::SetSidePanelVisible(v) => self.user.show_hierarchy = Some(v),
            Message::SetMenuVisible(v) => self.user.show_menu = Some(v),
            Message::ToggleMenu => {
                self.user.show_menu = Some(!self.show_menu());
            }
            Message::SetToolbarVisible(v) => self.user.show_toolbar = Some(v),
            Message::SetToolbarGroupEnabled(group_id, enabled) => {
                self.user
                    .toolbar_group_enabled
                    .insert(group_id, Some(enabled));
            }
            Message::SetToolbarGroupRow(group_id, row) => {
                self.set_toolbar_group_row(&group_id, row);
            }
            Message::SetShowEmptyScopes(v) => self.user.show_empty_scopes = Some(v),
            Message::SetShowHierarchyIcons(v) => self.user.show_hierarchy_icons = Some(v),
            Message::SetParameterDisplayLocation(location) => {
                self.user.parameter_display_location = Some(location);
            }
            Message::SetStatusbarVisible(v) => self.user.show_statusbar = Some(v),
            Message::SetTickLines(v) => self.user.show_ticks = Some(v),
            Message::SetVariableTooltip(v) => self.user.show_tooltip = Some(v),
            Message::SetScopeTooltip(v) => self.user.show_scope_tooltip = Some(v),
            Message::SetOverviewVisible(v) => self.user.show_overview = Some(v),
            Message::SetShowVariableDirection(v) => self.user.show_variable_direction = Some(v),
            Message::SetTransitionValue(v) => self.user.transition_value = Some(v),
            Message::SetDrawVectorUnknownsAsLine(v) => {
                self.user.draw_vector_unknowns_as_line = Some(v);
            }
            Message::SetFocusHighlight(focus_highlight) => {
                self.user.focus_highlight = Some(focus_highlight);
            }
            Message::SetShowIndices(v) => {
                let new = v;
                self.user.show_variable_indices = Some(new);
                let waves = self.user.waveform_edit()?;
                waves.document.display_variable_indices = new;
                waves.items.compute_variable_display_names(
                    &waves.document.inner,
                    waves.document.display_variable_indices,
                );
            }
            Message::HideCommandPrompt => {
                self.restore_prompt_theme();
                *self.command_prompt_text.borrow_mut() = String::new();
                self.command_prompt.suggestions = vec![];
                self.command_prompt.selected = self.command_prompt.previous_commands.len();
                self.command_prompt.visible = false;
            }
            Message::ShowCommandPrompt(text, selected) => {
                if !self.command_prompt.visible {
                    // Capture the target once; it stays fixed while the prompt is open.
                    self.command_prompt.target = self.user.workspace.command_target();
                }
                self.command_prompt.new_text = Some((text, selected.unwrap_or(String::new())));
                self.command_prompt.visible = true;
            }
            Message::FileDownloaded(url, bytes, load_options) => {
                self.load_from_bytes(WaveSource::Url(url), bytes.to_vec(), load_options);
            }
            Message::CommandFileDownloaded(_url, bytes) => {
                self.add_batch_commands(read_command_bytes(bytes.to_vec()));
                self.progress_tracker = None;
            }
            Message::SetConfigFromString(s) => {
                // FIXME think about a structured way to collect errors
                let config = SurferConfig::new_from_toml(&s)
                    .with_context(|| "Failed to load config file")
                    .ok()?;

                self.user.config = config;

                // Refresh time offset cache when config changes
                let enable_time_offset = self.enable_time_offset();
                if let Some(waves) = &mut self.user.waves {
                    waves.refresh_time_range(enable_time_offset);
                }

                self.apply_theme_visuals();
                self.invalidate_draw_commands();
            }
            Message::ReloadConfig => {
                // FIXME think about a structured way to collect errors
                let config = SurferConfig::new(false)
                    .with_context(|| "Failed to load config file")
                    .ok()?;
                self.translators = all_translators();
                self.user.config = config;

                // Refresh time offset cache when config changes
                let enable_time_offset = self.enable_time_offset();
                if let Some(waves) = &mut self.user.waves {
                    waves.refresh_time_range(enable_time_offset);
                }

                self.apply_theme_visuals();
                self.invalidate_draw_commands();
            }
            Message::ReloadWaveform(keep_unavailable) => {
                let waves = self.user.waveform_read()?;
                let options = if keep_unavailable {
                    LoadOptions::KeepAll
                } else {
                    LoadOptions::KeepAvailable
                };
                match &waves.document.source {
                    WaveSource::File(filename) => {
                        self.load_from_file(filename.clone(), options).ok();
                    }
                    WaveSource::Data => {}       // can't reload
                    WaveSource::Cxxrtl(..) => {} // can't reload
                    WaveSource::DragAndDrop(filename) => {
                        filename
                            .clone()
                            .and_then(|filename| self.load_from_file(filename, options).ok());
                    }
                    WaveSource::Url(url) => {
                        self.load_wave_from_url(
                            url.clone(),
                            options,
                            false,
                            self.user.selected_server_file_index,
                        );
                    }
                }

                for translator in self.translators.all_translators() {
                    translator.reload(self.channels.msg_sender.clone());
                }
                self.variable_name_info_cache.borrow_mut().clear();
                self.translator_generation += 1;

                if let Some(document) = self.user.waves.as_ref() {
                    self.user.workspace.recompute_display_names(document);
                }
            }
            Message::SuggestReloadWaveform => match self.autoreload_files() {
                AutoLoad::Always => self.update(Message::ReloadWaveform(true))?,
                AutoLoad::Never => (),
                AutoLoad::Ask => {
                    self.user.show_reload_suggestion = Some(ReloadWaveformDialog::default());
                }
            },
            Message::CloseReloadWaveformDialog {
                reload_file,
                do_not_show_again,
            } => {
                if do_not_show_again {
                    // FIXME: This is currently saved in state, but could be persisted in
                    // some setting.
                    self.user.autoreload_files = Some(AutoLoad::from_bool(reload_file));
                }
                self.user.show_reload_suggestion = None;
                if reload_file {
                    self.update(Message::ReloadWaveform(true));
                }
            }
            Message::UpdateReloadWaveformDialog(dialog) => {
                self.user.show_reload_suggestion = Some(dialog);
            }
            Message::OpenSiblingStateFile(open) => {
                if !open {
                    return None;
                }
                let waves = self.user.waveform_read()?;
                let state_file_path = waves.document.source.sibling_state_file()?;
                self.load_state_file(Some(state_file_path.clone()));
            }
            Message::SuggestOpenSiblingStateFile => match self.autoload_sibling_state_files() {
                AutoLoad::Always => {
                    self.update(Message::OpenSiblingStateFile(true));
                }
                AutoLoad::Never => {}
                AutoLoad::Ask => {
                    self.user.show_open_sibling_state_file_suggestion =
                        Some(OpenSiblingStateFileDialog::default());
                }
            },
            Message::CloseOpenSiblingStateFileDialog {
                load_state,
                do_not_show_again,
            } => {
                if do_not_show_again {
                    self.user.autoload_sibling_state_files = Some(AutoLoad::from_bool(load_state));
                }
                self.user.show_open_sibling_state_file_suggestion = None;
                if load_state {
                    self.update(Message::OpenSiblingStateFile(true));
                }
            }
            Message::UpdateOpenSiblingStateFileDialog(dialog) => {
                self.user.show_open_sibling_state_file_suggestion = Some(dialog);
            }
            Message::RemovePlaceholders => {
                let mut waves = self.user.waveform_edit()?;
                waves.remove_placeholders();
            }
            Message::SetClockHighlightType(new_type) => {
                self.user.clock_highlight_type = Some(new_type);
                self.invalidate_draw_commands();
            }
            Message::SetFillHighValues(fill) => self.user.fill_high_values = Some(fill),
            Message::SetTraceStyle(trace_style) => {
                self.user.trace_style = Some(trace_style);
                self.invalidate_draw_commands();
            }
            Message::ResolveMarkerSet { name, time } => {
                let marker_id = self.user.waveform_read()?.items.resolve_marker_name(&name);
                let msg = match marker_id {
                    Some(id) => Message::SetMarker { id, time },
                    None => Message::AddMarker {
                        time,
                        name: Some(name),
                        move_focus: true,
                    },
                };
                self.update(msg);
            }
            Message::AddMarker {
                time,
                name,
                move_focus,
            } => {
                let label = match &name {
                    Some(name) => format!("Add marker {name} at {time}"),
                    None => format!("Add marker at {time}"),
                };
                self.edit_shared_marker(label, None, |waves| {
                    waves.add_marker(&time, name, move_focus).map(|_| ())
                })?;
            }
            Message::SetMarker { id, time } => {
                self.edit_shared_marker(format!("Set marker {id} to {time}"), Some(id), |waves| {
                    waves.set_marker_position(id, &time).ok()
                })?;
            }
            Message::ResolveMarkerRemove(name) => {
                if let Some(id) = self.user.waveform_read()?.items.resolve_marker_name(&name) {
                    self.update(Message::RemoveMarker(id));
                }
            }
            Message::RemoveMarker(id) => {
                self.remove_shared_marker(id)?;
            }
            Message::MoveMarkerToCursor(idx) => {
                self.edit_shared_marker(
                    format!("Move marker {idx} to cursor"),
                    Some(idx),
                    |waves| waves.move_marker_to_cursor(idx).ok(),
                )?;
            }
            Message::GoToCursorIfNotInView => {
                let mut waves = self.user.waveform_edit()?;
                if waves.go_to_cursor_if_not_in_view() {
                    self.invalidate_draw_commands();
                }
            }
            Message::GoToMarkerPosition(idx, tile_id) => {
                let waves = self.user.waveform_edit_at(tile_id)?;
                // If there are no timestamps, the file is not fully loaded
                if waves.max_timestamp().is_some() {
                    let cursor = waves.document.markers.get(&idx)?.clone();
                    let range = waves.time_range().clone();
                    waves.view.viewport.go_to_time(&cursor, &range);
                    self.invalidate_draw_commands();
                } else {
                    warn!(
                        "Go to marker position: No timestamps count, even though waveforms should be loaded"
                    );
                }
            }
            Message::ChangeVariableNameType(target, name_type) => {
                let waves = self.user.waveform_edit()?;
                let recompute_names = waves.items.change_variable_name_type(
                    target,
                    name_type,
                    waves.view.focused_index(waves.items),
                );

                if recompute_names {
                    waves.items.compute_variable_display_names(
                        &waves.document.inner,
                        waves.document.display_variable_indices,
                    );
                }
            }
            Message::ForceVariableNameTypes(name_type) => {
                let waves = self.user.waveform_edit()?;
                waves.items.force_variable_name_type(
                    name_type,
                    &waves.document.inner,
                    waves.document.display_variable_indices,
                );
            }
            Message::CommandPromptClear => {
                self.restore_prompt_theme();
                *self.command_prompt_text.borrow_mut() = String::new();
                self.command_prompt.suggestions = vec![];
                // self.command_prompt.selected = self.command_prompt.previous_commands.len();
                self.command_prompt.selected = if self.command_prompt_text.borrow().is_empty() {
                    self.command_prompt.previous_commands.len().clamp(0, 3)
                } else {
                    0
                };
            }
            Message::CommandPromptUpdate { suggestions } => {
                self.command_prompt.suggestions = suggestions;
                self.command_prompt.selected = if self.command_prompt_text.borrow().is_empty() {
                    self.command_prompt.previous_commands.len().clamp(0, 3)
                } else {
                    0
                };
                self.command_prompt.new_selection =
                    Some(if self.command_prompt_text.borrow().is_empty() {
                        self.command_prompt.previous_commands.len().clamp(0, 3)
                    } else {
                        0
                    });
                self.preview_prompt_theme();
            }
            Message::CommandPromptPushPrevious(cmd) => {
                let len = cmd.len();
                self.command_prompt
                    .previous_commands
                    .insert(0, (cmd, vec![false; len]));
            }
            Message::OpenFileDialog(mode) => {
                self.open_file_dialog(mode);
            }
            Message::OpenCommandFileDialog => {
                self.open_command_file_dialog();
            }
            #[cfg(feature = "python")]
            Message::OpenPythonPluginDialog => {
                self.open_python_file_dialog();
            }
            #[cfg(feature = "python")]
            Message::ReloadPythonPlugin => {
                try_log_error!(
                    self.translators.reload_python_translator(),
                    "Error reloading Python translator"
                );
                self.translator_generation += 1;
                self.invalidate_draw_commands();
            }
            Message::SaveStateFile(path) => self.save_state_file(path),
            #[cfg(not(target_arch = "wasm32"))]
            Message::ExportSignalsToFst(path) => self.export_signals_to_fst(path),
            Message::LoadStateFromData(bytes) => self.load_state_from_bytes(&bytes),
            Message::LoadStateFile(path) => self.load_state_file(path),
            Message::LoadState(state, path) => self.load_state(state, path),
            Message::SetStateFile(path) => {
                // since in wasm we can't support "save", only "save as" - never set the `state_file`
                #[cfg(not(target_arch = "wasm32"))]
                {
                    self.user.state_file = Some(path);
                }
                #[cfg(target_arch = "wasm32")]
                {
                    error!("Failed to load {path:?}. Loading state files is unsupported on wasm")
                }
            }
            Message::SetAboutVisible(s) => self.user.show_about = s,
            Message::SetKeyHelpVisible(s) => self.user.show_keys = s,
            Message::SetGestureHelpVisible(s) => self.user.show_gestures = s,
            Message::SetUrlEntryVisible(s, f) => {
                self.user.show_url_entry = s;
                self.url_callback = f;
            }
            Message::SetLicenseVisible(s) => self.user.show_license = s,
            Message::SetQuickStartVisible(s) => self.user.show_quick_start = s,
            Message::SetPerformanceVisible(s) => {
                if !s {
                    self.continuous_redraw = false;
                }
                self.user.show_performance = s;
            }
            Message::SetContinuousRedraw(s) => self.continuous_redraw = s,
            Message::SetMouseGestureDragStart(pos, time, tile_id) => {
                let interaction = &mut self.user.waveform_edit_at(tile_id)?.view.interaction;
                interaction.gesture_start_location = pos;
                interaction.gesture_start_time = time;
            }
            Message::SetMeasureDragStart(pos, tile_id) => {
                self.user
                    .waveform_edit_at(tile_id)?
                    .view
                    .interaction
                    .measure_start_location = pos;
            }
            Message::SetTextEditFocused(id, s) => {
                self.text_edit_focused.insert(id, s);
            }
            Message::SetRequestTextEditFocus(id, s) => {
                self.text_edit_request_focus.insert(id, s);
            }
            Message::ClearAllTextEditFocuses => {
                self.text_edit_focused.clear();
                self.text_edit_request_focus.clear();
            }
            Message::SetVariableNameFilterType(variable_name_filter_type) => {
                self.user.variable_filter.name_filter_type = variable_name_filter_type;
            }
            Message::SetVariableNameFilterCaseInsensitive(s) => {
                self.user.variable_filter.name_filter_case_insensitive = s;
            }
            Message::SetVariableIOFilter(t, b) => match t {
                VariableIOFilterType::Output => self.user.variable_filter.include_outputs = b,
                VariableIOFilterType::Input => self.user.variable_filter.include_inputs = b,
                VariableIOFilterType::InOut => self.user.variable_filter.include_inouts = b,
                VariableIOFilterType::Other => self.user.variable_filter.include_others = b,
            },
            Message::SetVariableGroupByDirection(b) => {
                self.user.variable_filter.group_by_direction = b;
            }
            Message::SetUIZoomFactor(scale) => {
                if let Some(ctx) = &mut self.context.as_ref() {
                    ctx.set_zoom_factor(scale);
                }
                self.user.ui_zoom_factor = Some(scale);
            }
            Message::SelectPrevCommand => {
                self.command_prompt.new_selection = Some(
                    self.command_prompt
                        .new_selection
                        .unwrap_or(self.command_prompt.selected)
                        .saturating_sub(1),
                );
                self.preview_prompt_theme();
            }
            Message::SelectNextCommand => {
                self.command_prompt.new_selection = Some(
                    self.command_prompt
                        .new_selection
                        .unwrap_or(self.command_prompt.selected)
                        .saturating_add(1)
                        .min(self.command_prompt.suggestions.len().saturating_sub(1)),
                );
                self.preview_prompt_theme();
            }
            Message::SetHierarchyStyle(style) => self.user.hierarchy_style = Some(style),
            Message::SetArrowKeyBindings(bindings) => {
                self.user.arrow_key_bindings = Some(bindings);
            }
            Message::SetPrimaryMouseDragBehavior(behavior) => {
                self.user.primary_button_drag_behavior = Some(behavior);
            }
            Message::SetTimeOffsetEnabled(enabled) => {
                self.user.enable_time_offset = Some(enabled);
                let enable_time_offset = self.enable_time_offset();
                if let Some(waves) = &mut self.user.waves {
                    waves.refresh_time_range(enable_time_offset);
                    self.user.workspace.update_viewports(waves);
                }
                self.invalidate_draw_commands();
            }
            Message::InvalidateDrawCommands => self.invalidate_draw_commands(),
            Message::UnpauseSimulation => {
                self.user
                    .waves
                    .as_ref()?
                    .inner
                    .as_waves()?
                    .unpause_simulation();
            }
            Message::PauseSimulation => {
                self.user
                    .waves
                    .as_ref()?
                    .inner
                    .as_waves()?
                    .pause_simulation();
            }
            Message::Batch(messages) => {
                for message in messages {
                    self.update(message);
                }
            }
            Message::AddDraggedVariables {
                tile_id,
                variables,
                position,
            } => {
                let waves = self.user.waveform_read_at(tile_id)?;
                if variables.is_empty() || waves.inner.as_waves().is_none() {
                    return None;
                }
                // Validate insertion before loading signals or recording history.
                let mut candidate = waves.items.items_tree.clone();
                candidate
                    .insert_item(
                        crate::displayed_item::DisplayedItemRef(usize::MAX),
                        position,
                    )
                    .ok()?;
                self.save_current_canvas("Add dragged variables".into());
                let mut waves = self.user.waveform_edit_at(tile_id)?;
                if let (Some(cmd), _) = waves.add_variables(
                    &self.translators,
                    variables,
                    Some(position),
                    true,
                    false,
                    None,
                    false,
                ) {
                    self.load_variables(cmd);
                }
                self.invalidate_draw_commands();
            }
            Message::MoveDraggedItems {
                tile_id,
                items,
                position,
            } => {
                let waves = self.user.waveform_read_at(tile_id)?;
                if items.is_empty() {
                    return None;
                }
                let to_move = items
                    .iter()
                    .map(|id| {
                        waves
                            .items
                            .items_tree
                            .iter()
                            .position(|node| node.item_ref == *id)
                            .map(ItemIndex)
                    })
                    .collect::<Option<Vec<_>>>()?;
                let mut candidate = waves.items.items_tree.clone();
                candidate.move_items(to_move, position).ok()?;
                if candidate.iter().eq(waves.items.items_tree.iter()) {
                    return None;
                }
                self.save_current_canvas("Drag item".into());
                self.user.waveform_edit_at(tile_id)?.items.items_tree = candidate;
                self.invalidate_draw_commands();
            }
            Message::VariableValueToClipbord(vidx) => {
                self.handle_variable_clipboard_operation(
                    vidx,
                    |waves, item_ref: DisplayedItemRef| {
                        if let Some(DisplayedItem::Variable(_)) =
                            waves.items.displayed_items.get(&item_ref)
                        {
                            let field_ref = item_ref.into();
                            self.waveform_services().get_variable_value(
                                waves.document,
                                waves.items,
                                &field_ref,
                                waves
                                    .document
                                    .cursor
                                    .as_ref()
                                    .and_then(num::BigInt::to_biguint)
                                    .as_ref(),
                            )
                        } else {
                            None
                        }
                    },
                );
            }
            Message::VariableNameToClipboard(vidx) => {
                self.handle_variable_clipboard_operation(
                    vidx,
                    |waves, item_ref: DisplayedItemRef| {
                        if let Some(DisplayedItem::Variable(variable)) =
                            waves.items.displayed_items.get(&item_ref)
                        {
                            Some(variable.variable_ref.name.clone())
                        } else {
                            None
                        }
                    },
                );
            }
            Message::VariableFullNameToClipboard(vidx) => {
                self.handle_variable_clipboard_operation(
                    vidx,
                    |waves, item_ref: DisplayedItemRef| {
                        if let Some(DisplayedItem::Variable(variable)) =
                            waves.items.displayed_items.get(&item_ref)
                        {
                            Some(variable.variable_ref.full_path_string())
                        } else {
                            None
                        }
                    },
                );
            }
            Message::SetViewportStrategy(strategy) => {
                self.user.workspace.set_viewport_strategy(strategy);
            }

            Message::Undo(count) => {
                for _ in 0..count {
                    let Some(previous) = self.undo_stack.pop() else {
                        break;
                    };
                    match self.restore_history(previous, false) {
                        Ok(inverse) => self.redo_stack.push(inverse),
                        Err(previous) => {
                            self.undo_stack.push(previous);
                            break;
                        }
                    }
                }
            }
            Message::Redo(count) => {
                for _ in 0..count {
                    let Some(previous) = self.redo_stack.pop() else {
                        break;
                    };
                    match self.restore_history(previous, true) {
                        Ok(inverse) => self.undo_stack.push(inverse),
                        Err(previous) => {
                            self.redo_stack.push(previous);
                            break;
                        }
                    }
                }
            }
            Message::DumpTree => {
                let waves = self.user.waveform_read()?;
                dump_tree(waves.items);
            }
            Message::GroupNew {
                name,
                before,
                items,
            } => {
                self.save_current_canvas(format!(
                    "Create group {}",
                    name.clone().unwrap_or(String::new())
                ));
                self.invalidate_draw_commands();
                let mut waves = self.user.waveform_edit()?;

                let passed_or_focused = before
                    .and_then(|before| {
                        waves
                            .items
                            .items_tree
                            .get(before)
                            .map(|node| node.level)
                            .map(|level| TargetPosition { before, level })
                    })
                    .or_else(|| {
                        waves
                            .items
                            .insert_position(waves.view.focused_index(waves.items))
                    });
                let final_target =
                    passed_or_focused.unwrap_or_else(|| waves.items.end_insert_position());

                let mut item_refs = items.unwrap_or_else(|| {
                    waves
                        .items
                        .items_tree
                        .iter_visible_selected()
                        .map(|node| node.item_ref)
                        .collect::<Vec<_>>()
                });

                // if we are using the focus as the insert anchor, then move that as well
                let item_refs = if before.is_none() & passed_or_focused.is_some() {
                    let focus_index = waves
                        .items
                        .items_tree
                        .to_displayed(
                            waves
                                .view
                                .focused_index(waves.items)
                                .expect("Inconsistent state"),
                        )
                        .expect("Inconsistent state");
                    item_refs.push(
                        waves
                            .items
                            .items_tree
                            .get(focus_index)
                            .expect("Inconsistent state")
                            .item_ref,
                    );
                    item_refs
                } else {
                    item_refs
                };

                if item_refs.is_empty() {
                    return None;
                }

                let group_ref = waves
                    .add_group(name.unwrap_or("Group".to_owned()), Some(final_target))
                    .ok()?;

                let item_idxs = waves
                    .items
                    .items_tree
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, node)| {
                        item_refs
                            .contains(&node.item_ref)
                            .then_some(crate::displayed_item_tree::ItemIndex(idx))
                    })
                    .collect::<Vec<_>>();

                if let Err(e) = waves.items.items_tree.move_items(
                    item_idxs,
                    crate::displayed_item_tree::TargetPosition {
                        before: ItemIndex(final_target.before.0 + 1),
                        level: final_target.level.saturating_add(1),
                    },
                ) {
                    dump_tree(waves.items);
                    error!("failed to move items into group: {e:?}");
                }
                waves.items.items_tree.xselect_all_visible(false);
                waves.view.focused_item = Some(group_ref);
            }
            Message::GroupDissolve(item_ref) => {
                self.save_current_canvas("Dissolve group".to_owned());
                self.invalidate_draw_commands();
                let waves = self.user.waveform_edit()?;
                let item_index = waves.index_for_ref_or_focus(item_ref)?;

                let removed = waves.items.items_tree.remove_dissolve(item_index);
                waves.items.displayed_items.remove(&removed);
            }
            Message::GroupFold(item_ref)
            | Message::GroupUnfold(item_ref)
            | Message::GroupFoldRecursive(item_ref)
            | Message::GroupUnfoldRecursive(item_ref) => {
                let unfold = matches!(
                    message,
                    Message::GroupUnfold(..) | Message::GroupUnfoldRecursive(..)
                );
                let recursive = matches!(
                    message,
                    Message::GroupFoldRecursive(..) | Message::GroupUnfoldRecursive(..)
                );

                let undo_msg = if unfold {
                    "Unfold group".to_owned()
                } else {
                    "Fold group".to_owned()
                } + &(if recursive {
                    " recursive".to_owned()
                } else {
                    String::new()
                });
                // undo message even if no waves are available
                self.save_current_canvas(undo_msg);
                self.invalidate_draw_commands();

                let waves = self.user.waveform_edit()?;
                let item = waves.index_for_ref_or_focus(item_ref)?;

                if let Some(focused_item) = waves.view.focused_index(waves.items) {
                    let info = waves
                        .items
                        .items_tree
                        .get_visible_extra(focused_item)
                        .expect("Inconsistent state");
                    if waves.items.items_tree.subtree_contains(item, info.idx) {
                        waves.view.focused_item = None;
                    }
                }
                if recursive {
                    waves.items.items_tree.xfold_recursive(item, unfold);
                } else {
                    waves.items.items_tree.xfold(item, unfold);
                }
            }
            Message::GroupFoldAll | Message::GroupUnfoldAll => {
                let unfold = matches!(message, Message::GroupUnfoldAll);
                let undo_msg = if unfold {
                    "Fold all groups".to_owned()
                } else {
                    "Unfold all groups".to_owned()
                };
                self.save_current_canvas(undo_msg);
                self.invalidate_draw_commands();

                let waves = self.user.waveform_edit()?;

                // remove focus if focused item is folded away -> prevent future waveform
                // adds being invisibly inserted
                if let Some(focused_item) = waves.view.focused_index(waves.items) {
                    let focused_level = waves
                        .items
                        .items_tree
                        .get_visible(focused_item)
                        .expect("Inconsistent state")
                        .level;
                    if !unfold && (focused_level > 0) {
                        waves.view.focused_item = None;
                    }
                }
                waves.items.items_tree.xfold_all(unfold);
            }
            #[cfg(target_arch = "wasm32")]
            Message::StartWcpServer { .. } => {
                error!("Wcp is not supported on wasm")
            }
            #[cfg(target_arch = "wasm32")]
            Message::StopWcpServer => {
                error!("Wcp is not supported on wasm")
            }
            #[cfg(not(target_arch = "wasm32"))]
            Message::StartWcpServer { address, initiate } => {
                self.start_wcp_server(address, initiate);
            }
            #[cfg(not(target_arch = "wasm32"))]
            Message::StopWcpServer => {
                self.stop_wcp_server();
            }
            Message::SetupChannelWCP => {
                #[cfg(target_arch = "wasm32")]
                {
                    use futures::executor::block_on;
                    self.channels.wcp_c2s_receiver = block_on(WCP_CS_HANDLER.rx.write()).take();
                    if self.channels.wcp_c2s_receiver.is_none() {
                        error!("Failed to claim wasm tx, was SetupWasmWCP executed twice?");
                    }
                    self.channels.wcp_s2c_sender = Some(WCP_SC_HANDLER.tx.clone());
                }
            }
            Message::BuildAnalogCache {
                display_id,
                cache_key,
            } => {
                let waves = self.user.waveform_edit()?;
                let generation = waves.document.cache_generation;

                // Check if already have valid entry (building or ready)
                let item = waves.items.displayed_items.get(&display_id)?;
                let DisplayedItem::Variable(var) = item else {
                    return None;
                };
                if var
                    .analog
                    .as_ref()?
                    .cache
                    .as_ref()
                    .is_some_and(|e| e.generation == generation && e.cache_key == cache_key)
                {
                    return None;
                }

                // Try to share from in-flight builds first (handles removed-but-still-building case)
                if let Some(entry) = waves.document.inflight_caches.get(&cache_key)
                    && entry.generation == generation
                {
                    if let DisplayedItem::Variable(var) =
                        waves.items.displayed_items.get_mut(&display_id)?
                    {
                        var.analog.as_mut()?.cache = Some(entry.clone());
                    }
                    return None; // Shared from in-flight build
                }

                // Try to share from another displayed variable (O(n) scan - only during cache build)
                let existing = waves
                    .items
                    .displayed_items
                    .values()
                    .filter_map(|item| match item {
                        DisplayedItem::Variable(v) => v.analog.as_ref()?.cache.as_ref(),
                        _ => None,
                    })
                    .find(|e| e.cache_key == cache_key && e.generation == generation)
                    .cloned();

                if let Some(entry) = existing {
                    if let DisplayedItem::Variable(var) =
                        waves.items.displayed_items.get_mut(&display_id)?
                    {
                        var.analog.as_mut()?.cache = Some(entry);
                    }
                    return None; // Shared existing entry (may still be building)
                }

                // Clone variable_ref only when we need to spawn builder
                let variable_ref = match waves.items.displayed_items.get(&display_id)? {
                    DisplayedItem::Variable(v) => v.variable_ref.clone(),
                    _ => return None,
                };

                // Create new entry and spawn builder
                let entry = std::sync::Arc::new(crate::analog_signal_cache::AnalogCacheEntry::new(
                    cache_key.clone(),
                    generation,
                ));

                if let DisplayedItem::Variable(var) =
                    waves.items.displayed_items.get_mut(&display_id)?
                {
                    var.analog.as_mut()?.cache = Some(entry.clone());
                }

                let translator = self.translators.clone_translator(&cache_key.1);

                // Track in-flight build for sharing with other variables
                waves
                    .document
                    .inflight_caches
                    .insert(cache_key.clone(), entry.clone());

                waves.build_analog_cache_async(
                    entry,
                    &variable_ref,
                    translator,
                    &self.channels.msg_sender,
                );
            }
            Message::AnalogCacheBuilt { entry, result } => {
                OUTSTANDING_TRANSACTIONS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                // Remove from in-flight registry (may already be gone if generation changed)
                if let Some(waves) = self.user.waves.as_mut()
                    && waves
                        .inflight_caches
                        .get(&entry.cache_key)
                        .is_some_and(|current| Arc::ptr_eq(current, &entry))
                {
                    waves.inflight_caches.remove(&entry.cache_key);
                }
                match result {
                    Ok(cache) => {
                        entry.set(cache);
                    }
                    Err(err) => {
                        warn!("Failed to build analog cache: {err}");
                    }
                }
                self.invalidate_draw_commands();
            }
            Message::Exit | Message::ToggleFullscreen => {} // Handled in eframe::update
            Message::SelectTheme(theme_name) => {
                let theme = SurferTheme::new(theme_name)
                    .with_context(|| "Failed to set theme")
                    .ok()?;
                // An explicit choice commits any command-prompt preview. UI messages
                // are popped in reverse order, so prompt cleanup can follow this.
                self.command_prompt.original_theme = None;
                self.user.config.theme = theme;
                self.apply_theme_visuals();
                self.invalidate_draw_commands();
            }
            Message::EnableAnimations(enable) => {
                let ctx = self.context.as_ref()?;
                self.user.animation_enabled = Some(enable);
                ctx.all_styles_mut(|style| {
                    style.animation_time = if self.animation_enabled() {
                        self.user.config.animation_time
                    } else {
                        0.0
                    };
                });
            }
            Message::ShowDividerText(show) => {
                self.user.config.show_divider_text = show;
            }
            Message::AsyncDone(_) => (),
            Message::AddGraphic(id, g) => {
                let target = self
                    .user
                    .workspace
                    .resolve_waveform(crate::tiles::TileTarget::Focused)?;
                self.update(Message::ToTile(
                    target,
                    crate::tiles::kind::TileMessage::Waveform(
                        crate::tile_kinds::waveform::WaveformMessage::AddGraphic(id, g),
                    ),
                ))?;
            }
            Message::RemoveGraphic(id) => {
                let target = self
                    .user
                    .workspace
                    .resolve_waveform(crate::tiles::TileTarget::Focused)?;
                self.update(Message::ToTile(
                    target,
                    crate::tiles::kind::TileMessage::Waveform(
                        crate::tile_kinds::waveform::WaveformMessage::RemoveGraphic(id),
                    ),
                ))?;
            }
            Message::ExpandDrawnItem { item, levels } => {
                self.invalidate_draw_commands();
                let waves = self.user.waveform_edit()?;
                if let Some(DisplayedItem::Variable(var)) =
                    waves.items.displayed_items.get_mut(&item)
                {
                    var.unfolded_fields = Self::expand_levels_to_unfolded_fields(&var.info, levels);
                }
            }
            Message::ToggleVariableFieldFold(item, field) => {
                self.invalidate_draw_commands();
                let waves = self.user.waveform_edit()?;
                if let Some(DisplayedItem::Variable(var)) =
                    waves.items.displayed_items.get_mut(&item)
                {
                    var.toggle_field_fold(field);
                }
            }
            Message::SetMouseGestureAnnotation(annotation_kind, tile_id) => {
                self.user
                    .waveform_edit_at(tile_id)?
                    .view
                    .interaction
                    .annotation_kind = annotation_kind;
            }
            Message::RectangleAdded {
                time_at_start,
                time_at_end,
                wave_from,
                wave_to,
                rect,
            } => {
                let id = self.annotation_id();
                self.save_current_canvas(format!("Add rectangle {id:?}"));
                let waves = self.user.waveform_edit()?;
                waves.items.annotation_counter += 1;

                let new_rect = Annotation::Rect(RectAnnotation::new(
                    id,
                    time_at_start,
                    time_at_end,
                    wave_from,
                    wave_to,
                    rect,
                    waves.items.annotation_counter,
                ));
                let new_id = new_rect.get_id();
                waves.items.annotations.push(new_rect);

                waves
                    .items
                    .add_annotation_to_group(DEFAULT_GROUP_NAME, new_id);
            }
            Message::ArrowAdded {
                wave_point_from,
                wave_point_to,
                head_mode,
            } => {
                let id = self.annotation_id();
                self.save_current_canvas(format!("Add arrow {id:?}"));
                let waves = self.user.waveform_edit()?;
                waves.items.annotation_counter += 1;
                let new_arrow = Annotation::Arrow(ArrowAnnotation::new(
                    id,
                    wave_point_from,
                    wave_point_to,
                    head_mode,
                    waves.items.annotation_counter,
                ));
                let new_id = new_arrow.get_id();
                waves.items.annotations.push(new_arrow);

                waves
                    .items
                    .add_annotation_to_group(DEFAULT_GROUP_NAME, new_id);
            }

            Message::RemoveAnnotation(anno_id) => {
                self.save_current_canvas(format!("Removed annotation {anno_id:?}"));
                let mut waves = self.user.waveform_edit()?;
                waves.items.delete_annotation(anno_id);
                for view in std::iter::once(&mut *waves.view)
                    .chain(waves.peers.iter_mut().map(|view| &mut **view))
                {
                    if view.selected_annotation == Some(anno_id) {
                        view.selected_annotation = None;
                        view.annotation_menu = None;
                    }
                }
                waves.items.remove_annotation_from_group(anno_id);
            }

            Message::ToggleAnnotationVisiblility(anno_id) => {
                self.save_current_canvas(format!("Changed visibility on {anno_id:?}"));
                let waves = self.user.waveform_edit()?;
                if let Some(target) = waves
                    .items
                    .annotations
                    .iter_mut()
                    .find(|a| a.get_id() == anno_id)
                {
                    target.set_visibility(!target.is_visible());
                }
            }

            Message::ToggleAnnotationListShowComments(anno_id) => {
                let waves = self.user.waveform_edit()?;
                if let Some(target) = waves
                    .items
                    .annotations
                    .iter_mut()
                    .find(|a| a.get_id() == anno_id)
                {
                    target.set_show_comments(!target.show_comments());
                }
            }

            Message::GoToAnnotationPosition(anno_id, tile_id) => {
                self.go_to_annotation_position(anno_id, tile_id);
            }

            Message::CreateAnnotationGroup(name) => {
                self.save_current_canvas(format!("Added annotation group {name}"));
                let waves = self.user.waveform_edit()?;

                let new_group = AnnotationGroup {
                    name: name.clone(),
                    annotations: Vec::new(),
                };

                waves.items.annotation_groups.push(new_group);
            }

            Message::DeleteAnnotationGroup(name) => {
                self.save_current_canvas(format!("Removed annotation group {name}"));
                let waves = self.user.waveform_edit()?;

                waves.items.delete_group(&name);
            }

            Message::DeleteAllAnnotationInGroup(name) => {
                self.save_current_canvas(format!(
                    "Removed annotation group {name} and all it's annotations"
                ));
                let waves = self.user.waveform_edit()?;

                waves.items.remove_all_annotations_from_group(&name);
            }

            Message::AddCharToPrompt(c) => *self.char_to_add_to_prompt.borrow_mut() = Some(c),

            Message::UpdateAnnotationGroup(anno_id, name) => {
                self.save_current_canvas(format!("Added {anno_id:?} to {name:?}"));
                let waves = self.user.waveform_edit()?;

                let target = waves.items.remove_annotation_from_group(anno_id);

                match target {
                    Some(id) => {
                        waves.items.add_annotation_to_group(name.as_ref()?, id);
                    }
                    None => {
                        warn!("Error: Just removed non existent id!");
                    }
                }
            }

            Message::UpdateAnnotationName(anno_id, name) => {
                self.save_current_canvas(format!("Changed {anno_id:?} name to {name}"));
                let waves = self.user.waveform_edit()?;
                if let Some(target) = waves
                    .items
                    .annotations
                    .iter_mut()
                    .find(|a| a.get_id() == anno_id)
                {
                    target.set_name(&name);
                }
            }

            Message::SetGroupVisibility(group, visible) => {
                self.save_current_canvas(format!("Changed group {:?} visibility", group.name));
                if let Some(waves) = self.user.waveform_edit() {
                    for annotation_id in group.annotations {
                        for annotation in &mut waves.items.annotations {
                            if annotation_id == annotation.get_id() {
                                annotation.set_visibility(visible);
                            }
                        }
                    }
                }
            }

            Message::AnnotationClicked(id, menu_pos, tile_id, to_screen, frame_width) => {
                let target = tile_id.or_else(|| {
                    self.user
                        .workspace
                        .resolve_waveform(crate::tiles::TileTarget::Focused)
                })?;
                let waves = self.user.waveform_edit_at(target)?;
                let view = &*waves.view;
                let menu = id.and_then(|_| {
                    let position = menu_pos?;
                    let local = to_screen?.inverse().transform_pos(position);
                    let time =
                        view.viewport
                            .as_time_bigint(local.x, frame_width?, waves.time_range());
                    Some((position, time))
                });
                let view = waves.view;
                view.selected_annotation = id;
                view.annotation_menu = menu;
            }

            Message::RemoveCommentMessage(annotation_id, message_id) => {
                //self.save_current_canvas(format!("Removed message"));
                let waves = self.user.waveform_edit()?;
                if let Some(target) = waves
                    .items
                    .annotations
                    .iter_mut()
                    .find(|a| a.get_id() == annotation_id)
                {
                    target
                        .get_comment_box_mut()
                        .message_chain
                        .retain(|comment_message| comment_message.id != message_id);
                }
            }

            Message::ClickHandled() => {
                self.click_handled = true;
            }
            Message::UpdateCommentBox(changes) => {
                let waves = self.user.waveform_edit()?;
                for (annotation_id, comment) in changes {
                    if let Some(target) = waves
                        .items
                        .annotations
                        .iter_mut()
                        .find(|a| a.get_id() == annotation_id)
                    {
                        target.update_comment_box(comment);
                    }
                }
            }
            Message::AddCommentMessage(annotation_id, message, user) => {
                //self.save_current_canvas(format!("Added message"));
                let waves = self.user.waveform_edit()?;
                if let Some(target) = waves
                    .items
                    .annotations
                    .iter_mut()
                    .find(|a| a.get_id() == annotation_id)
                {
                    let comment = target.get_comment_box_mut();
                    comment.message_chain.push(CommentMessage {
                        id: egui::Id::new(("comment", comment.message_id_source)),
                        user,
                        text: message,
                    });
                    comment.message_id_source += 1;
                }
            }
            Message::ToggleCommentVisibility(annotation_id) => {
                let waves = self.user.waveform_edit()?;
                if let Some(target) = waves
                    .items
                    .annotations
                    .iter_mut()
                    .find(|a| a.get_id() == annotation_id)
                {
                    target.get_comment_box_mut().visible = !target.get_comment_box().visible;
                }
            }
        }

        Some(())
    }

    pub fn add_scope_as_group(
        &mut self,
        scope: &ScopeRef,
        pos: TargetPosition,
        recursive: bool,
        variable_name_type: Option<VariableNameType>,
    ) -> TargetPosition {
        let Some(mut waves) = self.user.waveform_edit() else {
            return pos;
        };
        let Some(container) = waves.document.inner.as_waves() else {
            return pos;
        };

        let variables = container
            .variables_in_scope(scope)
            .iter()
            .sorted_by(|a, b| numeric_sort::cmp(&a.name, &b.name))
            .cloned()
            .collect_vec();
        let child_scopes = container.child_scopes(scope);
        let variable_name_type = variable_name_type.or_else(|| {
            container
                .scope_is_variable(scope)
                .then_some(VariableNameType::Local)
        });

        if let Err(error) = waves.add_group(scope.name(), Some(pos)) {
            error!(%error, "scope insertion rejected");
            return pos;
        }
        let into_group_pos = TargetPosition {
            before: ItemIndex(pos.before.0 + 1),
            level: pos.level + 1,
        };

        let (cmd, variable_refs) = waves.add_variables(
            &self.translators,
            variables,
            Some(into_group_pos),
            false,
            false,
            variable_name_type,
            true,
        );
        let mut into_group_pos = TargetPosition {
            before: ItemIndex(into_group_pos.before.0 + variable_refs.len()),
            level: pos.level + 1,
        };

        if let Some(cmd) = cmd {
            self.load_variables(cmd);
        }

        if recursive {
            for child in child_scopes.unwrap_or(vec![]) {
                into_group_pos =
                    self.add_scope_as_group(&child, into_group_pos, recursive, variable_name_type);
                into_group_pos.level = pos.level + 1;
            }
        }
        into_group_pos
    }

    fn handle_variable_clipboard_operation<F>(
        &self,
        vidx: MessageTarget<VisibleItemIndex>,
        get_text: F,
    ) where
        F: FnOnce(&crate::wave_data::WaveformRead<'_>, DisplayedItemRef) -> Option<String>,
    {
        let Some(waves) = self.user.waveform_read() else {
            return;
        };
        let vidx = if let MessageTarget::Explicit(vidx) = vidx {
            vidx
        } else if let Some(focused) = waves.view.focused_index(waves.items) {
            focused
        } else {
            return;
        };
        let Some(item_ref) = waves
            .items
            .items_tree
            .get_visible(vidx)
            .map(|node| node.item_ref)
        else {
            return;
        };

        if let Some(text) = get_text(&waves, item_ref)
            && let Some(ctx) = &self.context
        {
            ctx.copy_text(text);
        }
    }
}

fn dump_tree(items: &crate::item_list::ItemList) {
    let mut result = String::new();
    for (idx, node) in items.items_tree.iter().enumerate() {
        for _ in 0..node.level.saturating_sub(1) {
            result.push(' ');
        }

        if node.level > 0 {
            match items.items_tree.get(ItemIndex(idx + 1)) {
                Some(next) if next.level < node.level => result.push_str("╰╴"),
                _ => result.push_str("├╴"),
            }
        }

        result.push_str(
            &items
                .displayed_items
                .get(&node.item_ref)
                .map_or("?".to_owned(), displayed_item::DisplayedItem::name),
        );
        result.push_str(&format!("   ({:?})", node.item_ref));
        if node.selected {
            result.push_str(" !SEL! ");
        }
        result.push('\n');
    }
    info!("tree: \n{}", &result);
}

pub struct StateWrapper(Arc<RwLock<SystemState>>);
impl App for StateWrapper {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        App::ui(&mut *self.0.write().unwrap(), ui, frame);
    }
}
