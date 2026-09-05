//! Semantic view and document operations with access to workspace storage.
use crate::tile_kinds::waveform::{WaveformMessage, WaveformPayloadError, WaveformUpdateCtx};
use crate::tiles::kind::{TileEntry, TileKind, TileMessage, TileUpdateError};
impl crate::tiles::workspace::Workspace {
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
                    .waveform_list()
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
        for entry in self.tiles.values_mut() {
            if let Some(command) = entry.kind.attach_inspector(document) {
                loads.push(command);
            }
        }
        self.reset_document_runtime();
        loads
    }

    pub(crate) fn update_viewports(&mut self, document: &mut crate::wave_data::WaveData) {
        if let Some(old_end) = document.old_max_timestamp.take() {
            let new_end = document.safe_max_timestamp();
            if new_end == old_end {
                document.cached_time_range.end = new_end;
                return;
            }
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
        id: crate::tiles::TileId,
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
        id: crate::tiles::TileId,
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
        target: crate::tiles::TileId,
        document: &'a mut crate::wave_data::WaveData,
    ) -> Option<crate::wave_data::WaveformEdit<'a>> {
        let list_id = self.tiles.get(&target)?.kind.waveform_list()?;
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
        target: crate::tiles::TileId,
        message: &TileMessage,
    ) -> Option<crate::tiles::TileId> {
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
        target: crate::tiles::TileId,
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

impl crate::tiles::workspace::Workspace {
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

impl super::Workspace {
    pub(crate) fn attach_inspector(
        &mut self,
        target: crate::tiles::TileId,
        document: &mut crate::wave_data::WaveData,
    ) -> Option<crate::wellen::LoadSignalsCmd> {
        self.tiles.get_mut(&target)?.kind.attach_inspector(document)
    }
}
