//! The single registry of tile kinds and their versioned codecs.

use crate::tile_kinds::{
    unknown::UnknownTile,
    waveform::{
        WaveformMessage, WaveformPayloadError, WaveformTile, WaveformTileFile, WaveformUpdateCtx,
    },
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

impl super::workspace::Workspace {
    pub(crate) fn attach_document(
        &mut self,
        document: &mut crate::wave_data::WaveData,
        translators: &crate::translation::TranslatorList,
        clear: bool,
        keep_unavailable: bool,
    ) -> Vec<crate::wellen::LoadSignalsCmd> {
        for entry in self.tiles.values_mut() {
            entry.kind.reset_runtime();
        }
        let mut seen = std::collections::BTreeSet::new();
        let targets = self
            .tiles
            .iter()
            .filter_map(|(id, entry)| {
                entry
                    .kind
                    .item_list()
                    .filter(|list| seen.insert(*list))
                    .map(|_| *id)
            })
            .collect::<Vec<_>>();
        let mut loads = Vec::new();
        for target in targets {
            let Some(mut edit) = self.waveform_edit(target, document) else {
                continue;
            };
            if clear {
                let name_type = edit.items.default_variable_name_type;
                *edit.items = crate::item_list::ItemList::default();
                edit.items.default_variable_name_type = name_type;
                for view in std::iter::once(&mut *edit.view)
                    .chain(edit.peers.iter_mut().map(|view| &mut **view))
                {
                    *view = crate::viewport::Viewport::new().into();
                }
            } else if let Some(load) = edit.reattach(translators, keep_unavailable) {
                loads.push(load);
            }
        }
        if let Some(container) = document.inner.as_waves_mut() {
            let mut arrays = std::collections::BTreeSet::new();
            for entry in self.tiles.values() {
                if let TileKind::Memory(tile) = &entry.kind
                    && let Some(scope) = &tile.settings.scope
                    && arrays.insert(scope.strs.clone())
                    && container.scope_exists(scope)
                {
                    let variables = container.variables_in_scope(scope);
                    match container.load_variables(variables.iter()) {
                        Ok(Some(command)) => loads.push(command),
                        Ok(None) => {}
                        Err(error) => tracing::warn!("Memory array load failed: {error}"),
                    }
                }
            }
        }
        for entry in self.tiles.values_mut() {
            if let TileKind::FrameBuffer(tile) = &mut entry.kind
                && let Some(command) = tile.attach(document)
            {
                loads.push(command);
            }
        }
        self.reset_document_runtime();
        loads
    }

    pub(crate) fn update_viewports(&mut self, document: &mut crate::wave_data::WaveData) {
        if let Some(old_end) = document.old_max_timestamp.take() {
            let new_end = document.safe_max_timestamp();
            let old_range = crate::wave_data::TimeRange {
                start: document.time_range().start.clone(),
                end: old_end,
            };
            for entry in self.tiles.values_mut() {
                if let TileKind::Waveform(tile) = &mut entry.kind {
                    tile.view.viewport = tile.view.viewport.clip_to(&old_range, &new_end);
                }
            }
            document.cached_time_range.end = new_end;
        }
    }

    pub(crate) fn measure_waveform(
        &mut self,
        id: super::TileId,
        height: f32,
        scroll_offset: Option<f32>,
    ) -> Result<bool, TileUpdateError> {
        let Some(TileEntry {
            kind: TileKind::Waveform(tile),
            ..
        }) = self.tiles.get_mut(&id)
        else {
            return Ok(false);
        };
        let changed = height.is_finite() && height >= 0.0 && tile.view.viewport_height != height;
        if changed {
            tile.view.viewport_height = height;
        }
        let offset = scroll_offset
            .filter(|offset| offset.is_finite())
            .unwrap_or(tile.view.scroll_offset);
        let scrolled = self.apply_tile_message(
            id,
            TileMessage::Waveform(WaveformMessage::ScrollTo(offset)),
            None,
        )?;
        Ok(changed || scrolled)
    }

    pub(crate) fn waveform_resources(
        &self,
        id: super::TileId,
    ) -> Option<(
        &crate::item_list::ItemList,
        &crate::tile_kinds::waveform::WaveformView,
    )> {
        match &self.tiles.get(&id)?.kind {
            TileKind::Waveform(tile) => Some((self.item_lists.get(&tile.items)?, &tile.view)),
            TileKind::FrameBuffer(_)
            | TileKind::Memory(_)
            | TileKind::AnnotationList(_)
            | TileKind::TransactionDetails(_)
            | TileKind::Markers(_)
            | TileKind::Logs(_)
            | TileKind::Unknown(_) => None,
        }
    }

    /// Borrow exactly the requested waveform's resources. The authoritative
    /// layout, tile entries and unrelated lists remain installed throughout.
    pub(crate) fn waveform_edit<'a>(
        &'a mut self,
        target: super::TileId,
        document: &'a mut crate::wave_data::WaveData,
    ) -> Option<crate::wave_data::WaveformEdit<'a>> {
        let list_id = self.tiles.get(&target)?.kind.item_list()?;
        let items = self.item_lists.get_mut(&list_id)?;
        let mut views = self
            .tiles
            .iter_mut()
            .filter_map(|(id, entry)| match &mut entry.kind {
                TileKind::Waveform(tile) if tile.items == list_id => Some((*id, &mut tile.view)),
                _ => None,
            })
            .collect::<Vec<_>>();
        let index = views.iter().position(|(id, _)| *id == target)?;
        let (_, view) = views.swap_remove(index);
        Some(crate::wave_data::WaveformEdit {
            document,
            items,
            view,
            peers: views.into_iter().map(|(_, view)| view).collect(),
        })
    }

    pub(crate) fn invalidate_all(&self) {
        for entry in self.tiles.values() {
            match &entry.kind {
                TileKind::Waveform(tile) => tile.view.invalidate_draw_cache(),
                TileKind::FrameBuffer(_)
                | TileKind::Memory(_)
                | TileKind::AnnotationList(_)
                | TileKind::TransactionDetails(_)
                | TileKind::Markers(_)
                | TileKind::Logs(_)
                | TileKind::Unknown(_) => {}
            }
        }
    }

    pub(crate) fn reset_document_runtime(&mut self) {
        for entry in self.tiles.values_mut() {
            entry.kind.reset_runtime();
        }
        for items in self.item_lists.values_mut() {
            items.layout_cache.take();
            items.flattened_rows_cache.take();
        }
    }

    pub fn validate_message_target(
        &self,
        target: super::TileId,
        message: &TileMessage,
    ) -> Option<super::TileId> {
        let kind = &self.tiles.get(&target)?.kind;
        matches!(
            (kind, message),
            (TileKind::FrameBuffer(_), TileMessage::FrameBuffer(_))
                | (TileKind::Memory(_), TileMessage::Memory(_))
                | (TileKind::AnnotationList(_), TileMessage::AnnotationList(_))
                | (TileKind::Logs(_), TileMessage::Logs(_))
                | (TileKind::Waveform(_), TileMessage::Waveform(_))
        )
        .then_some(target)
    }

    /// Reapply current offsets after topology or measured row/viewport geometry changes.
    pub fn reconcile_waveform_scroll(&mut self) {
        let mut groups = std::collections::BTreeSet::new();
        let targets = self
            .tiles
            .iter()
            .filter_map(|(id, entry)| match &entry.kind {
                TileKind::Waveform(tile)
                    if !tile.link_vertical_scroll || groups.insert(tile.items) =>
                {
                    Some((*id, tile.view.scroll_offset))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        for (id, offset) in targets {
            let _ = self.apply_tile_message(
                id,
                TileMessage::Waveform(WaveformMessage::ScrollTo(offset)),
                None,
            );
        }
    }

    pub fn apply_tile_message(
        &mut self,
        target: super::TileId,
        message: TileMessage,
        document: Option<&crate::wave_data::WaveData>,
    ) -> Result<bool, TileUpdateError> {
        match message {
            TileMessage::FrameBuffer(message) => {
                let Some(TileEntry {
                    kind: TileKind::FrameBuffer(tile),
                    ..
                }) = self.tiles.get_mut(&target)
                else {
                    return Ok(false);
                };
                tile.update(message).map_err(Into::into)
            }
            TileMessage::Memory(message) => {
                let Some(TileEntry {
                    kind: TileKind::Memory(tile),
                    ..
                }) = self.tiles.get_mut(&target)
                else {
                    return Ok(false);
                };
                Ok(tile.update(message))
            }
            TileMessage::AnnotationList(message) => {
                let Some(TileEntry {
                    kind: TileKind::AnnotationList(tile),
                    ..
                }) = self.tiles.get_mut(&target)
                else {
                    return Ok(false);
                };
                Ok(tile.update(message))
            }
            TileMessage::Logs(message) => {
                let Some(TileEntry {
                    kind: TileKind::Logs(tile),
                    ..
                }) = self.tiles.get_mut(&target)
                else {
                    return Ok(false);
                };
                Ok(tile.update(message))
            }
            TileMessage::Waveform(message) => {
                let Some(TileEntry {
                    kind: TileKind::Waveform(tile),
                    ..
                }) = self.tiles.get(&target)
                else {
                    tracing::warn!(
                        ?target,
                        "waveform command ignored: missing or wrong-kind tile"
                    );
                    return Ok(false);
                };
                let list = tile.items;
                let Some(items) = self.item_lists.get_mut(&list) else {
                    return Err(WaveformPayloadError::InvalidList.into());
                };
                let visible = self.layout.visible_tiles();
                let mut peers = self
                    .tiles
                    .iter_mut()
                    .filter_map(|(id, entry)| match &mut entry.kind {
                        TileKind::Waveform(tile) if tile.items == list => {
                            Some((*id, tile.as_mut(), visible.contains(id)))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                let index = peers
                    .iter()
                    .position(|(id, _, _)| *id == target)
                    .expect("target checked above");
                let (_, tile, visible) = peers.swap_remove(index);
                tile.update(
                    message,
                    &mut WaveformUpdateCtx {
                        document,
                        items,
                        visible,
                        peers: peers
                            .into_iter()
                            .map(|(_, tile, visible)| (tile, visible))
                            .collect(),
                    },
                )
                .map_err(Into::into)
            }
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

    pub fn display_title(&self) -> String {
        self.title.clone().unwrap_or_else(|| match &self.kind {
            TileKind::Waveform(_) => "Waveform".into(),
            TileKind::Logs(_) => "Logs".into(),
            TileKind::Markers(_) => "Markers".into(),
            TileKind::FrameBuffer(_) => "Frame Buffer".into(),
            TileKind::Memory(_) => "Memory".into(),
            TileKind::AnnotationList(_) => "Annotations".into(),
            TileKind::TransactionDetails(_) => "Transaction Details".into(),
            TileKind::Unknown(tile) => format!("Unavailable: {}", tile.kind_name),
        })
    }
}

impl TileKind {
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
    ) -> Result<(Self, Option<(ItemListId, crate::item_list::ItemList)>), KindCreateError> {
        match name {
            name if name == FRAME_BUFFER.name => Ok((Self::FrameBuffer(Default::default()), None)),
            name if name == MEMORY.name => Ok((Self::Memory(Default::default()), None)),
            name if name == ANNOTATION_LIST.name => {
                Ok((Self::AnnotationList(Default::default()), None))
            }
            name if name == TRANSACTION_DETAILS.name => {
                Ok((Self::TransactionDetails(Default::default()), None))
            }
            name if name == MARKERS.name => Ok((Self::Markers(Default::default()), None)),
            name if name == LOGS.name => Ok((Self::Logs(Default::default()), None)),
            name if name == WAVEFORM.name => {
                let id = runtime.allocate_list()?;
                Ok((
                    Self::Waveform(Box::new(WaveformTile::new(id))),
                    Some((id, Default::default())),
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

    pub fn replace_item_list(&mut self, id: ItemListId) {
        match self {
            Self::Waveform(tile) => {
                tile.items = id;
                tile.link_vertical_scroll = false;
            }
            Self::FrameBuffer(_)
            | Self::Memory(_)
            | Self::AnnotationList(_)
            | Self::TransactionDetails(_)
            | Self::Markers(_)
            | Self::Logs(_)
            | Self::Unknown(_) => {}
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

    /// Known list references. Unknown payloads require conservative resource retention.
    pub fn item_list(&self) -> Option<ItemListId> {
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

    pub fn valid_item_references(&self, items: &crate::item_list::ItemList) -> bool {
        match self {
            Self::Waveform(tile) => tile
                .view
                .focused_item
                .is_none_or(|id| items.displayed_items.contains_key(&id)),
            Self::FrameBuffer(_)
            | Self::Memory(_)
            | Self::AnnotationList(_)
            | Self::TransactionDetails(_)
            | Self::Markers(_)
            | Self::Logs(_)
            | Self::Unknown(_) => true,
        }
    }

    pub fn has_unknown_resources(&self) -> bool {
        matches!(self, Self::Unknown(_))
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
    pub focus_ids: bool,
}
impl super::render::PaneRenderer for ApplicationPanes<'_> {
    type Command = crate::Message;
    fn focus_stroke(&self, _visuals: &egui::Visuals) -> egui::Stroke {
        (&self.state.user.config.theme.tile_focus_stroke).into()
    }
    fn title(&self, id: super::TileId) -> String {
        self.state
            .user
            .workspace
            .tiles
            .get(&id)
            .map(TileEntry::display_title)
            .unwrap_or_default()
    }
    fn ui(
        &self,
        id: super::TileId,
        focused: bool,
        ui: &mut egui::Ui,
        commands: &mut Vec<crate::Message>,
    ) {
        let Some(entry) = self.state.user.workspace.tiles.get(&id) else {
            return;
        };
        match &entry.kind {
            TileKind::Waveform(tile) => self.state.draw_waveform_body(
                ui,
                commands,
                id,
                crate::tile_kinds::waveform_body::WaveformColumns {
                    focus_ids: self.focus_ids && focused,
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
                        item_lists: &self.state.user.workspace.item_lists,
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
        let Some(entry) = self.state.user.workspace.tiles.get(&id) else {
            return;
        };
        let mut cx = super::view::TileCtx::new(
            super::view::TileReadServices {
                document: self.state.user.waves.as_ref(),
                item_lists: &self.state.user.workspace.item_lists,
                config: &self.state.user.config,
                translators: &self.state.translators,
                runtime: &self.state.workspace_runtime,
            },
            id,
            self.state.user.workspace.layout.focused() == Some(id),
            commands,
        );
        super::view::tab_context_menu(entry, ui, &mut cx);
    }
}

impl super::workspace::Workspace {
    pub(crate) fn animate_waveforms(&mut self, dt: f32) -> bool {
        let mut moving = false;
        for entry in self.tiles.values_mut() {
            if let TileKind::Waveform(tile) = &mut entry.kind
                && tile.view.viewport.is_moving()
            {
                tile.view.viewport.move_viewport(dt);
                moving = true;
            }
        }
        moving
    }
    pub(crate) fn set_viewport_strategy(&mut self, strategy: crate::viewport::ViewportStrategy) {
        for entry in self.tiles.values_mut() {
            if let TileKind::Waveform(tile) = &mut entry.kind {
                tile.view.viewport.move_strategy = strategy;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        tile_kinds::waveform::{WaveformView, WaveformViewFile},
        viewport::Viewport,
    };

    fn two_linked_tiles() -> (
        super::super::workspace::Workspace,
        WorkspaceRuntime,
        super::super::TileId,
        super::super::TileId,
    ) {
        use super::super::{
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
        let list = workspace.tiles[&first].kind.item_list().unwrap();
        let mut layout = workspace.item_lists[&list].layout_cache.borrow_mut();
        layout.signature = Some(1);
        layout.total_height = 500.0;
        drop(layout);
        (workspace, runtime, first, second)
    }

    fn offset(workspace: &super::super::workspace::Workspace, id: super::super::TileId) -> f32 {
        let TileKind::Waveform(tile) = &workspace.tiles[&id].kind else {
            unreachable!()
        };
        tile.view.scroll_offset
    }

    #[test]
    fn linked_scroll_uses_visible_bounds_and_reclamps_when_a_tab_is_revealed() {
        use super::super::{commands::WorkspaceCommand, layout::Placement};
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
        use super::super::{commands::WorkspaceCommand, layout::Direction};
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
            workspace.tiles[&first].kind.item_list().unwrap()
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
                    super::super::TileId(999),
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
        assert_eq!(copy.item_list(), Some(ItemListId(7)));
        let TileKind::Waveform(copy) = copy else {
            panic!("wrong kind");
        };
        assert_eq!(copy.view.scroll_offset, 70.0);
        assert_eq!(copy.view.draw_cache.borrow().builds, 0);
        assert!(copy.view.interaction.measure_start_location.is_none());
        let encoded = ron::to_string(&entry.to_file().unwrap()).unwrap();
        let restored =
            TileEntry::from_file(super::super::serde::decode(&encoded).unwrap()).unwrap();
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
                super::super::serde::decode(include_str!("fixtures/future-tile.ron")).unwrap();
            file.kind = name.into();
            file.kind_version = version;
            let raw = file.payload.get_ron().to_owned();
            let mut entry = TileEntry::from_file(file).unwrap();
            assert!(entry.kind.has_unknown_resources());
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
            TransactionContainer { inner },
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
        let list_id = workspace.tiles[&first].kind.item_list().unwrap();
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
        let focus = |workspace: &super::super::workspace::Workspace, id| {
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
        use super::super::{commands::WorkspaceCommand, layout::Direction};
        use crate::{
            displayed_item::{DisplayedDivider, DisplayedItem},
            item_list::ItemSelection,
        };
        let (mut workspace, mut runtime, first, second) = two_linked_tiles();
        let list_id = workspace.tiles[&first].kind.item_list().unwrap();
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
        let copy_list = workspace.tiles[&copy].kind.item_list().unwrap();
        assert!(
            workspace
                .apply_tile_message(
                    second,
                    TileMessage::Waveform(WaveformMessage::Selection(ItemSelection::Toggle(item))),
                    None
                )
                .unwrap()
        );
        assert_eq!(workspace.tiles[&second].kind.item_list(), Some(list_id));
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
        use super::super::{commands::WorkspaceCommand, layout::Direction};
        use crate::displayed_item::{DisplayedDivider, DisplayedItem};
        let (mut workspace, mut runtime, first, second) = two_linked_tiles();
        let list_id = workspace.tiles[&first].kind.item_list().unwrap();
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
        let copy_list = workspace.tiles[&copy].kind.item_list().unwrap();
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
}
