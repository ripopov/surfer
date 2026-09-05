//! The single registry of tile kinds and their versioned codecs.

use crate::tile_kinds::{
    unknown::UnknownTile,
    waveform::{WaveformMessage, WaveformPayloadError, WaveformTile, WaveformTileFile},
};

#[derive(Debug, ::serde::Deserialize)]
pub enum TileMessage {
    FrameBuffer(crate::tile_kinds::frame_buffer::FrameBufferMessage),
    Memory(crate::tile_kinds::memory::MemoryMessage),
    AnnotationList(crate::tile_kinds::annotation_list::AnnotationListMessage),
    Logs(crate::tile_kinds::logs::LogsMessage),
    Waveform(WaveformMessage),
}

/// Each variant contains only semantic settings owned by its kind.
#[derive(Clone)]
pub(crate) enum TileSettings {
    FrameBuffer(Box<crate::tile_kinds::frame_buffer::FrameBufferState>),
    Memory(Box<crate::memory_viewer::MemoryViewerSettings>),
    Logs(crate::tile_kinds::logs::LevelFilter),
    AnnotationList(bool),
    WaveformColumns { names: bool, values: bool },
    VerticalLink(bool),
}

impl TileSettings {
    pub(crate) fn source_changed(&self, kind: &TileKind) -> bool {
        match (self, kind) {
            (Self::Memory(before), TileKind::Memory(after)) => before.scope != after.settings.scope,
            (Self::FrameBuffer(before), TileKind::FrameBuffer(after)) => {
                before.content != after.state.content
            }
            _ => false,
        }
    }

    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::FrameBuffer(_) => "Change framebuffer settings",
            Self::Memory(_) => "Change memory settings",
            Self::Logs(_) => "Change log filter",
            Self::AnnotationList(_) => "Change annotation comments",
            Self::WaveformColumns { .. } => "Change waveform columns",
            Self::VerticalLink(_) => "Change linked scrolling",
        }
    }

    pub(crate) fn capture_like(&self, kind: &TileKind) -> Option<Self> {
        match (self, kind) {
            (Self::Logs(_), TileKind::Logs(tile)) => Some(Self::Logs(tile.filter)),
            (Self::AnnotationList(_), TileKind::AnnotationList(tile)) => {
                Some(Self::AnnotationList(tile.show_comments))
            }
            (Self::WaveformColumns { .. }, TileKind::Waveform(tile)) => {
                Some(Self::WaveformColumns {
                    names: tile.show_name_column,
                    values: tile.show_value_column,
                })
            }
            (Self::VerticalLink(_), TileKind::Waveform(tile)) => {
                Some(Self::VerticalLink(tile.link_vertical_scroll))
            }
            (Self::Memory(_), TileKind::Memory(tile)) => {
                Some(Self::Memory(Box::new(tile.settings.clone())))
            }
            (Self::FrameBuffer(_), TileKind::FrameBuffer(tile)) => {
                Some(Self::FrameBuffer(Box::new(tile.state.clone())))
            }
            _ => None,
        }
    }

    pub(crate) fn restore_message(&self) -> TileMessage {
        match self {
            Self::FrameBuffer(state) => TileMessage::FrameBuffer(
                crate::tile_kinds::frame_buffer::FrameBufferMessage::State(state.clone()),
            ),
            Self::Memory(settings) => TileMessage::Memory(
                crate::tile_kinds::memory::MemoryMessage::Settings(settings.clone()),
            ),
            Self::Logs(filter) => {
                TileMessage::Logs(crate::tile_kinds::logs::LogsMessage::SetFilter(*filter))
            }
            Self::AnnotationList(show) => TileMessage::AnnotationList(
                crate::tile_kinds::annotation_list::AnnotationListMessage::ShowComments(*show),
            ),
            Self::WaveformColumns { names, values } => {
                TileMessage::Waveform(WaveformMessage::Columns {
                    names: *names,
                    values: *values,
                })
            }
            Self::VerticalLink(link) => {
                TileMessage::Waveform(WaveformMessage::LinkVerticalScroll(*link))
            }
        }
    }
}

impl TileMessage {
    pub(crate) fn settings_before(&self, kind: &TileKind) -> Option<TileSettings> {
        match (self, kind) {
            (Self::Waveform(WaveformMessage::Columns { .. }), TileKind::Waveform(tile)) => {
                Some(TileSettings::WaveformColumns {
                    names: tile.show_name_column,
                    values: tile.show_value_column,
                })
            }
            (Self::Logs(_), TileKind::Logs(tile)) => Some(TileSettings::Logs(tile.filter)),
            (Self::AnnotationList(_), TileKind::AnnotationList(tile)) => {
                Some(TileSettings::AnnotationList(tile.show_comments))
            }
            (Self::Waveform(WaveformMessage::LinkVerticalScroll(_)), TileKind::Waveform(tile)) => {
                Some(TileSettings::VerticalLink(tile.link_vertical_scroll))
            }
            (Self::Memory(_), TileKind::Memory(tile)) => {
                Some(TileSettings::Memory(Box::new(tile.settings.clone())))
            }
            (Self::FrameBuffer(_), TileKind::FrameBuffer(tile)) => {
                Some(TileSettings::FrameBuffer(Box::new(tile.state.clone())))
            }
            _ => None,
        }
    }

    pub(crate) fn item_edit_label(&self) -> Option<&'static str> {
        match self {
            Self::Waveform(message) => message.item_edit_label(),
            Self::FrameBuffer(_) | Self::Memory(_) | Self::AnnotationList(_) | Self::Logs(_) => {
                None
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TileUpdateError {
    #[error(transparent)]
    Waveform(#[from] WaveformPayloadError),
    #[error(transparent)]
    FrameBuffer(#[from] crate::tile_kinds::frame_buffer::FrameBufferError),
}

impl TileKind {
    pub(crate) fn attach_inspector(
        &mut self,
        document: &mut crate::wave_data::WaveData,
    ) -> Option<crate::wellen::LoadSignalsCmd> {
        match self {
            Self::Memory(tile) => tile.attach(document),
            Self::FrameBuffer(tile) => tile.attach(document),
            _ => None,
        }
    }
}

impl crate::SystemState {
    pub(crate) fn attach_tile_source(&mut self, target: super::TileId) {
        let load = self
            .user
            .waves
            .as_mut()
            .and_then(|document| self.user.workspace.attach_inspector(target, document));
        if let Some(load) = load {
            self.load_variables(load);
        }
    }
}

use super::{
    ItemListId,
    commands::SplitMode,
    runtime::{IdentityError, WorkspaceRuntime},
    serde::{DecodeError, TileFile},
};

#[derive(Debug, thiserror::Error)]
pub enum KindCreateError {
    #[error("unavailable tile kind {0}")]
    Unavailable(String),
    #[error(transparent)]
    Identity(#[from] IdentityError),
}

pub struct KindDescriptor {
    pub name: &'static str,
    pub payload_version: u32,
    pub singleton: bool,
}

pub const WAVEFORM: KindDescriptor = KindDescriptor {
    name: "waveform",
    payload_version: 1,
    singleton: false,
};

pub const LOGS: KindDescriptor = KindDescriptor {
    name: "logs",
    payload_version: 1,
    singleton: true,
};
pub const MARKERS: KindDescriptor = KindDescriptor {
    name: "markers",
    payload_version: 1,
    singleton: true,
};
pub const TRANSACTION_DETAILS: KindDescriptor = KindDescriptor {
    name: "transaction_details",
    payload_version: 1,
    singleton: true,
};
pub const ANNOTATION_LIST: KindDescriptor = KindDescriptor {
    name: "annotation_list",
    payload_version: 1,
    singleton: true,
};
pub const MEMORY: KindDescriptor = KindDescriptor {
    name: "memory",
    payload_version: 1,
    singleton: false,
};
pub const FRAME_BUFFER: KindDescriptor = KindDescriptor {
    name: "frame_buffer",
    payload_version: 1,
    singleton: false,
};
pub const KINDS: &[KindDescriptor] = &[
    FRAME_BUFFER,
    MEMORY,
    WAVEFORM,
    LOGS,
    MARKERS,
    TRANSACTION_DETAILS,
    ANNOTATION_LIST,
];

#[derive(Clone)]
pub enum TileKind {
    FrameBuffer(crate::tile_kinds::frame_buffer::FrameBufferTile),
    Memory(crate::tile_kinds::memory::MemoryTile),
    AnnotationList(crate::tile_kinds::annotation_list::AnnotationListTile),
    TransactionDetails(crate::tile_kinds::transaction_details::TransactionDetailsTile),
    Markers(crate::tile_kinds::markers::MarkersTile),
    Logs(crate::tile_kinds::logs::LogsTile),
    Waveform(Box<WaveformTile>),
    Unknown(UnknownTile),
}

#[derive(Clone)]
pub struct TileEntry {
    pub title: Option<String>,
    pub kind: TileKind,
}

#[derive(Debug, thiserror::Error)]
pub enum KindDecodeError {
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error(transparent)]
    Waveform(#[from] WaveformPayloadError),
}

impl TileEntry {
    pub fn from_file(file: TileFile) -> Result<Self, KindDecodeError> {
        let kind = if file.kind == WAVEFORM.name && file.kind_version == WAVEFORM.payload_version {
            TileKind::Waveform(Box::new(WaveformTile::try_from(
                file.decode_payload::<WaveformTileFile>()?,
            )?))
        } else if file.kind == FRAME_BUFFER.name
            && file.kind_version == FRAME_BUFFER.payload_version
        {
            TileKind::FrameBuffer(file.decode_payload()?)
        } else if file.kind == MEMORY.name && file.kind_version == MEMORY.payload_version {
            TileKind::Memory(file.decode_payload()?)
        } else if file.kind == ANNOTATION_LIST.name
            && file.kind_version == ANNOTATION_LIST.payload_version
        {
            TileKind::AnnotationList(file.decode_payload()?)
        } else if file.kind == TRANSACTION_DETAILS.name
            && file.kind_version == TRANSACTION_DETAILS.payload_version
        {
            TileKind::TransactionDetails(file.decode_payload()?)
        } else if file.kind == MARKERS.name && file.kind_version == MARKERS.payload_version {
            TileKind::Markers(file.decode_payload()?)
        } else if file.kind == LOGS.name && file.kind_version == LOGS.payload_version {
            TileKind::Logs(file.decode_payload()?)
        } else {
            TileKind::Unknown(UnknownTile {
                kind_name: file.kind,
                kind_version: file.kind_version,
                payload: file.payload,
            })
        };
        Ok(Self {
            title: file.title,
            kind,
        })
    }

    pub fn to_file(&self) -> Result<TileFile, ron::Error> {
        match &self.kind {
            TileKind::FrameBuffer(tile) => TileFile::encode(
                self.title.clone(),
                FRAME_BUFFER.name,
                FRAME_BUFFER.payload_version,
                tile,
            ),
            TileKind::Memory(tile) => TileFile::encode(
                self.title.clone(),
                MEMORY.name,
                MEMORY.payload_version,
                tile,
            ),
            TileKind::AnnotationList(tile) => TileFile::encode(
                self.title.clone(),
                ANNOTATION_LIST.name,
                ANNOTATION_LIST.payload_version,
                tile,
            ),
            TileKind::TransactionDetails(tile) => TileFile::encode(
                self.title.clone(),
                TRANSACTION_DETAILS.name,
                TRANSACTION_DETAILS.payload_version,
                tile,
            ),
            TileKind::Markers(tile) => TileFile::encode(
                self.title.clone(),
                MARKERS.name,
                MARKERS.payload_version,
                tile,
            ),
            TileKind::Logs(tile) => {
                TileFile::encode(self.title.clone(), LOGS.name, LOGS.payload_version, tile)
            }
            TileKind::Waveform(tile) => TileFile::encode(
                self.title.clone(),
                WAVEFORM.name,
                WAVEFORM.payload_version,
                &WaveformTileFile::from(tile.as_ref()),
            ),
            TileKind::Unknown(tile) => Ok(TileFile {
                title: self.title.clone(),
                kind: tile.kind_name.clone(),
                kind_version: tile.kind_version,
                payload: tile.payload.clone(),
            }),
        }
    }

    /// The title without workspace context. Prefer `Workspace::titles`, which
    /// numbers waveforms and marks linked views.
    pub fn display_title(&self) -> String {
        self.title
            .clone()
            .unwrap_or_else(|| self.kind.default_title())
    }
}

/// A palette command a kind registers for itself. It is offered only while the
/// captured tile is of that kind and always executes against that tile.
pub struct KindCommand {
    pub name: &'static str,
    pub suggestions: &'static [&'static str],
    pub parse: fn(&str) -> Option<TileMessage>,
}

fn parse_switch(word: &str) -> Option<bool> {
    match word {
        "on" | "true" | "yes" => Some(true),
        "off" | "false" | "no" => Some(false),
        _ => None,
    }
}

const WAVEFORM_COMMANDS: &[KindCommand] = &[
    KindCommand {
        name: "tile_columns",
        suggestions: &["both", "names", "values", "none"],
        parse: |word| {
            let (names, values) = match word {
                "both" => (true, true),
                "names" => (true, false),
                "values" => (false, true),
                "none" => (false, false),
                _ => return None,
            };
            Some(TileMessage::Waveform(WaveformMessage::Columns {
                names,
                values,
            }))
        },
    },
    KindCommand {
        name: "tile_link_scroll",
        suggestions: &["on", "off"],
        parse: |word| {
            Some(TileMessage::Waveform(WaveformMessage::LinkVerticalScroll(
                parse_switch(word)?,
            )))
        },
    },
];

const LOGS_COMMANDS: &[KindCommand] = &[KindCommand {
    name: "logs_filter",
    suggestions: &["off", "error", "warn", "info", "debug", "trace"],
    parse: |word| {
        use crate::tile_kinds::logs::LevelFilter;
        let filter = match word {
            "off" => LevelFilter::Off,
            "error" => LevelFilter::Error,
            "warn" => LevelFilter::Warn,
            "info" => LevelFilter::Info,
            "debug" => LevelFilter::Debug,
            "trace" => LevelFilter::Trace,
            _ => return None,
        };
        Some(TileMessage::Logs(
            crate::tile_kinds::logs::LogsMessage::SetFilter(filter),
        ))
    },
}];

const ANNOTATION_LIST_COMMANDS: &[KindCommand] = &[KindCommand {
    name: "annotation_list_comments",
    suggestions: &["on", "off"],
    parse: |word| {
        Some(TileMessage::AnnotationList(
            crate::tile_kinds::annotation_list::AnnotationListMessage::ShowComments(parse_switch(
                word,
            )?),
        ))
    },
}];

impl TileKind {
    pub fn default_title(&self) -> String {
        match self {
            Self::Waveform(_) => "Waveform".into(),
            Self::Logs(_) => "Logs".into(),
            Self::Markers(_) => "Markers".into(),
            Self::FrameBuffer(_) => "Frame Buffer".into(),
            Self::Memory(tile) => tile
                .settings
                .name
                .clone()
                .or_else(|| {
                    tile.settings
                        .scope
                        .as_ref()
                        .map(|scope| scope.strs.join("."))
                })
                .map_or_else(|| "Memory".into(), |name| format!("Memory: {name}")),
            Self::AnnotationList(_) => "Annotations".into(),
            Self::TransactionDetails(_) => "Transaction Details".into(),
            Self::Unknown(tile) => format!("Unavailable: {}", tile.kind_name),
        }
    }

    /// Kind-specific palette commands; the parser resolves them to the captured tile.
    pub fn commands(&self) -> &'static [KindCommand] {
        match self {
            Self::Waveform(_) => WAVEFORM_COMMANDS,
            Self::Logs(_) => LOGS_COMMANDS,
            Self::AnnotationList(_) => ANNOTATION_LIST_COMMANDS,
            Self::FrameBuffer(_)
            | Self::Memory(_)
            | Self::TransactionDetails(_)
            | Self::Markers(_)
            | Self::Unknown(_) => &[],
        }
    }

    pub(crate) fn reset_runtime(&mut self) {
        match self {
            Self::Waveform(tile) => tile.view.reset_runtime(),
            Self::FrameBuffer(tile) => tile.reset_runtime(),
            Self::Memory(tile) => tile.reset_runtime(),
            Self::AnnotationList(_)
            | Self::TransactionDetails(_)
            | Self::Markers(_)
            | Self::Logs(_)
            | Self::Unknown(_) => {}
        }
    }

    pub fn tab_context_menu(&self, ui: &mut egui::Ui, cx: &mut super::view::TileCtx<'_>) {
        match self {
            Self::Waveform(tile) => tile.tab_context_menu(ui, cx),
            Self::FrameBuffer(_)
            | Self::Memory(_)
            | Self::AnnotationList(_)
            | Self::TransactionDetails(_)
            | Self::Markers(_)
            | Self::Logs(_)
            | Self::Unknown(_) => {}
        }
    }

    /// The registry is the only factory dispatch; resources are returned for atomic installation.
    pub fn create(
        name: &str,
        runtime: &mut WorkspaceRuntime,
    ) -> Result<
        (
            Self,
            std::collections::BTreeMap<ItemListId, crate::item_list::ItemList>,
        ),
        KindCreateError,
    > {
        match name {
            name if name == FRAME_BUFFER.name => {
                Ok((Self::FrameBuffer(Default::default()), Default::default()))
            }
            name if name == MEMORY.name => {
                Ok((Self::Memory(Default::default()), Default::default()))
            }
            name if name == ANNOTATION_LIST.name => {
                Ok((Self::AnnotationList(Default::default()), Default::default()))
            }
            name if name == TRANSACTION_DETAILS.name => Ok((
                Self::TransactionDetails(Default::default()),
                Default::default(),
            )),
            name if name == MARKERS.name => {
                Ok((Self::Markers(Default::default()), Default::default()))
            }
            name if name == LOGS.name => Ok((Self::Logs(Default::default()), Default::default())),
            name if name == WAVEFORM.name => {
                let id = runtime.allocate_list()?;
                Ok((
                    Self::Waveform(Box::new(WaveformTile::new(id))),
                    [(id, Default::default())].into(),
                ))
            }
            _ => Err(KindCreateError::Unavailable(name.into())),
        }
    }

    pub fn descriptor(&self) -> Option<&'static KindDescriptor> {
        match self {
            Self::Waveform(_) => Some(&WAVEFORM),
            Self::Logs(_) => Some(&LOGS),
            Self::Markers(_) => Some(&MARKERS),
            Self::FrameBuffer(_) => Some(&FRAME_BUFFER),
            Self::Memory(_) => Some(&MEMORY),
            Self::AnnotationList(_) => Some(&ANNOTATION_LIST),
            Self::TransactionDetails(_) => Some(&TRANSACTION_DETAILS),
            Self::Unknown(_) => None,
        }
    }

    pub fn is_waveform(&self) -> bool {
        matches!(self, Self::Waveform(_))
    }

    pub fn supports_split(&self, mode: SplitMode) -> bool {
        match self {
            Self::Waveform(_) => matches!(mode, SplitMode::Linked | SplitMode::Independent),
            Self::FrameBuffer(_) => mode == SplitMode::Clone,
            Self::Memory(_) => mode == SplitMode::Clone,
            Self::AnnotationList(_)
            | Self::TransactionDetails(_)
            | Self::Markers(_)
            | Self::Logs(_)
            | Self::Unknown(_) => false,
        }
    }

    pub fn kind_name(&self) -> &str {
        match self {
            Self::Waveform(_) => WAVEFORM.name,
            Self::Logs(_) => LOGS.name,
            Self::Markers(_) => MARKERS.name,
            Self::FrameBuffer(_) => FRAME_BUFFER.name,
            Self::Memory(_) => MEMORY.name,
            Self::AnnotationList(_) => ANNOTATION_LIST.name,
            Self::TransactionDetails(_) => TRANSACTION_DETAILS.name,
            Self::Unknown(tile) => &tile.kind_name,
        }
    }

    /// The primary waveform list, for waveform-specific editing only.
    /// Ownership code must use `dependencies` instead.
    pub fn waveform_list(&self) -> Option<ItemListId> {
        match self {
            Self::Waveform(tile) => Some(tile.items),
            Self::FrameBuffer(_)
            | Self::Memory(_)
            | Self::AnnotationList(_)
            | Self::TransactionDetails(_)
            | Self::Markers(_)
            | Self::Logs(_)
            | Self::Unknown(_) => None,
        }
    }

    pub(crate) fn valid_resource_references<'a>(
        &self,
        item_list: impl Fn(ItemListId) -> Option<&'a crate::item_list::ItemList>,
    ) -> bool {
        match self {
            Self::Waveform(tile) => item_list(tile.items).is_some_and(|items| {
                tile.view
                    .focused_item
                    .is_none_or(|id| items.displayed_items.contains_key(&id))
            }),
            Self::FrameBuffer(_)
            | Self::Memory(_)
            | Self::AnnotationList(_)
            | Self::TransactionDetails(_)
            | Self::Markers(_)
            | Self::Logs(_)
            | Self::Unknown(_) => true,
        }
    }

    pub fn split_clone(&self) -> Option<Self> {
        match self {
            Self::Waveform(tile) => Some(Self::Waveform(tile.clone())),
            Self::FrameBuffer(tile) => Some(Self::FrameBuffer(tile.clone())),
            Self::Memory(tile) => Some(Self::Memory(tile.clone())),
            Self::AnnotationList(_)
            | Self::TransactionDetails(_)
            | Self::Markers(_)
            | Self::Logs(_)
            | Self::Unknown(_) => None,
        }
    }
}

/// Registry-backed renderer used by the application layout adapter.
pub(crate) struct ApplicationPanes<'a> {
    pub state: &'a crate::SystemState,
    /// Draw the `item_focus` overlay in the target waveform tile.
    pub focus_ids: bool,
    target_waveform: Option<super::TileId>,
    /// Computed once per frame (§6.5).
    titles: std::collections::BTreeMap<super::TileId, String>,
}
impl<'a> ApplicationPanes<'a> {
    pub fn new(state: &'a crate::SystemState, focus_ids: bool) -> Self {
        Self {
            state,
            focus_ids,
            target_waveform: state
                .user
                .workspace
                .resolve_waveform(super::TileTarget::Focused),
            titles: state.user.workspace.titles(),
        }
    }
}
impl super::render::PaneRenderer for ApplicationPanes<'_> {
    type Command = crate::Message;
    fn focus_stroke(&self, _visuals: &egui::Visuals) -> egui::Stroke {
        (&self.state.user.config.theme.tile_focus_stroke).into()
    }
    fn title(&self, id: super::TileId) -> String {
        self.titles.get(&id).cloned().unwrap_or_default()
    }
    fn tab_bar_menu(
        &self,
        anchor: super::TileId,
        ui: &mut egui::Ui,
        commands: &mut Vec<crate::Message>,
    ) {
        use super::{TileTarget, layout::Direction};
        let workspace = &self.state.user.workspace;
        ui.menu_button("New tile", |ui| {
            for kind in KINDS {
                if ui.button(kind.name).clicked() {
                    commands.push(crate::Message::Workspace(
                        super::commands::WorkspaceCommand::OpenTile {
                            kind: kind.name.into(),
                            placement: super::layout::Placement::TabAfter(anchor),
                            focus: true,
                        },
                    ));
                    ui.close();
                }
            }
        });
        for (label, dir) in [
            ("Split right", Direction::Right),
            ("Split down", Direction::Down),
        ] {
            let command = workspace.split_command(TileTarget::Id(anchor), dir, false);
            if ui
                .add_enabled(command.is_some(), egui::Button::new(label))
                .clicked()
                && let Some(command) = command
            {
                commands.push(crate::Message::Workspace(command));
                ui.close();
            }
        }
    }
    fn ui(
        &self,
        id: super::TileId,
        focused: bool,
        ui: &mut egui::Ui,
        commands: &mut Vec<crate::Message>,
    ) {
        let Some(entry) = self.state.user.workspace.tiles().get(&id) else {
            return;
        };
        match &entry.kind {
            TileKind::Waveform(tile) => self.state.draw_waveform_body(
                ui,
                commands,
                id,
                crate::tile_kinds::waveform_body::WaveformColumns {
                    focus_ids: self.focus_ids && self.target_waveform == Some(id),
                    names: tile.show_name_column.then_some(tile.name_column_width),
                    values: tile.show_value_column.then_some(tile.value_column_width),
                },
            ),
            TileKind::TransactionDetails(tile) => tile.ui(ui, self.state.user.waveform_read()),
            TileKind::FrameBuffer(tile) => {
                tile.ui(ui, id, self.state.user.waves.as_ref(), commands)
            }
            TileKind::Memory(tile) => tile.ui(
                ui,
                id,
                self.state.user.waves.as_ref(),
                &self.state.translators,
                &self.state.user.config.theme,
                commands,
            ),
            TileKind::AnnotationList(tile) => tile.ui(
                ui,
                id,
                self.state.user.waveform_read(),
                &self.state.waveform_services(),
                commands,
            ),
            TileKind::Markers(tile) => tile.ui(
                ui,
                self.state.user.waveform_read(),
                &self.state.waveform_services(),
                commands,
            ),
            TileKind::Logs(tile) => {
                let mut cx = super::view::TileCtx::new(
                    super::view::TileReadServices {
                        document: self.state.user.waves.as_ref(),
                        item_lists: self.state.user.workspace.item_lists(),
                        config: &self.state.user.config,
                        translators: &self.state.translators,
                        runtime: &self.state.workspace_runtime,
                    },
                    id,
                    focused,
                    commands,
                );
                tile.ui(ui, &mut cx);
            }
            TileKind::Unknown(tile) => {
                ui.vertical_centered(|ui| {
                    ui.heading("Tile unavailable");
                    ui.label(format!(
                        "{} (version {})",
                        tile.kind_name, tile.kind_version
                    ));
                    ui.label("This tile's saved settings are preserved.");
                });
            }
        }
    }
    fn tab_context_menu(
        &self,
        id: super::TileId,
        ui: &mut egui::Ui,
        commands: &mut Vec<crate::Message>,
    ) {
        let Some(entry) = self.state.user.workspace.tiles().get(&id) else {
            return;
        };
        let mut cx = super::view::TileCtx::new(
            super::view::TileReadServices {
                document: self.state.user.waves.as_ref(),
                item_lists: self.state.user.workspace.item_lists(),
                config: &self.state.user.config,
                translators: &self.state.translators,
                runtime: &self.state.workspace_runtime,
            },
            id,
            self.state.user.workspace.layout().focused() == Some(id),
            commands,
        );
        super::view::tab_context_menu(entry, ui, &mut cx);
    }
}

impl TileKind {
    pub fn dependencies(&self) -> super::resources::Dependencies {
        use super::resources::{Dependencies, ResourceId};
        match self {
            Self::Waveform(tile) => Dependencies::known([ResourceId::ItemList(tile.items)]),
            Self::Unknown(_) => Dependencies::opaque(),
            Self::FrameBuffer(_)
            | Self::Memory(_)
            | Self::AnnotationList(_)
            | Self::TransactionDetails(_)
            | Self::Markers(_)
            | Self::Logs(_) => Dependencies::default(),
        }
    }
}

impl super::resources::ResourceOwner for TileKind {
    fn dependencies(&self) -> super::resources::Dependencies {
        self.dependencies()
    }
    fn remap_resources(
        &mut self,
        mapping: &super::resources::ResourceRemap,
    ) -> Result<(), super::resources::ResourceError> {
        match self {
            Self::Waveform(tile) => {
                tile.items = mapping.item_list(tile.items)?;
                tile.link_vertical_scroll = false;
            }
            Self::Unknown(_) => return Err(super::resources::ResourceError::Opaque),
            Self::FrameBuffer(_)
            | Self::Memory(_)
            | Self::AnnotationList(_)
            | Self::TransactionDetails(_)
            | Self::Markers(_)
            | Self::Logs(_) => {}
        }
        Ok(())
    }
}
