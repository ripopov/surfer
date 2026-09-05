//! Semantic history retains affected resources, never the whole workspace.
use crate::tiles::{
    TileId,
    kind::TileSettings,
    layout::LayoutNode,
    resources::{Dependencies, ResourceId},
};

pub(crate) enum UndoRecord {
    Marker {
        id: u8,
        time: Option<num::BigInt>,
        lists: Vec<crate::CanvasState>,
    },
    Items(Box<crate::CanvasState>),
    Resources(Box<ResourceChange>),
    Move {
        tile: TileId,
        before: crate::tiles::placement::TileLocation,
        after: crate::tiles::placement::TileLocation,
    },
    SetLayout {
        before: Option<LayoutNode>,
        after: Option<LayoutNode>,
    },
    Title {
        tile: TileId,
        before: Option<String>,
        after: Option<String>,
    },
    Settings {
        tile: TileId,
        before: TileSettings,
        after: TileSettings,
    },
}

impl UndoRecord {
    pub(crate) fn label(&self) -> &str {
        match self {
            Self::Marker { lists, .. } => lists.first().map_or("Edit marker", |list| &list.message),
            Self::Items(items) => &items.message,
            Self::Resources(change) => change.label,
            Self::Move { .. } => "Move tile",
            Self::SetLayout { .. } => "Replace layout",
            Self::Title { .. } => "Rename tile",
            Self::Settings { before, .. } => before.label(),
        }
    }
}

use crate::tiles::{ItemListId, commands::WorkspaceCommand, kind::TileEntry, workspace::Workspace};
use std::collections::{BTreeMap, BTreeSet};

/// The record owns only resources absent from the workspace. Swapping it with
/// the live side retains changes to surviving views and avoids duplicate lists.
pub(crate) struct ResourceChange {
    label: &'static str,
    restore_root: Option<LayoutNode>,
    absent_tiles: BTreeMap<TileId, TileEntry>,
    absent_lists: BTreeMap<ItemListId, crate::item_list::ItemList>,
    present_tiles: BTreeSet<TileId>,
    present_lists: BTreeSet<ItemListId>,
}

pub(crate) struct ResourceEditStart {
    change: ResourceChange,
    tile_ids: BTreeSet<TileId>,
    list_ids: BTreeSet<ItemListId>,
}

impl ResourceEditStart {
    pub(crate) fn with_label(mut self, label: &'static str) -> Self {
        self.change.label = label;
        self
    }
    pub(crate) fn capture(workspace: &Workspace, command: &WorkspaceCommand) -> Option<Self> {
        let mut removed = BTreeSet::new();
        let label = match command {
            WorkspaceCommand::CreateTile { .. } | WorkspaceCommand::OpenTile { .. } => "Open tile",
            WorkspaceCommand::SplitTile { .. } => "Split tile",
            WorkspaceCommand::CloseTile(tile) => {
                removed.insert(*tile);
                "Close tile"
            }
            WorkspaceCommand::CloseOtherTiles(tile) => {
                let mut next = *tile;
                while let Some(id) = workspace.layout.next_in_group(next, 1) {
                    if id == *tile || !removed.insert(id) {
                        break;
                    }
                    next = id;
                }
                "Close other tabs"
            }
            WorkspaceCommand::Reset { keep } => {
                removed.extend(workspace.tiles.keys().filter(|id| Some(**id) != *keep));
                "Reset workspace"
            }
            _ => return None,
        };
        let survivors = workspace
            .tiles
            .iter()
            .filter(|(id, _)| !removed.contains(id))
            .map(|(_, entry)| &entry.kind)
            .collect::<Vec<_>>();
        let dependencies = Dependencies::union(survivors.iter().map(|kind| kind.dependencies()));
        Some(Self {
            change: ResourceChange {
                label,
                restore_root: workspace.layout.root().cloned(),
                absent_tiles: workspace
                    .tiles
                    .iter()
                    .filter(|(id, _)| removed.contains(id))
                    .map(|(id, entry)| (*id, entry.clone()))
                    .collect(),
                absent_lists: workspace
                    .item_lists
                    .iter()
                    .filter(|(id, _)| {
                        !removed.is_empty() && !dependencies.retains(ResourceId::ItemList(**id))
                    })
                    .map(|(id, list)| (*id, list.copy_content()))
                    .collect(),
                present_tiles: BTreeSet::new(),
                present_lists: BTreeSet::new(),
            },
            tile_ids: workspace.tiles.keys().copied().collect(),
            list_ids: workspace.item_lists.keys().copied().collect(),
        })
    }

    pub(crate) fn finish(mut self, workspace: &Workspace) -> Option<UndoRecord> {
        self.change
            .absent_tiles
            .retain(|id, _| !workspace.tiles.contains_key(id));
        self.change
            .absent_lists
            .retain(|id, _| !workspace.item_lists.contains_key(id));
        self.change.present_tiles = workspace
            .tiles
            .keys()
            .filter(|id| !self.tile_ids.contains(id))
            .copied()
            .collect();
        self.change.present_lists = workspace
            .item_lists
            .keys()
            .filter(|id| !self.list_ids.contains(id))
            .copied()
            .collect();
        if self.change.absent_tiles.is_empty() && self.change.present_tiles.is_empty() {
            return None;
        }
        Some(UndoRecord::Resources(Box::new(self.change)))
    }
}

impl ResourceChange {
    fn restore(&mut self, workspace: &mut Workspace) -> bool {
        if self
            .present_tiles
            .iter()
            .any(|id| !workspace.tiles.contains_key(id))
            || self
                .present_lists
                .iter()
                .any(|id| !workspace.item_lists.contains_key(id))
            || self
                .absent_tiles
                .keys()
                .any(|id| workspace.tiles.contains_key(id))
            || self
                .absent_lists
                .keys()
                .any(|id| workspace.item_lists.contains_key(id))
        {
            return false;
        }
        let ids = workspace
            .tiles
            .keys()
            .filter(|id| !self.present_tiles.contains(id))
            .chain(self.absent_tiles.keys())
            .copied()
            .collect();
        let list_ids = workspace
            .item_lists
            .keys()
            .filter(|id| !self.present_lists.contains(id))
            .chain(self.absent_lists.keys())
            .copied()
            .map(ResourceId::ItemList)
            .collect::<BTreeSet<_>>();
        if workspace
            .tiles
            .iter()
            .filter(|(id, _)| !self.present_tiles.contains(id))
            .map(|(_, tile)| tile)
            .chain(self.absent_tiles.values())
            .any(|tile| tile.kind.dependencies().validate(&list_ids).is_err())
        {
            return false;
        }
        let mut root = self.restore_root.clone();
        if let (Some(restored), Some(current)) = (root.as_mut(), workspace.layout.root()) {
            let changed_tiles = self
                .present_tiles
                .iter()
                .chain(self.absent_tiles.keys())
                .copied()
                .collect();
            retain_navigation(restored, current, &changed_tiles);
        }
        let mut layout = workspace.layout.clone();
        if layout.restore_topology(root, &ids).is_err() {
            return false;
        }
        let removed_tiles = self
            .present_tiles
            .iter()
            .map(|id| (*id, workspace.tiles.remove(id).unwrap()))
            .collect();
        let removed_lists = self
            .present_lists
            .iter()
            .map(|id| (*id, workspace.item_lists.remove(id).unwrap()))
            .collect();
        self.present_tiles = self.absent_tiles.keys().copied().collect();
        self.present_lists = self.absent_lists.keys().copied().collect();
        for tile in self.absent_tiles.values_mut() {
            tile.kind.reset_runtime();
        }
        for list in self.absent_lists.values_mut() {
            *list.layout_cache.get_mut() = Default::default();
            list.flattened_rows_cache.get_mut().clear();
        }
        workspace.tiles.append(&mut self.absent_tiles);
        workspace.item_lists.append(&mut self.absent_lists);
        self.absent_tiles = removed_tiles;
        self.absent_lists = removed_lists;
        self.restore_root = workspace.layout.root().cloned();
        workspace.layout = layout;
        workspace.reconcile_waveform_scroll();
        true
    }
}

use crate::tiles::layout::same_topology;

/// Match surviving containers by stable child membership, independent of order.
/// Only reconstructed containers use the shares and active tab from history.
fn retain_layout_navigation(restored: &mut LayoutNode, current: &LayoutNode) {
    retain_navigation(restored, current, &BTreeSet::new());
}
pub(crate) fn retain_move_navigation(
    restored: &mut LayoutNode,
    current: &LayoutNode,
    tile: TileId,
) {
    retain_navigation(restored, current, &BTreeSet::from([tile]));
}
fn retain_navigation(restored: &mut LayoutNode, current: &LayoutNode, ignored: &BTreeSet<TileId>) {
    use std::collections::BTreeMap;
    fn ids(node: &LayoutNode, ignored: &BTreeSet<TileId>) -> Vec<TileId> {
        let mut result = match node {
            LayoutNode::Tile(id) => vec![*id],
            LayoutNode::Tabs { children, .. } | LayoutNode::Split { children, .. } => children
                .iter()
                .flat_map(|child| ids(child, ignored))
                .collect(),
        };
        result.retain(|id| !ignored.contains(id));
        result.sort_unstable();
        result
    }
    type SplitKey = (u8, Vec<Vec<TileId>>);
    #[derive(Default)]
    struct Navigation {
        splits: BTreeMap<SplitKey, BTreeMap<Vec<TileId>, f32>>,
        tabs: BTreeMap<Vec<TileId>, TileId>,
    }
    fn split_key(
        dir: crate::tiles::layout::SplitDir,
        children: &[LayoutNode],
        ignored: &BTreeSet<TileId>,
    ) -> SplitKey {
        let mut groups = children
            .iter()
            .map(|child| ids(child, ignored))
            .collect::<Vec<_>>();
        groups.sort();
        (
            match dir {
                crate::tiles::layout::SplitDir::Horizontal => 0,
                crate::tiles::layout::SplitDir::Vertical => 1,
            },
            groups,
        )
    }
    fn capture(node: &LayoutNode, navigation: &mut Navigation, ignored: &BTreeSet<TileId>) {
        match node {
            LayoutNode::Tile(_) => {}
            LayoutNode::Tabs { active, children } => {
                if let Some(LayoutNode::Tile(id)) = children.get(*active) {
                    navigation.tabs.insert(ids(node, ignored), *id);
                }
            }
            LayoutNode::Split {
                dir,
                shares,
                children,
            } => {
                let key = split_key(*dir, children, ignored);
                if key.1.iter().all(|members| !members.is_empty()) {
                    navigation.splits.insert(
                        key,
                        children
                            .iter()
                            .map(|child| ids(child, ignored))
                            .zip(shares.iter().copied())
                            .collect(),
                    );
                }
                for child in children {
                    capture(child, navigation, ignored);
                }
            }
        }
    }
    fn apply(node: &mut LayoutNode, navigation: &Navigation, ignored: &BTreeSet<TileId>) {
        let members = ids(node, ignored);
        match node {
            LayoutNode::Tile(_) => {}
            LayoutNode::Tabs { active, children } => {
                if let Some(id) = navigation.tabs.get(&members)
                    && let Some(index) = children
                        .iter()
                        .position(|child| matches!(child, LayoutNode::Tile(tile) if tile == id))
                {
                    *active = index;
                }
            }
            LayoutNode::Split {
                dir,
                shares,
                children,
            } => {
                if let Some(weights) = navigation.splits.get(&split_key(*dir, children, ignored)) {
                    *shares = children
                        .iter()
                        .map(|child| weights[&ids(child, ignored)])
                        .collect();
                }
                for child in children {
                    apply(child, navigation, ignored);
                }
            }
        }
    }
    let mut navigation = Navigation::default();
    capture(current, &mut navigation, ignored);
    apply(restored, &navigation, ignored);
}

pub(crate) fn move_record(
    tile: TileId,
    before: &LayoutNode,
    after: &LayoutNode,
) -> Option<UndoRecord> {
    if same_topology(Some(before), Some(after)) {
        return None;
    }
    Some(UndoRecord::Move {
        tile,
        before: crate::tiles::placement::TileLocation::capture(before, tile)?,
        after: crate::tiles::placement::TileLocation::capture(after, tile)?,
    })
}

impl crate::SystemState {
    pub(crate) fn record_edit(&mut self, record: UndoRecord) {
        self.undo_stack.push(record);
        if self.undo_stack.len() > self.user.config.undo_stack_size {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    pub(crate) fn restore_history(
        &mut self,
        record: UndoRecord,
        redo: bool,
    ) -> Result<UndoRecord, UndoRecord> {
        match record {
            UndoRecord::Marker { id, time, lists } => {
                if self.user.waves.is_none()
                    || lists
                        .iter()
                        .any(|list| !self.user.workspace.item_lists.contains_key(&list.list))
                {
                    return Err(UndoRecord::Marker { id, time, lists });
                }
                let markers = &mut self.user.waves.as_mut().unwrap().markers;
                let inverse_time = match time {
                    Some(time) => markers.insert(id, time),
                    None => markers.remove(&id),
                };
                let inverse_lists = lists
                    .into_iter()
                    .map(|list| {
                        // All resource dependencies were checked before mutating either side.
                        match self.restore_canvas_state(list) {
                            Ok(inverse) => inverse,
                            Err(_) => unreachable!("validated marker history list"),
                        }
                    })
                    .collect();
                self.invalidate_draw_commands();
                Ok(UndoRecord::Marker {
                    id,
                    time: inverse_time,
                    lists: inverse_lists,
                })
            }
            UndoRecord::Move {
                tile,
                mut before,
                mut after,
            } => {
                let restored = (|| {
                    let layout = &mut self.user.workspace.layout;
                    let current = layout.root()?;
                    let inverse = crate::tiles::placement::TileLocation::capture(current, tile)?;
                    let root = if redo {
                        after.restore(current)?
                    } else {
                        before.restore(current)?
                    };
                    layout
                        .apply_proposal(layout.revision(), Some(root), layout.focused())
                        .ok()?;
                    if redo {
                        before = inverse;
                    } else {
                        after = inverse;
                    }
                    Some(())
                })()
                .is_some();
                let record = UndoRecord::Move {
                    tile,
                    before,
                    after,
                };
                if restored {
                    self.user.workspace.reconcile_waveform_scroll();
                    Ok(record)
                } else {
                    Err(record)
                }
            }
            UndoRecord::Resources(mut change) => {
                let restored = change.restore(&mut self.user.workspace);
                if restored {
                    for tile in &change.present_tiles {
                        self.attach_tile_source(*tile);
                    }
                }
                let record = UndoRecord::Resources(change);
                if restored { Ok(record) } else { Err(record) }
            }
            UndoRecord::SetLayout {
                ref before,
                ref after,
            } => {
                let mut root = if redo { after.clone() } else { before.clone() };
                let layout = &mut self.user.workspace.layout;
                if let (Some(restored), Some(current)) = (root.as_mut(), layout.root()) {
                    retain_layout_navigation(restored, current);
                }
                if layout
                    .apply_proposal(layout.revision(), root, layout.focused())
                    .is_err()
                {
                    return Err(record);
                }
                self.user.workspace.reconcile_waveform_scroll();
                Ok(record)
            }
            UndoRecord::Title {
                tile,
                ref before,
                ref after,
            } => {
                let Some(entry) = self.user.workspace.tiles.get_mut(&tile) else {
                    return Err(record);
                };
                entry.title = if redo { after.clone() } else { before.clone() };
                Ok(record)
            }
            UndoRecord::Items(items) => self
                .restore_canvas_state(*items)
                .map(|inverse| UndoRecord::Items(Box::new(inverse)))
                .map_err(UndoRecord::Items),
            UndoRecord::Settings {
                tile,
                ref before,
                ref after,
            } => {
                let settings = if redo { after } else { before };
                let matches_kind = self
                    .user
                    .workspace
                    .tiles
                    .get(&tile)
                    .is_some_and(|entry| settings.capture_like(&entry.kind).is_some());
                let source_changed = self
                    .user
                    .workspace
                    .tiles
                    .get(&tile)
                    .is_some_and(|entry| settings.source_changed(&entry.kind));
                let restored = matches_kind
                    && self
                        .user
                        .workspace
                        .apply_tile_message(
                            tile,
                            settings.restore_message(),
                            self.user.waves.as_ref(),
                        )
                        .is_ok();
                if restored {
                    if source_changed {
                        self.attach_tile_source(tile);
                    }
                    Ok(record)
                } else {
                    Err(record)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        Message, SystemState,
        tile_kinds::{
            annotation_list::AnnotationListMessage,
            logs::{LevelFilter, LogsMessage},
        },
        tiles::{
            commands::WorkspaceCommand,
            kind::{TileKind, TileMessage},
            layout::{Direction, Placement},
        },
    };

    fn open(state: &mut SystemState, kind: &str) -> super::TileId {
        state
            .update(Message::Workspace(WorkspaceCommand::OpenTile {
                kind: kind.into(),
                placement: Placement::Edge(Direction::Right),
                focus: true,
            }))
            .unwrap();
        state.user.workspace.layout.focused().unwrap()
    }

    #[tokio::test]
    async fn inspector_source_commands_load_signals_and_survive_undo_redo() {
        use crate::{
            StartupParams, WaveSource,
            frame_buffer::FrameBufferContent,
            tile_kinds::{
                frame_buffer::{FrameBufferMessage, FrameBufferState},
                memory::MemoryMessage,
            },
            wave_container::{ScopeRef, ScopeRefExt, VariableRef, VariableRefExt},
        };
        async fn settle(state: &mut SystemState) {
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    state.handle_async_messages();
                    state.handle_batch_commands();
                    if state.waves_fully_loaded() {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                }
            })
            .await
            .expect("signal loading timed out");
        }
        for kind in ["memory", "frame_buffer"] {
            let mut state = SystemState::new_default_config()
                .unwrap()
                .with_params(StartupParams {
                    waves: Some(WaveSource::File(
                        project_root::get_project_root()
                            .unwrap()
                            .join("examples/counter.vcd")
                            .try_into()
                            .unwrap(),
                    )),
                    ..Default::default()
                });
            settle(&mut state).await;
            let variable = VariableRef::from_hierarchy_string("tb.dut.counter");
            let loaded = |state: &SystemState| {
                let waves = state.user.waves.as_ref().unwrap().inner.as_waves().unwrap();
                waves.is_signal_loaded(&waves.signal_id(&variable).unwrap())
            };
            assert!(
                !loaded(&state),
                "fixture must begin with unloaded signal data"
            );
            let target = open(&mut state, kind);
            let message = if kind == "memory" {
                let mut settings = crate::memory_viewer::MemoryViewerSettings::default();
                settings.scope = Some(ScopeRef::from_hierarchy_string("tb.dut"));
                TileMessage::Memory(MemoryMessage::Settings(Box::new(settings)))
            } else {
                TileMessage::FrameBuffer(FrameBufferMessage::State(Box::new(FrameBufferState {
                    content: Some(FrameBufferContent::Variable(variable.clone())),
                    ..Default::default()
                })))
            };
            state.update(Message::ToTile(target, message)).unwrap();
            settle(&mut state).await;
            assert!(
                loaded(&state),
                "{kind} source command must request its signal data"
            );
            let has_source = |state: &SystemState| match &state.user.workspace.tiles[&target].kind {
                TileKind::Memory(tile) => tile.settings.scope.is_some(),
                TileKind::FrameBuffer(tile) => tile.state.content.is_some(),
                _ => unreachable!(),
            };
            state.update(Message::Undo(1)).unwrap();
            assert!(!has_source(&state));
            state.update(Message::Redo(1)).unwrap();
            settle(&mut state).await;
            assert!(has_source(&state));
            assert!(loaded(&state));

            // Model a restored inspector whose source has not been fetched yet.
            // Resource replay must use the same attachment path as settings replay.
            let pending = VariableRef::from_hierarchy_string("tb._tmp");
            let pending_loaded = |state: &SystemState| {
                let waves = state.user.waves.as_ref().unwrap().inner.as_waves().unwrap();
                waves.is_signal_loaded(&waves.signal_id(&pending).unwrap())
            };
            assert!(!pending_loaded(&state));
            match &mut state.user.workspace.tiles.get_mut(&target).unwrap().kind {
                TileKind::Memory(tile) => {
                    tile.settings.scope = Some(ScopeRef::from_hierarchy_string("tb"))
                }
                TileKind::FrameBuffer(tile) => {
                    tile.state.content = Some(FrameBufferContent::Variable(pending.clone()))
                }
                _ => unreachable!(),
            }
            state
                .update(Message::Workspace(WorkspaceCommand::CloseTile(target)))
                .unwrap();
            assert!(!state.user.workspace.tiles.contains_key(&target));
            state.update(Message::Undo(1)).unwrap();
            settle(&mut state).await;
            assert!(state.user.workspace.tiles.contains_key(&target));
            assert!(
                pending_loaded(&state),
                "{kind} resource restoration must attach its source"
            );
        }
    }

    #[test]
    fn reset_is_one_record_that_restores_tiles_lists_and_layout() {
        use crate::tiles::layout::same_topology;
        use crate::{
            displayed_item_tree::{ItemIndex, TargetPosition},
            tile_kinds::waveform::WaveformMessage,
        };
        let mut state = SystemState::new_default_config().unwrap();
        let first = open(&mut state, "waveform");
        state
            .update(Message::ToTile(
                first,
                TileMessage::Waveform(WaveformMessage::AddDivider {
                    name: Some("row".into()),
                    position: TargetPosition {
                        before: ItemIndex(0),
                        level: 0,
                    },
                }),
            ))
            .unwrap();
        let second = open(&mut state, "waveform");
        let logs = open(&mut state, "logs");
        state
            .update(Message::Workspace(WorkspaceCommand::FocusTile(first)))
            .unwrap();
        let root_before = state.user.workspace.layout.root().cloned();
        let history = state.undo_stack.len();
        let reset = state.user.workspace.reset_command();
        assert!(matches!(reset, WorkspaceCommand::Reset { keep: Some(id) } if id == first));
        state.update(Message::Workspace(reset)).unwrap();
        assert_eq!(state.undo_stack.len(), history + 1);
        assert_eq!(state.undo_stack.last().unwrap().label(), "Reset workspace");
        assert_eq!(
            state
                .user
                .workspace
                .tiles
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            [first]
        );
        assert_eq!(state.user.workspace.item_lists.len(), 1);
        state.update(Message::Undo(1)).unwrap();
        for id in [first, second, logs] {
            assert!(state.user.workspace.tiles.contains_key(&id));
        }
        assert_eq!(state.user.workspace.item_lists.len(), 2);
        assert!(same_topology(
            state.user.workspace.layout.root(),
            root_before.as_ref()
        ));
        let list = state.user.workspace.tiles[&first]
            .kind
            .waveform_list()
            .unwrap();
        assert_eq!(state.user.workspace.item_lists[&list].items_tree.len(), 1);
        state.update(Message::Redo(1)).unwrap();
        assert_eq!(state.user.workspace.tiles.len(), 1);
        // Resetting an empty workspace creates a fresh waveform; undo removes it again.
        state
            .update(Message::Workspace(WorkspaceCommand::CloseTile(first)))
            .unwrap();
        assert!(state.user.workspace.tiles.is_empty());
        let reset = state.user.workspace.reset_command();
        assert!(matches!(reset, WorkspaceCommand::Reset { keep: None }));
        state.update(Message::Workspace(reset)).unwrap();
        let fresh = state.user.workspace.layout.focused().unwrap();
        assert!(fresh != first && fresh != second);
        assert_eq!(state.user.workspace.item_lists.len(), 1);
        state.update(Message::Undo(1)).unwrap();
        assert!(state.user.workspace.tiles.is_empty());
        assert!(state.user.workspace.item_lists.is_empty());
        state.update(Message::Redo(1)).unwrap();
        assert_eq!(state.user.workspace.layout.focused(), Some(fresh));
    }

    #[test]
    fn marker_edits_record_only_their_marker_and_item_edits_keep_marker_times() {
        use crate::{
            data_container::DataContainer,
            wave_data::{TimeRange, WaveData},
            wave_source::{WaveFormat, WaveSource},
        };
        let mut state = SystemState::new_default_config().unwrap();
        state.user.waves = Some(WaveData {
            inner: DataContainer::Empty,
            source: WaveSource::Data,
            format: WaveFormat::Vcd,
            active_scope: None,
            cursor: Some(40.into()),
            markers: Default::default(),
            display_variable_indices: false,
            old_max_timestamp: None,
            cache_generation: 0,
            inflight_caches: Default::default(),
            cached_time_range: TimeRange::default(),
        });
        let waveform = open(&mut state, "waveform");
        let list = state.user.workspace.tiles[&waveform]
            .kind
            .waveform_list()
            .unwrap();
        let rows = |state: &SystemState| state.user.workspace.item_lists[&list].items_tree.len();
        let marker = |state: &SystemState, id: u8| {
            state.user.waves.as_ref().unwrap().markers.get(&id).cloned()
        };
        state
            .update(Message::AddMarker {
                time: 10.into(),
                name: Some("start".into()),
                move_focus: false,
            })
            .unwrap();
        state
            .update(Message::SetMarker {
                id: 5,
                time: 50.into(),
            })
            .unwrap();
        assert_eq!(rows(&state), 2);
        assert_eq!(state.undo_stack.len(), 3);
        // Re-setting the same time is a no-op without a record.
        assert!(
            state
                .update(Message::SetMarker {
                    id: 5,
                    time: 50.into(),
                })
                .is_none()
        );
        assert_eq!(state.undo_stack.len(), 3);
        state.update(Message::AddDivider(None, None)).unwrap();
        assert_eq!(rows(&state), 3);
        // A marker changed by an unrelated path survives undoing the divider and marker 5.
        state
            .user
            .waves
            .as_mut()
            .unwrap()
            .markers
            .insert(7, 99.into());
        state.update(Message::Undo(2)).unwrap();
        assert_eq!(rows(&state), 1);
        assert_eq!(marker(&state, 5), None);
        assert_eq!(marker(&state, 7), Some(99.into()));
        assert_eq!(marker(&state, 0), Some(10.into()));
        state.update(Message::Redo(1)).unwrap();
        assert_eq!(marker(&state, 5), Some(50.into()));
        assert_eq!(marker(&state, 7), Some(99.into()));
        assert_eq!(rows(&state), 2);
        state.update(Message::MoveMarkerToCursor(5)).unwrap();
        assert_eq!(marker(&state, 5), Some(40.into()));
        assert_eq!(
            state.undo_stack.last().unwrap().label(),
            "Move marker 5 to cursor"
        );
        state.update(Message::Undo(1)).unwrap();
        assert_eq!(marker(&state, 5), Some(50.into()));
        state.update(Message::Undo(2)).unwrap();
        assert_eq!(rows(&state), 0);
        assert_eq!(marker(&state, 0), None);
        assert_eq!(marker(&state, 7), Some(99.into()));
        assert_eq!(state.user.waves.as_ref().unwrap().markers.len(), 1);
    }

    #[test]
    fn shared_marker_deletion_restores_all_lists_without_rewinding_other_markers_or_focus() {
        use crate::{
            data_container::DataContainer,
            wave_data::{TimeRange, WaveData},
            wave_source::{WaveFormat, WaveSource},
        };
        let mut state = SystemState::new_default_config().unwrap();
        state.user.waves = Some(WaveData {
            inner: DataContainer::Empty,
            source: WaveSource::Data,
            format: WaveFormat::Vcd,
            active_scope: None,
            cursor: None,
            markers: Default::default(),
            display_variable_indices: false,
            old_max_timestamp: None,
            cache_generation: 0,
            inflight_caches: Default::default(),
            cached_time_range: TimeRange::default(),
        });
        let first = open(&mut state, "waveform");
        state
            .update(Message::SetMarker {
                id: 7,
                time: 100.into(),
            })
            .unwrap();
        state
            .update(Message::Workspace(WorkspaceCommand::SplitTile {
                tile: first,
                dir: Direction::Right,
                mode: crate::tiles::commands::SplitMode::Independent,
            }))
            .unwrap();
        let second = state.user.workspace.layout.focused().unwrap();
        state
            .update(Message::Workspace(WorkspaceCommand::SplitTile {
                tile: second,
                dir: Direction::Down,
                mode: crate::tiles::commands::SplitMode::Linked,
            }))
            .unwrap();
        assert_eq!(state.user.workspace.item_lists.len(), 2);
        let inspector = open(&mut state, "markers");
        let has_row = |items: &crate::item_list::ItemList| {
            items.displayed_items.values().any(|item| {
            matches!(item, crate::displayed_item::DisplayedItem::Marker(marker) if marker.idx == 7)
        })
        };
        assert!(state.user.workspace.item_lists.values().all(has_row));
        let history_len = state.undo_stack.len();
        state.update(Message::RemoveMarker(7)).unwrap();
        assert_eq!(state.undo_stack.len(), history_len + 1);
        assert!(
            state
                .user
                .workspace
                .item_lists
                .values()
                .all(|items| !has_row(items))
        );
        assert!(!state.user.waves.as_ref().unwrap().markers.contains_key(&7));
        // Unrelated document changes must survive replay of this one marker edit.
        state
            .user
            .waves
            .as_mut()
            .unwrap()
            .markers
            .insert(9, 900.into());
        state.update(Message::Undo(1)).unwrap();
        assert!(state.user.workspace.item_lists.values().all(has_row));
        assert_eq!(state.user.waves.as_ref().unwrap().markers[&7], 100.into());
        assert_eq!(state.user.waves.as_ref().unwrap().markers[&9], 900.into());
        assert_eq!(state.user.workspace.layout.focused(), Some(inspector));
        let redo_len = state.redo_stack.len();
        assert!(state.update(Message::RemoveMarker(99)).is_none());
        assert_eq!(state.redo_stack.len(), redo_len);
        state.update(Message::Redo(1)).unwrap();
        assert!(
            state
                .user
                .workspace
                .item_lists
                .values()
                .all(|items| !has_row(items))
        );
        assert_eq!(state.user.waves.as_ref().unwrap().markers[&9], 900.into());
        assert_eq!(state.user.workspace.layout.focused(), Some(inspector));
    }

    #[test]
    fn graphic_history_targets_its_list_and_preserves_navigation_and_redo_on_noops() {
        use crate::{
            displayed_item_tree::{ItemIndex, TargetPosition},
            graphics::{
                Anchor, Direction as GraphicDirection, GrPoint, Graphic, GraphicId, GraphicsY,
            },
            tile_kinds::waveform::WaveformMessage,
        };
        let mut state = SystemState::new_default_config().unwrap();
        let first = open(&mut state, "waveform");
        state
            .update(Message::ToTile(
                first,
                TileMessage::Waveform(WaveformMessage::AddDivider {
                    name: Some("Notes".into()),
                    position: TargetPosition {
                        before: ItemIndex(0),
                        level: 0,
                    },
                }),
            ))
            .unwrap();
        let list = state.user.workspace.tiles[&first]
            .kind
            .waveform_list()
            .unwrap();
        let row = *state.user.workspace.item_lists[&list]
            .displayed_items
            .keys()
            .next()
            .unwrap();
        let second = open(&mut state, "waveform");
        let graphic = Graphic::Text {
            pos: (
                GrPoint {
                    x: 10.into(),
                    y: GraphicsY {
                        item: row,
                        anchor: Anchor::Center,
                    },
                },
                GraphicDirection::North,
            ),
            text: "A note".into(),
        };
        state
            .update(Message::ToTile(
                first,
                TileMessage::Waveform(WaveformMessage::AddGraphic(GraphicId(1), graphic.clone())),
            ))
            .unwrap();
        let history_len = state.undo_stack.len();
        state
            .update(Message::ToTile(
                first,
                TileMessage::Waveform(WaveformMessage::AddGraphic(GraphicId(1), graphic.clone())),
            ))
            .unwrap();
        assert_eq!(state.undo_stack.len(), history_len);
        state
            .update(Message::ToTile(
                first,
                TileMessage::Waveform(WaveformMessage::ColumnWidths {
                    names: 250.0,
                    values: 80.0,
                }),
            ))
            .unwrap();
        state.update(Message::Undo(1)).unwrap();
        assert!(state.user.workspace.item_lists[&list].graphics.is_empty());
        let redo_len = state.redo_stack.len();
        state
            .update(Message::ToTile(
                first,
                TileMessage::Waveform(WaveformMessage::RemoveGraphic(GraphicId(99))),
            ))
            .unwrap();
        assert_eq!(state.redo_stack.len(), redo_len);
        state.update(Message::Redo(1)).unwrap();
        assert_eq!(
            state.user.workspace.item_lists[&list].graphics[&GraphicId(1)],
            graphic
        );
        assert_eq!(state.user.workspace.layout.focused(), Some(second));
        let TileKind::Waveform(tile) = &state.user.workspace.tiles[&first].kind else {
            unreachable!()
        };
        assert_eq!(tile.name_column_width, 250.0);
        state
            .update(Message::ToTile(
                first,
                TileMessage::Waveform(WaveformMessage::RemoveGraphic(GraphicId(1))),
            ))
            .unwrap();
        assert!(state.user.workspace.item_lists[&list].graphics.is_empty());
        state.update(Message::Undo(1)).unwrap();
        assert_eq!(
            state.user.workspace.item_lists[&list].graphics[&GraphicId(1)],
            graphic
        );
    }

    #[test]
    fn moves_restore_collapsed_nested_groups_for_every_direction_and_tab_position() {
        use super::{LayoutNode, same_topology};
        use crate::tiles::layout::SplitDir;
        for source in 0..4 {
            for destination in 0..4 {
                if source == destination {
                    continue;
                }
                for direction in [
                    None,
                    Some(Direction::Left),
                    Some(Direction::Right),
                    Some(Direction::Up),
                    Some(Direction::Down),
                ] {
                    let mut state = SystemState::new_default_config().unwrap();
                    let ids = (0..4)
                        .map(|_| open(&mut state, "waveform"))
                        .collect::<Vec<_>>();
                    let root = LayoutNode::Split {
                        dir: SplitDir::Vertical,
                        shares: vec![0.3, 0.7],
                        children: vec![
                            LayoutNode::Split {
                                dir: SplitDir::Horizontal,
                                shares: vec![0.6, 0.4],
                                children: vec![
                                    LayoutNode::Tabs {
                                        active: 0,
                                        children: vec![
                                            LayoutNode::Tile(ids[0]),
                                            LayoutNode::Tile(ids[1]),
                                        ],
                                    },
                                    LayoutNode::Tile(ids[2]),
                                ],
                            },
                            LayoutNode::Tile(ids[3]),
                        ],
                    };
                    state
                        .user
                        .workspace
                        .apply_command(
                            &mut state.workspace_runtime,
                            WorkspaceCommand::SetLayout(Some(root)),
                        )
                        .unwrap();
                    state.undo_stack.clear();
                    let original = state.user.workspace.layout.root().cloned().unwrap();
                    let placement = direction
                        .map_or(Placement::TabAfter(ids[destination]), |dir| {
                            Placement::Beside(ids[destination], dir)
                        });
                    state
                        .update(Message::Workspace(WorkspaceCommand::MoveTile {
                            tile: ids[source],
                            to: placement,
                        }))
                        .unwrap();
                    let moved = state.user.workspace.layout.root().cloned().unwrap();
                    if same_topology(Some(&original), Some(&moved)) {
                        assert!(state.undo_stack.is_empty());
                        continue;
                    }
                    assert_eq!(state.undo_stack.len(), 1);
                    let revision = state.user.workspace.layout.revision();
                    state.update(Message::Undo(1)).unwrap();
                    assert!(
                        same_topology(state.user.workspace.layout.root(), Some(&original)),
                        "undo {source} -> {destination} {direction:?}"
                    );
                    assert!(state.user.workspace.layout.revision() > revision);
                    state.update(Message::Redo(1)).unwrap();
                    assert!(
                        same_topology(state.user.workspace.layout.root(), Some(&moved)),
                        "redo {source} -> {destination} {direction:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn adapter_and_command_moves_have_the_same_history_and_preserve_later_resizing() {
        use super::{LayoutNode, same_topology};
        use crate::tiles::{layout::SplitDir, render::LayoutEdit};
        let mut state = SystemState::new_default_config().unwrap();
        let ids = (0..4)
            .map(|_| open(&mut state, "waveform"))
            .collect::<Vec<_>>();
        let tabs = |indices: &[usize]| LayoutNode::Tabs {
            active: 0,
            children: indices.iter().map(|i| LayoutNode::Tile(ids[*i])).collect(),
        };
        state
            .user
            .workspace
            .apply_command(
                &mut state.workspace_runtime,
                WorkspaceCommand::SetLayout(Some(LayoutNode::Split {
                    dir: SplitDir::Horizontal,
                    shares: vec![0.5, 0.5],
                    children: vec![tabs(&[0, 1]), tabs(&[2, 3])],
                })),
            )
            .unwrap();
        state.undo_stack.clear();
        let original = state.user.workspace.layout.root().cloned();
        let mut proposed = state.user.workspace.layout.clone();
        proposed
            .move_tile(ids[1], Placement::TabAfter(ids[2]))
            .unwrap();
        state
            .update(Message::ApplyLayoutProposal(LayoutEdit {
                revision: state.user.workspace.layout.revision(),
                root: proposed.root().cloned(),
                focused: Some(ids[3]),
                structural: true,
                moved_tile: Some(ids[1]),
            }))
            .unwrap();
        assert_eq!(state.undo_stack.len(), 1);
        assert_eq!(state.undo_stack[0].label(), "Move tile");
        let mut resized = state.user.workspace.layout.root().cloned().unwrap();
        let LayoutNode::Split { shares, .. } = &mut resized else {
            panic!()
        };
        *shares = vec![0.8, 0.2];
        state
            .update(Message::ApplyLayoutProposal(LayoutEdit {
                revision: state.user.workspace.layout.revision(),
                root: Some(resized),
                focused: Some(ids[3]),
                structural: false,
                moved_tile: None,
            }))
            .unwrap();
        state.update(Message::Undo(1)).unwrap();
        assert!(same_topology(
            state.user.workspace.layout.root(),
            original.as_ref()
        ));
        let LayoutNode::Split { shares, .. } = state.user.workspace.layout.root().unwrap() else {
            panic!()
        };
        assert_eq!(shares, &[0.8, 0.2]);
        assert_eq!(state.user.workspace.layout.focused(), Some(ids[3]));
        assert_eq!(state.redo_stack.len(), 1);
        state
            .update(Message::Workspace(WorkspaceCommand::MoveTile {
                tile: ids[1],
                to: Placement::TabAfter(ids[2]),
            }))
            .unwrap();
        assert_eq!(state.undo_stack.len(), 1);
        assert_eq!(state.undo_stack[0].label(), "Move tile");
        assert!(same_topology(
            state.user.workspace.layout.root(),
            proposed.root()
        ));
        assert!(state.redo_stack.is_empty());
    }

    #[test]
    fn linked_scrolling_undo_preserves_offsets_and_redo_joins_the_current_group() {
        use crate::{tile_kinds::waveform::WaveformMessage, tiles::commands::SplitMode};
        let mut state = SystemState::new_default_config().unwrap();
        let first = open(&mut state, "waveform");
        state
            .update(Message::Workspace(WorkspaceCommand::SplitTile {
                tile: first,
                dir: Direction::Right,
                mode: SplitMode::Linked,
            }))
            .unwrap();
        let second = state.user.workspace.layout.focused().unwrap();
        let list = state.user.workspace.tiles[&first]
            .kind
            .waveform_list()
            .unwrap();
        state.user.workspace.item_lists[&list]
            .layout_cache
            .borrow_mut()
            .total_height = 1000.0;
        for id in [first, second] {
            let TileKind::Waveform(tile) =
                &mut state.user.workspace.tiles.get_mut(&id).unwrap().kind
            else {
                panic!()
            };
            tile.view.viewport_height = 100.0;
        }
        let command = |id, message| Message::ToTile(id, TileMessage::Waveform(message));
        state
            .update(command(first, WaveformMessage::LinkVerticalScroll(true)))
            .unwrap();
        state
            .update(command(first, WaveformMessage::ScrollTo(100.0)))
            .unwrap();
        let history = state.undo_stack.len();
        state
            .update(command(second, WaveformMessage::LinkVerticalScroll(true)))
            .unwrap();
        state
            .update(command(second, WaveformMessage::ScrollTo(250.0)))
            .unwrap();
        assert_eq!(state.undo_stack.len(), history + 1);
        state.update(Message::Undo(1)).unwrap();
        let TileKind::Waveform(tile) = &state.user.workspace.tiles[&second].kind else {
            panic!()
        };
        assert!(!tile.link_vertical_scroll);
        assert_eq!(tile.view.scroll_offset, 250.0);
        state
            .update(command(first, WaveformMessage::ScrollTo(400.0)))
            .unwrap();
        assert_eq!(state.redo_stack.len(), 1);
        state.update(Message::Redo(1)).unwrap();
        let TileKind::Waveform(tile) = &state.user.workspace.tiles[&second].kind else {
            panic!()
        };
        assert!(tile.link_vertical_scroll);
        assert_eq!(tile.view.scroll_offset, 400.0);
    }

    #[test]
    fn reopening_a_tab_preserves_resizing_of_its_surviving_parent_split() {
        use super::LayoutNode;
        use crate::tiles::layout::SplitDir;
        let mut state = SystemState::new_default_config().unwrap();
        let ids = (0..3)
            .map(|_| open(&mut state, "waveform"))
            .collect::<Vec<_>>();
        state
            .user
            .workspace
            .apply_command(
                &mut state.workspace_runtime,
                WorkspaceCommand::SetLayout(Some(LayoutNode::Split {
                    dir: SplitDir::Horizontal,
                    shares: vec![0.5, 0.5],
                    children: vec![
                        LayoutNode::Tabs {
                            active: 0,
                            children: vec![LayoutNode::Tile(ids[0]), LayoutNode::Tile(ids[1])],
                        },
                        LayoutNode::Tile(ids[2]),
                    ],
                })),
            )
            .unwrap();
        state
            .update(Message::Workspace(WorkspaceCommand::CloseTile(ids[1])))
            .unwrap();
        let mut root = state.user.workspace.layout.root().cloned().unwrap();
        let LayoutNode::Split { shares, .. } = &mut root else {
            panic!()
        };
        *shares = vec![0.7, 0.3];
        state
            .update(Message::Workspace(WorkspaceCommand::SetLayout(Some(root))))
            .unwrap();
        state.update(Message::Undo(1)).unwrap();
        let LayoutNode::Split {
            shares, children, ..
        } = state.user.workspace.layout.root().unwrap()
        else {
            panic!()
        };
        assert_eq!(shares, &[0.7, 0.3]);
        assert!(matches!(&children[0], LayoutNode::Tabs { children, .. } if children.len() == 2));
    }

    #[test]
    fn closing_other_tabs_is_one_record_and_restores_their_order_and_owned_lists() {
        let mut state = SystemState::new_default_config().unwrap();
        let mut ids = Vec::new();
        for _ in 0..3 {
            state
                .user
                .workspace
                .apply_command(
                    &mut state.workspace_runtime,
                    WorkspaceCommand::CreateTile {
                        kind: "waveform".into(),
                        placement: ids
                            .last()
                            .copied()
                            .map_or(Placement::Root, Placement::TabAfter),
                        focus: true,
                    },
                )
                .unwrap();
            ids.push(state.user.workspace.layout.focused().unwrap());
        }
        let outside = open(&mut state, "logs");
        state.undo_stack.clear();
        let before = state.user.workspace.layout.root().cloned();
        state
            .update(Message::Workspace(WorkspaceCommand::CloseOtherTiles(
                ids[1],
            )))
            .unwrap();
        assert_eq!(state.undo_stack.len(), 1);
        assert_eq!(state.user.workspace.tiles.len(), 2);
        assert_eq!(state.user.workspace.item_lists.len(), 1);
        state.update(Message::Undo(1)).unwrap();
        assert!(super::same_topology(
            state.user.workspace.layout.root(),
            before.as_ref()
        ));
        let super::LayoutNode::Split { children, .. } = state.user.workspace.layout.root().unwrap()
        else {
            panic!()
        };
        assert!(
            matches!(&children[0], super::LayoutNode::Tabs { active: 1, .. }),
            "surviving tab activation is retained"
        );
        assert_eq!(state.user.workspace.item_lists.len(), 3);
        assert_eq!(state.user.workspace.layout.focused(), Some(outside));
        state.update(Message::Redo(1)).unwrap();
        let history = state.undo_stack.len();
        state
            .update(Message::Workspace(WorkspaceCommand::CloseOtherTiles(
                ids[1],
            )))
            .unwrap();
        assert_eq!(
            state.undo_stack.len(),
            history,
            "no siblings creates no history"
        );
        assert_eq!(state.user.workspace.layout.focused(), Some(outside));
    }

    #[test]
    fn closing_last_linked_view_restores_owned_list_and_current_survivor_navigation() {
        use crate::tiles::commands::SplitMode;
        let mut state = SystemState::new_default_config().unwrap();
        let first = open(&mut state, "waveform");
        let list = state.user.workspace.tiles[&first]
            .kind
            .waveform_list()
            .unwrap();
        state
            .user
            .workspace
            .item_lists
            .get_mut(&list)
            .unwrap()
            .annotation_counter = 17;
        state
            .update(Message::Workspace(WorkspaceCommand::SplitTile {
                tile: first,
                dir: Direction::Right,
                mode: SplitMode::Linked,
            }))
            .unwrap();
        let second = state.user.workspace.layout.focused().unwrap();
        assert_eq!(
            state.user.workspace.tiles[&second].kind.waveform_list(),
            Some(list)
        );
        let TileKind::Waveform(tile) =
            &mut state.user.workspace.tiles.get_mut(&first).unwrap().kind
        else {
            panic!()
        };
        tile.view.viewport.handle_canvas_scroll(0.3);
        let navigation = ron::to_string(&tile.view.viewport).unwrap();
        state.update(Message::Undo(1)).unwrap();
        assert!(state.user.workspace.item_lists.contains_key(&list));
        let TileKind::Waveform(tile) = &state.user.workspace.tiles[&first].kind else {
            panic!()
        };
        assert_eq!(ron::to_string(&tile.view.viewport).unwrap(), navigation);
        state.update(Message::Redo(1)).unwrap();
        assert_eq!(
            state.user.workspace.tiles[&second].kind.waveform_list(),
            Some(list)
        );
        state
            .update(Message::Workspace(WorkspaceCommand::CloseTile(first)))
            .unwrap();
        assert_eq!(state.user.workspace.item_lists.len(), 1);
        state
            .update(Message::Workspace(WorkspaceCommand::CloseTile(second)))
            .unwrap();
        assert!(state.user.workspace.tiles.is_empty());
        assert!(state.user.workspace.item_lists.is_empty());
        for _ in 0..2 {
            state.update(Message::Undo(2)).unwrap();
            assert_eq!(state.user.workspace.tiles.len(), 2);
            assert_eq!(state.user.workspace.item_lists.len(), 1);
            assert_eq!(
                state.user.workspace.item_lists[&list].annotation_counter,
                17
            );
            let TileKind::Waveform(tile) = &state.user.workspace.tiles[&first].kind else {
                panic!()
            };
            assert_eq!(ron::to_string(&tile.view.viewport).unwrap(), navigation);
            state.update(Message::Redo(2)).unwrap();
            assert!(state.user.workspace.tiles.is_empty());
            assert!(state.user.workspace.item_lists.is_empty());
        }
        let fresh = open(&mut state, "waveform");
        assert!(fresh.0 > second.0);
        assert!(
            state.user.workspace.tiles[&fresh]
                .kind
                .waveform_list()
                .unwrap()
                .0
                > list.0
        );
    }

    #[test]
    fn independent_split_undo_owns_only_the_new_list_and_resets_restored_runtime() {
        use crate::tiles::commands::SplitMode;
        let mut state = SystemState::new_default_config().unwrap();
        let first = open(&mut state, "waveform");
        let original = state.user.workspace.tiles[&first]
            .kind
            .waveform_list()
            .unwrap();
        state
            .update(Message::Workspace(WorkspaceCommand::SplitTile {
                tile: first,
                dir: Direction::Right,
                mode: SplitMode::Independent,
            }))
            .unwrap();
        let second = state.user.workspace.layout.focused().unwrap();
        let copied = state.user.workspace.tiles[&second]
            .kind
            .waveform_list()
            .unwrap();
        assert_ne!(original, copied);
        state
            .user
            .workspace
            .item_lists
            .get_mut(&copied)
            .unwrap()
            .annotation_counter = 23;
        let TileKind::Waveform(tile) =
            &mut state.user.workspace.tiles.get_mut(&second).unwrap().kind
        else {
            panic!()
        };
        tile.view.viewport.handle_canvas_scroll(0.4);
        tile.view.selected_annotation = Some(egui::Id::new("temporary"));
        let navigation = ron::to_string(&tile.view.viewport).unwrap();
        state.update(Message::Undo(1)).unwrap();
        assert!(!state.user.workspace.item_lists.contains_key(&copied));
        state
            .user
            .workspace
            .item_lists
            .get_mut(&original)
            .unwrap()
            .annotation_counter = 42;
        state.update(Message::Redo(1)).unwrap();
        assert_eq!(
            state.user.workspace.item_lists[&original].annotation_counter,
            42
        );
        assert_eq!(
            state.user.workspace.item_lists[&copied].annotation_counter,
            23
        );
        let TileKind::Waveform(tile) = &state.user.workspace.tiles[&second].kind else {
            panic!()
        };
        assert_eq!(ron::to_string(&tile.view.viewport).unwrap(), navigation);
        assert!(tile.view.selected_annotation.is_none());
        assert_eq!(state.user.workspace.layout.focused(), Some(first));
    }

    #[test]
    fn layout_replacement_undo_preserves_surviving_resizes_focus_and_views() {
        use super::LayoutNode;
        use crate::tiles::layout::SplitDir;
        let mut state = SystemState::new_default_config().unwrap();
        let ids = (0..4)
            .map(|_| open(&mut state, "waveform"))
            .collect::<Vec<_>>();
        state.undo_stack.clear();
        let pair = |a, b| LayoutNode::Split {
            dir: SplitDir::Horizontal,
            shares: vec![0.5, 0.5],
            children: vec![LayoutNode::Tile(ids[a]), LayoutNode::Tile(ids[b])],
        };
        let original = LayoutNode::Split {
            dir: SplitDir::Vertical,
            shares: vec![0.4, 0.6],
            children: vec![pair(0, 1), pair(2, 3)],
        };
        state
            .user
            .workspace
            .apply_command(
                &mut state.workspace_runtime,
                WorkspaceCommand::SetLayout(Some(original)),
            )
            .unwrap();
        let mut replacement = state.user.workspace.layout.root().cloned().unwrap();
        let LayoutNode::Split { dir, .. } = &mut replacement else {
            panic!()
        };
        *dir = SplitDir::Horizontal;
        state
            .update(Message::Workspace(WorkspaceCommand::SetLayout(Some(
                replacement,
            ))))
            .unwrap();
        assert_eq!(state.undo_stack.len(), 1);
        let mut resized = state.user.workspace.layout.root().cloned().unwrap();
        let LayoutNode::Split { children, .. } = &mut resized else {
            panic!()
        };
        let LayoutNode::Split { shares, .. } = &mut children[1] else {
            panic!()
        };
        *shares = vec![0.8, 0.2];
        state
            .update(Message::Workspace(WorkspaceCommand::SetLayout(Some(
                resized,
            ))))
            .unwrap();
        assert_eq!(
            state.undo_stack.len(),
            1,
            "resizing alone creates no history"
        );
        state
            .update(Message::Workspace(WorkspaceCommand::FocusTile(ids[1])))
            .unwrap();
        let TileKind::Waveform(tile) =
            &mut state.user.workspace.tiles.get_mut(&ids[0]).unwrap().kind
        else {
            panic!()
        };
        tile.view.viewport.handle_canvas_scroll(0.2);
        let viewport = ron::to_string(&tile.view.viewport).unwrap();
        for (message, expected_dir) in [
            (Message::Undo(1), SplitDir::Vertical),
            (Message::Redo(1), SplitDir::Horizontal),
        ] {
            state.update(message).unwrap();
            let LayoutNode::Split { dir, children, .. } =
                state.user.workspace.layout.root().unwrap()
            else {
                panic!()
            };
            assert_eq!(*dir, expected_dir);
            let LayoutNode::Split { shares, .. } = &children[1] else {
                panic!()
            };
            assert_eq!(shares, &[0.8, 0.2]);
            assert_eq!(state.user.workspace.layout.focused(), Some(ids[1]));
            let TileKind::Waveform(tile) = &state.user.workspace.tiles[&ids[0]].kind else {
                panic!()
            };
            assert_eq!(ron::to_string(&tile.view.viewport).unwrap(), viewport);
        }
        state.update(Message::Undo(1)).unwrap();
        assert!(
            state
                .update(Message::Workspace(WorkspaceCommand::SetLayout(Some(
                    LayoutNode::Tile(ids[0])
                ))))
                .is_none()
        );
        assert_eq!(
            state.redo_stack.len(),
            1,
            "invalid replacement preserves redo"
        );
    }

    #[test]
    fn navigation_matching_tracks_children_across_reordering() {
        use super::{LayoutNode, TileId, retain_layout_navigation};
        use crate::tiles::layout::SplitDir;
        let tabs = |active| LayoutNode::Tabs {
            active,
            children: vec![LayoutNode::Tile(TileId(1)), LayoutNode::Tile(TileId(2))],
        };
        let current = LayoutNode::Split {
            dir: SplitDir::Horizontal,
            shares: vec![0.7, 0.3],
            children: vec![tabs(1), LayoutNode::Tile(TileId(3))],
        };
        let mut restored = LayoutNode::Split {
            dir: SplitDir::Horizontal,
            shares: vec![0.5, 0.5],
            children: vec![LayoutNode::Tile(TileId(3)), tabs(0)],
        };
        retain_layout_navigation(&mut restored, &current);
        let LayoutNode::Split {
            shares, children, ..
        } = restored
        else {
            panic!()
        };
        assert_eq!(shares, vec![0.3, 0.7]);
        assert!(matches!(&children[1], LayoutNode::Tabs { active: 1, .. }));
    }

    #[test]
    fn settings_history_interleaves_kinds_without_restoring_focus() {
        let mut state = SystemState::new_default_config().unwrap();
        let logs = open(&mut state, "logs");
        let annotations = open(&mut state, "annotation_list");
        state.undo_stack.clear();
        let filter =
            |filter| Message::ToTile(logs, TileMessage::Logs(LogsMessage::SetFilter(filter)));
        state.update(filter(LevelFilter::Warn)).unwrap();
        state
            .update(Message::ToTile(
                annotations,
                TileMessage::AnnotationList(AnnotationListMessage::ShowComments(true)),
            ))
            .unwrap();
        assert_eq!(state.undo_stack.len(), 2);
        state
            .update(Message::Workspace(WorkspaceCommand::FocusTile(logs)))
            .unwrap();
        let settings = |state: &SystemState| {
            let TileKind::Logs(log) = &state.user.workspace.tiles[&logs].kind else {
                panic!()
            };
            let TileKind::AnnotationList(annotation) =
                &state.user.workspace.tiles[&annotations].kind
            else {
                panic!()
            };
            (log.filter, annotation.show_comments)
        };
        state.update(Message::Undo(1)).unwrap();
        assert_eq!(settings(&state), (LevelFilter::Warn, false));
        state.update(filter(LevelFilter::Warn)).unwrap();
        assert_eq!(state.redo_stack.len(), 1, "no-op filter must preserve redo");
        state.update(Message::Undo(1)).unwrap();
        assert_eq!(settings(&state), (LevelFilter::Trace, false));
        state.update(Message::Redo(2)).unwrap();
        assert_eq!(settings(&state), (LevelFilter::Warn, true));
        assert_eq!(state.user.workspace.layout.focused(), Some(logs));
        state.update(Message::Undo(1)).unwrap();
        state.update(filter(LevelFilter::Error)).unwrap();
        assert!(state.redo_stack.is_empty(), "semantic edit clears redo");
    }

    #[test]
    fn renaming_and_column_visibility_undo_preserve_later_column_resizing() {
        use crate::tile_kinds::waveform::WaveformMessage;
        let mut state = SystemState::new_default_config().unwrap();
        let waveform = open(&mut state, "waveform");
        let logs = open(&mut state, "logs");
        state.undo_stack.clear();
        let initial_columns = match &state.user.workspace.tiles[&waveform].kind {
            TileKind::Waveform(tile) => (tile.show_name_column, tile.show_value_column),
            _ => panic!(),
        };
        state
            .update(Message::Workspace(WorkspaceCommand::RenameTile {
                tile: waveform,
                title: Some("Bus".into()),
            }))
            .unwrap();
        state
            .update(Message::ToTile(
                waveform,
                TileMessage::Waveform(WaveformMessage::Columns {
                    names: !initial_columns.0,
                    values: !initial_columns.1,
                }),
            ))
            .unwrap();
        state
            .update(Message::ToTile(
                waveform,
                TileMessage::Waveform(WaveformMessage::ColumnWidths {
                    names: 123.0,
                    values: 87.0,
                }),
            ))
            .unwrap();
        state
            .update(Message::Workspace(WorkspaceCommand::FocusTile(logs)))
            .unwrap();
        assert_eq!(state.undo_stack.len(), 2);
        state.update(Message::Undo(2)).unwrap();
        assert_eq!(state.user.workspace.tiles[&waveform].title, None);
        let TileKind::Waveform(tile) = &state.user.workspace.tiles[&waveform].kind else {
            panic!()
        };
        assert_eq!(
            (tile.show_name_column, tile.show_value_column),
            initial_columns
        );
        assert_eq!(
            (tile.name_column_width, tile.value_column_width),
            (123.0, 87.0)
        );
        state.update(Message::Redo(2)).unwrap();
        assert_eq!(
            state.user.workspace.tiles[&waveform].title.as_deref(),
            Some("Bus")
        );
        let TileKind::Waveform(tile) = &state.user.workspace.tiles[&waveform].kind else {
            panic!()
        };
        assert_eq!(
            (tile.show_name_column, tile.show_value_column),
            (!initial_columns.0, !initial_columns.1)
        );
        assert_eq!(
            (tile.name_column_width, tile.value_column_width),
            (123.0, 87.0)
        );
        assert_eq!(state.user.workspace.layout.focused(), Some(logs));
    }

    #[test]
    fn setting_history_never_redirects_to_a_reopened_singleton() {
        let mut state = SystemState::new_default_config().unwrap();
        let first = open(&mut state, "logs");
        state
            .update(Message::ToTile(
                first,
                TileMessage::Logs(LogsMessage::SetFilter(LevelFilter::Warn)),
            ))
            .unwrap();
        let record = state.undo_stack.pop().unwrap();
        state
            .update(Message::Workspace(WorkspaceCommand::CloseTile(first)))
            .unwrap();
        let second = open(&mut state, "logs");
        assert_ne!(first, second);
        assert!(state.restore_history(record, false).is_err());
        let TileKind::Logs(tile) = &state.user.workspace.tiles[&second].kind else {
            panic!()
        };
        assert_eq!(tile.filter, LevelFilter::Trace);
    }
}
