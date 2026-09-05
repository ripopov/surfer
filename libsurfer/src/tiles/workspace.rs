//! Workspace resource ownership and atomic replacement after decoding.

use std::collections::{BTreeMap, BTreeSet};

use ::serde::{Deserialize, Serialize};

use crate::item_list::ItemList;

use super::{
    ItemListId, TileId, TileTarget,
    commands::{SplitMode, WorkspaceCommand},
    kind::{KINDS, KindCreateError, KindDecodeError, TileEntry, TileKind, WAVEFORM},
    layout::{Layout, LayoutError, LayoutFile, Placement},
    runtime::{IdentityError, WorkspaceRuntime},
    serde::{DecodeError, ItemListError, ItemListFile, TileFile, decode},
};

pub const WORKSPACE_VERSION: u32 = 1;

/// Marks views sharing an item list in default titles (`Waveform 2 🔗1`).
/// The icon font is a fallback of every text family, so tabs can render it.
pub const LINKED_LIST_GLYPH: &str = egui_remixicon::icons::LINK;

#[derive(Serialize, Deserialize)]
pub struct WorkspaceFile {
    pub version: u32,
    pub layout: LayoutFile,
    pub tiles: BTreeMap<TileId, TileFile>,
    pub item_lists: BTreeMap<ItemListId, ItemListFile>,
}

#[cfg(test)]
mod tests {
    use super::super::layout::LayoutNode;
    use super::*;

    fn unknown_workspace() -> WorkspaceFile {
        WorkspaceFile {
            version: WORKSPACE_VERSION,
            layout: LayoutFile {
                root: Some(LayoutNode::Tile(TileId(4))),
                focused: Some(TileId(4)),
                focus_history: vec![TileId(4)],
            },
            tiles: BTreeMap::from([(
                TileId(4),
                decode(include_str!("fixtures/future-tile.ron")).unwrap(),
            )]),
            item_lists: BTreeMap::from([(ItemListId(8), ItemListFile::from(&ItemList::default()))]),
        }
    }

    #[test]
    fn replacement_preserves_unknown_resources_and_invalidates_old_requests() {
        let mut workspace = Workspace::default();
        let mut runtime = WorkspaceRuntime::default();
        let pending = runtime.request(TileId(4)).unwrap();
        let input = ron::to_string(&unknown_workspace()).unwrap();
        workspace.replace(&mut runtime, &input).unwrap();
        assert!(!runtime.accepts(pending, Some(pending)));
        assert!(workspace.item_lists.contains_key(&ItemListId(8)));
        assert_eq!(runtime.allocate_tile().unwrap(), TileId(5));
        assert_eq!(runtime.allocate_list().unwrap(), ItemListId(9));
        let saved = workspace.to_file().unwrap();
        assert_eq!(
            saved.tiles[&TileId(4)].payload.get_ron(),
            unknown_workspace().tiles[&TileId(4)].payload.get_ron()
        );
        Workspace::from_file(saved).unwrap();
    }

    #[test]
    fn failed_replacement_preserves_workspace_and_runtime() {
        let mut workspace = Workspace::from_file(unknown_workspace()).unwrap();
        let mut runtime = WorkspaceRuntime::default();
        let pending = runtime.request(TileId(4)).unwrap();
        let original = ron::to_string(&workspace.to_file().unwrap()).unwrap();
        let mut invalid = unknown_workspace();
        invalid.layout.root = None;
        assert!(
            workspace
                .replace(&mut runtime, &ron::to_string(&invalid).unwrap())
                .is_err()
        );
        assert!(workspace.replace(&mut runtime, "invalid RON").is_err());
        assert_eq!(
            ron::to_string(&workspace.to_file().unwrap()).unwrap(),
            original
        );
        assert!(runtime.accepts(pending, Some(pending)));
        assert_eq!(runtime.allocate_tile().unwrap(), TileId(1));
    }

    #[test]
    fn rejects_orphan_resources_without_unknown_tiles() {
        let mut file = unknown_workspace();
        file.tiles.clear();
        file.layout = LayoutFile::default();
        assert!(matches!(
            Workspace::from_file(file),
            Err(WorkspaceError::OrphanList(ItemListId(8)))
        ));
    }

    fn create(
        workspace: &mut Workspace,
        runtime: &mut WorkspaceRuntime,
        placement: Placement,
    ) -> TileId {
        workspace
            .apply_command(
                runtime,
                WorkspaceCommand::CreateTile {
                    kind: "waveform".into(),
                    placement,
                    focus: true,
                },
            )
            .unwrap();
        workspace.layout.focused().unwrap()
    }

    fn validate(workspace: &Workspace) {
        Workspace::from_file(workspace.to_file().unwrap()).unwrap();
    }

    #[test]
    fn linked_and_independent_splits_collect_only_unowned_lists() {
        use super::super::layout::Direction;
        let mut workspace = Workspace::default();
        let mut runtime = WorkspaceRuntime::default();
        let first = create(&mut workspace, &mut runtime, Placement::Root);
        let original = workspace.tiles[&first].kind.item_list().unwrap();
        workspace
            .item_lists
            .get_mut(&original)
            .unwrap()
            .layout_cache
            .borrow_mut()
            .signature = Some(99);
        for mode in [SplitMode::Linked, SplitMode::Independent] {
            workspace
                .apply_command(
                    &mut runtime,
                    WorkspaceCommand::SplitTile {
                        tile: first,
                        dir: Direction::Right,
                        mode,
                    },
                )
                .unwrap();
            validate(&workspace);
        }
        let independent = workspace.layout.focused().unwrap();
        let copied = workspace.tiles[&independent].kind.item_list().unwrap();
        assert_ne!(original, copied);
        assert!(
            workspace.item_lists[&copied]
                .layout_cache
                .borrow()
                .signature
                .is_none()
        );
        assert_eq!(workspace.item_lists.len(), 2);
        workspace
            .apply_command(&mut runtime, WorkspaceCommand::CloseTile(first))
            .unwrap();
        assert!(workspace.item_lists.contains_key(&original));
        workspace
            .apply_command(&mut runtime, WorkspaceCommand::CloseTile(independent))
            .unwrap();
        assert!(!workspace.item_lists.contains_key(&copied));
        let last = workspace.layout.tile_order()[0];
        workspace
            .apply_command(&mut runtime, WorkspaceCommand::CloseTile(last))
            .unwrap();
        assert!(workspace.tiles.is_empty());
        assert!(workspace.item_lists.is_empty());
        assert!(workspace.layout.root().is_none());
        validate(&workspace);
    }

    #[test]
    fn close_others_only_removes_sibling_tabs_and_stale_commands_do_not_redirect() {
        use super::super::layout::Direction;
        let mut workspace = Workspace::default();
        let mut runtime = WorkspaceRuntime::default();
        let first = create(&mut workspace, &mut runtime, Placement::Root);
        let sibling = create(&mut workspace, &mut runtime, Placement::TabAfter(first));
        let separate = create(
            &mut workspace,
            &mut runtime,
            Placement::Beside(first, Direction::Down),
        );
        workspace
            .apply_command(&mut runtime, WorkspaceCommand::CloseOtherTiles(first))
            .unwrap();
        assert_eq!(
            workspace.tiles.keys().copied().collect::<Vec<_>>(),
            vec![first, separate]
        );
        assert_eq!(workspace.layout.focused(), Some(separate));
        let before = ron::to_string(&workspace.to_file().unwrap()).unwrap();
        for command in [
            WorkspaceCommand::FocusTile(sibling),
            WorkspaceCommand::CloseTile(sibling),
            WorkspaceCommand::RenameTile {
                tile: sibling,
                title: Some("stale".into()),
            },
            WorkspaceCommand::CreateTile {
                kind: "waveform".into(),
                placement: Placement::Root,
                focus: true,
            },
            WorkspaceCommand::CreateTile {
                kind: "missing".into(),
                placement: Placement::TabAfter(first),
                focus: true,
            },
            WorkspaceCommand::SplitTile {
                tile: first,
                dir: Direction::Right,
                mode: SplitMode::Clone,
            },
            WorkspaceCommand::SetLayout(None),
        ] {
            assert!(workspace.apply_command(&mut runtime, command).is_err());
            assert_eq!(
                ron::to_string(&workspace.to_file().unwrap()).unwrap(),
                before
            );
        }
        validate(&workspace);
    }

    #[test]
    fn closing_last_unknown_resumes_resource_collection() {
        let mut workspace = Workspace::default();
        let mut runtime = WorkspaceRuntime::default();
        workspace
            .replace(&mut runtime, &ron::to_string(&unknown_workspace()).unwrap())
            .unwrap();
        let waveform = create(&mut workspace, &mut runtime, Placement::TabAfter(TileId(4)));
        workspace
            .apply_command(&mut runtime, WorkspaceCommand::CloseTile(waveform))
            .unwrap();
        // Even the known tile's former list is retained until all unknown owners are gone.
        assert_eq!(workspace.item_lists.len(), 2);
        workspace
            .apply_command(&mut runtime, WorkspaceCommand::CloseTile(TileId(4)))
            .unwrap();
        assert!(workspace.item_lists.is_empty());
        validate(&workspace);
    }

    #[test]
    fn waveform_resolution_is_explicit_and_does_not_reveal_hidden_tabs() {
        let mut workspace = Workspace::default();
        let mut runtime = WorkspaceRuntime::default();
        workspace
            .replace(&mut runtime, &ron::to_string(&unknown_workspace()).unwrap())
            .unwrap();
        let first = create(&mut workspace, &mut runtime, Placement::TabAfter(TileId(4)));
        let second = create(&mut workspace, &mut runtime, Placement::TabAfter(first));
        workspace
            .apply_command(&mut runtime, WorkspaceCommand::FocusTile(first))
            .unwrap();
        workspace
            .apply_command(&mut runtime, WorkspaceCommand::FocusTile(TileId(4)))
            .unwrap();
        assert_eq!(workspace.resolve_tile(TileTarget::Focused), Some(TileId(4)));
        assert_eq!(workspace.resolve_waveform(TileTarget::Focused), Some(first));
        assert_eq!(
            workspace.resolve_waveform(TileTarget::Id(second)),
            Some(second)
        );
        assert_eq!(workspace.resolve_waveform(TileTarget::Id(TileId(4))), None);
        assert_eq!(
            workspace.resolve_waveform(TileTarget::Id(TileId(999))),
            None
        );
        assert_eq!(workspace.layout.visible_tiles(), vec![TileId(4)]);
        workspace
            .apply_command(&mut runtime, WorkspaceCommand::CloseTile(first))
            .unwrap();
        assert_eq!(
            workspace.resolve_waveform(TileTarget::Focused),
            Some(second)
        );
        assert_eq!(workspace.resolve_waveform(TileTarget::Id(first)), None);
    }

    #[test]
    fn adapter_edits_reject_stale_revisions_and_invalid_tile_sets() {
        use super::super::render::LayoutEdit;
        let mut workspace = Workspace::default();
        let mut runtime = WorkspaceRuntime::default();
        let first = create(&mut workspace, &mut runtime, Placement::Root);
        let revision = workspace.layout.revision();
        let root = workspace.layout.to_file().root;
        create(&mut workspace, &mut runtime, Placement::TabAfter(first));
        let before = workspace.layout.to_file();
        assert!(
            workspace
                .apply_layout_edit(LayoutEdit {
                    revision,
                    root,
                    focused: Some(first),
                    structural: true,
                    moved_tile: Some(first),
                })
                .is_err()
        );
        assert!(
            workspace
                .apply_layout_edit(LayoutEdit {
                    revision: workspace.layout.revision(),
                    root: None,
                    focused: None,
                    structural: true,
                    moved_tile: Some(first),
                })
                .is_err()
        );
        assert_eq!(workspace.layout.to_file(), before);
    }

    #[test]
    fn reset_keeps_only_the_target_waveform_or_creates_an_empty_one() {
        use super::super::layout::Direction;
        let mut workspace = Workspace::default();
        let mut runtime = WorkspaceRuntime::default();
        assert!(
            workspace
                .apply_command(&mut runtime, WorkspaceCommand::Reset { keep: None })
                .unwrap()
        );
        let created = workspace.layout.focused().unwrap();
        assert_eq!(workspace.tiles.len(), 1);
        assert_eq!(workspace.item_lists.len(), 1);
        assert!(
            !workspace
                .apply_command(
                    &mut runtime,
                    WorkspaceCommand::Reset {
                        keep: Some(created)
                    }
                )
                .unwrap()
        );
        let second = create(
            &mut workspace,
            &mut runtime,
            Placement::Beside(created, Direction::Down),
        );
        workspace
            .apply_command(
                &mut runtime,
                WorkspaceCommand::CreateTile {
                    kind: "logs".into(),
                    placement: Placement::TabAfter(second),
                    focus: true,
                },
            )
            .unwrap();
        assert_eq!(workspace.item_lists.len(), 2);
        let kept_list = workspace.tiles[&second].kind.item_list().unwrap();
        assert!(
            workspace
                .apply_command(&mut runtime, WorkspaceCommand::Reset { keep: Some(second) })
                .unwrap()
        );
        assert_eq!(
            workspace.tiles.keys().copied().collect::<Vec<_>>(),
            [second]
        );
        assert_eq!(
            workspace.item_lists.keys().copied().collect::<Vec<_>>(),
            [kept_list]
        );
        assert_eq!(workspace.layout.focused(), Some(second));
        assert!(matches!(
            workspace.layout.root(),
            Some(LayoutNode::Tabs { children, .. }) if children == &[LayoutNode::Tile(second)]
        ));
        let before = ron::to_string(&workspace.to_file().unwrap()).unwrap();
        assert!(
            workspace
                .apply_command(
                    &mut runtime,
                    WorkspaceCommand::Reset {
                        keep: Some(created)
                    }
                )
                .is_err()
        );
        assert_eq!(
            ron::to_string(&workspace.to_file().unwrap()).unwrap(),
            before
        );
        validate(&workspace);
    }

    #[test]
    fn titles_number_waveforms_mark_shared_lists_and_honor_overrides() {
        use super::super::layout::Direction;
        let mut workspace = Workspace::default();
        let mut runtime = WorkspaceRuntime::default();
        let first = create(&mut workspace, &mut runtime, Placement::Root);
        assert_eq!(workspace.titles()[&first], "Waveform");
        workspace
            .apply_command(
                &mut runtime,
                WorkspaceCommand::CreateTile {
                    kind: "memory".into(),
                    placement: Placement::TabAfter(first),
                    focus: true,
                },
            )
            .unwrap();
        let memory = workspace.layout.focused().unwrap();
        assert_eq!(workspace.titles()[&memory], "Memory");
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
        let linked = workspace.layout.focused().unwrap();
        workspace
            .apply_command(
                &mut runtime,
                WorkspaceCommand::SplitTile {
                    tile: linked,
                    dir: Direction::Down,
                    mode: SplitMode::Independent,
                },
            )
            .unwrap();
        let independent = workspace.layout.focused().unwrap();
        let titles = workspace.titles();
        assert_eq!(titles[&first], format!("Waveform 1 {LINKED_LIST_GLYPH}1"));
        assert_eq!(titles[&linked], format!("Waveform 2 {LINKED_LIST_GLYPH}1"));
        assert_eq!(titles[&independent], "Waveform 3");
        workspace
            .apply_command(
                &mut runtime,
                WorkspaceCommand::RenameTile {
                    tile: independent,
                    title: Some("Detail".into()),
                },
            )
            .unwrap();
        fn memory_tile(
            workspace: &mut Workspace,
            id: TileId,
        ) -> &mut crate::tile_kinds::memory::MemoryTile {
            match &mut workspace.tiles.get_mut(&id).unwrap().kind {
                TileKind::Memory(tile) => tile,
                _ => panic!(),
            }
        }
        memory_tile(&mut workspace, memory).settings.scope =
            Some(crate::wave_container::ScopeRef {
                strs: vec!["dut".into(), "mem".into()],
                id: Default::default(),
            });
        assert_eq!(workspace.titles()[&memory], "Memory: dut.mem");
        memory_tile(&mut workspace, memory).settings.name = Some("RAM".into());
        let titles = workspace.titles();
        assert_eq!(titles[&memory], "Memory: RAM");
        assert_eq!(titles[&independent], "Detail");
        assert_eq!(
            workspace.tile_suggestions(),
            [
                "#1".to_string(),
                format!("Waveform 1 {LINKED_LIST_GLYPH}1"),
                "#2".into(),
                "Memory: RAM".into(),
                "#3".into(),
                format!("Waveform 2 {LINKED_LIST_GLYPH}1"),
                "#4".into(),
                "Detail".into()
            ]
        );
    }

    #[test]
    fn loaded_focus_must_reference_the_tiles_own_list() {
        let mut workspace = Workspace::default();
        let mut runtime = WorkspaceRuntime::default();
        let id = create(&mut workspace, &mut runtime, Placement::Root);
        let TileKind::Waveform(tile) = &mut workspace.tiles.get_mut(&id).unwrap().kind else {
            panic!("expected waveform");
        };
        tile.view.focused_item = Some(crate::displayed_item::DisplayedItemRef(99));
        assert!(
            matches!(Workspace::from_file(workspace.to_file().unwrap()), Err(WorkspaceError::InvalidItemReference(target)) if target == id)
        );
    }
}

#[derive(Default)]
pub struct Workspace {
    pub layout: Layout,
    pub tiles: BTreeMap<TileId, TileEntry>,
    pub item_lists: BTreeMap<ItemListId, ItemList>,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("unsupported workspace version {0}")]
    Version(u32),
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error(transparent)]
    Kind(#[from] KindDecodeError),
    #[error(transparent)]
    Layout(#[from] LayoutError),
    #[error(transparent)]
    Identity(#[from] IdentityError),
    #[error("tile references missing item list {0:?}")]
    MissingList(ItemListId),
    #[error("unreferenced item list {0:?}")]
    OrphanList(ItemListId),
    #[error("invalid item list {0:?}: {1}")]
    ItemList(ItemListId, ItemListError),
    #[error(transparent)]
    Create(#[from] KindCreateError),
    #[error("tile {0:?} cannot use the requested split mode")]
    CannotSplit(TileId),
    #[error("singleton kind {0} appears more than once")]
    Singleton(String),
    #[error("tile {0:?} contains an invalid item reference")]
    InvalidItemReference(TileId),
}

impl Workspace {
    pub fn resolve_tile(&self, target: TileTarget) -> Option<TileId> {
        let id = match target {
            TileTarget::Id(id) => id,
            TileTarget::Focused => self.layout.focused()?,
        };
        self.tiles.contains_key(&id).then_some(id)
    }

    /// Only callers explicitly declaring waveform fallback use this resolver.
    /// Resolution itself does not reveal hidden tabs or change focus.
    pub fn resolve_waveform(&self, target: TileTarget) -> Option<TileId> {
        let is_waveform = |id: &TileId| {
            self.tiles
                .get(id)
                .is_some_and(|tile| tile.kind.is_waveform())
        };
        match target {
            TileTarget::Id(id) => is_waveform(&id).then_some(id),
            TileTarget::Focused => self
                .layout
                .focused()
                .into_iter()
                .chain(self.layout.focus_history().iter().copied())
                .chain(self.layout.tile_order())
                .find(is_waveform),
        }
    }

    pub fn apply_layout_edit(
        &mut self,
        edit: super::render::LayoutEdit,
    ) -> Result<bool, WorkspaceError> {
        let changed = self
            .layout
            .apply_proposal(edit.revision, edit.root, edit.focused)?;
        if changed {
            self.reconcile_waveform_scroll();
        }
        Ok(changed)
    }

    /// Apply a resolved operation. All fallible topology/resource preparation precedes mutation.
    pub fn apply_command(
        &mut self,
        runtime: &mut WorkspaceRuntime,
        command: WorkspaceCommand,
    ) -> Result<bool, WorkspaceError> {
        let previous_revision = self.layout.revision();
        let changed = self.apply_command_inner(runtime, command)?;
        runtime.mark_workspace_initialized();
        if self.layout.revision() != previous_revision {
            self.reconcile_waveform_scroll();
        }
        Ok(changed)
    }

    fn apply_command_inner(
        &mut self,
        runtime: &mut WorkspaceRuntime,
        command: WorkspaceCommand,
    ) -> Result<bool, WorkspaceError> {
        match command {
            WorkspaceCommand::CreateTile {
                kind,
                placement,
                focus,
            }
            | WorkspaceCommand::OpenTile {
                kind,
                placement,
                focus,
            } => {
                if KINDS
                    .iter()
                    .any(|entry| entry.name == kind && entry.singleton)
                    && let Some((&id, _)) = self
                        .tiles
                        .iter()
                        .find(|(_, tile)| tile.kind.kind_name() == kind)
                {
                    return if focus {
                        Ok(self.layout.focus(id)?)
                    } else {
                        Ok(false)
                    };
                }
                let id = runtime.allocate_tile()?;
                let mut layout = self.layout.clone();
                layout.insert(id, placement)?;
                if focus {
                    layout.focus(id)?;
                }
                let (kind, list) = TileKind::create(&kind, runtime)?;
                self.tiles.insert(id, TileEntry { title: None, kind });
                if let Some((id, list)) = list {
                    self.item_lists.insert(id, list);
                }
                self.layout = layout;
                Ok(true)
            }
            WorkspaceCommand::SplitTile { tile, dir, mode } => {
                let source = self.tiles.get(&tile).ok_or(LayoutError::Missing(tile))?;
                if !source.kind.supports_split(mode)
                    || source.kind.descriptor().is_some_and(|kind| kind.singleton)
                {
                    return Err(WorkspaceError::CannotSplit(tile));
                }
                let mut kind = source
                    .kind
                    .split_clone()
                    .ok_or(WorkspaceError::CannotSplit(tile))?;
                let title = source.title.clone();
                let list = if mode == SplitMode::Independent {
                    let original = kind.item_list().ok_or(WorkspaceError::CannotSplit(tile))?;
                    let content = self
                        .item_lists
                        .get(&original)
                        .ok_or(WorkspaceError::MissingList(original))?
                        .copy_content();
                    let id = runtime.allocate_list()?;
                    kind.replace_item_list(id);
                    Some((id, content))
                } else {
                    None
                };
                let id = runtime.allocate_tile()?;
                let mut layout = self.layout.clone();
                layout.insert(id, Placement::Beside(tile, dir))?;
                layout.focus(id)?;
                self.tiles.insert(id, TileEntry { title, kind });
                if let Some((id, list)) = list {
                    self.item_lists.insert(id, list);
                }
                self.layout = layout;
                Ok(true)
            }
            WorkspaceCommand::CloseTile(tile) => {
                self.layout.remove(tile)?;
                self.tiles.remove(&tile);
                self.collect_lists();
                Ok(true)
            }
            WorkspaceCommand::CloseOtherTiles(tile) => {
                if !self.tiles.contains_key(&tile) {
                    return Err(LayoutError::Missing(tile).into());
                }
                let mut siblings = Vec::new();
                let mut next = tile;
                while let Some(id) = self.layout.next_in_group(next, 1) {
                    if id == tile {
                        break;
                    }
                    siblings.push(id);
                    next = id;
                }
                if siblings.is_empty() {
                    return Ok(false);
                }
                let mut layout = self.layout.clone();
                for id in &siblings {
                    layout.remove(*id)?;
                }
                self.layout = layout;
                for id in siblings {
                    self.tiles.remove(&id);
                }
                self.collect_lists();
                Ok(true)
            }
            WorkspaceCommand::FocusTile(tile) => Ok(self.layout.focus(tile)?),
            WorkspaceCommand::MoveTile { tile, to } => Ok(self.layout.move_tile(tile, to)?),
            WorkspaceCommand::RenameTile { tile, title } => {
                let entry = self
                    .tiles
                    .get_mut(&tile)
                    .ok_or(LayoutError::Missing(tile))?;
                if entry.title == title {
                    return Ok(false);
                }
                entry.title = title;
                Ok(true)
            }
            WorkspaceCommand::SetLayout(root) => {
                let mut file = self.layout.to_file();
                file.root = root;
                Ok(self
                    .layout
                    .apply_proposal(self.layout.revision(), file.root, file.focused)?)
            }
            WorkspaceCommand::Reset { keep } => {
                if let Some(id) = keep
                    && !self.tiles.contains_key(&id)
                {
                    return Err(LayoutError::Missing(id).into());
                }
                let removed = self.tiles.len() - usize::from(keep.is_some());
                let mut layout = self.layout.clone();
                let root = keep.map(super::layout::LayoutNode::Tile);
                let mut changed = layout.restore_topology(root, &keep.into_iter().collect())?;
                let created = match keep {
                    Some(id) => {
                        changed |= layout.focus(id)?;
                        None
                    }
                    None => {
                        let id = runtime.allocate_tile()?;
                        layout.insert(id, Placement::Root)?;
                        layout.focus(id)?;
                        changed = true;
                        Some((id, TileKind::create(WAVEFORM.name, runtime)?))
                    }
                };
                if removed == 0 && !changed {
                    return Ok(false);
                }
                self.tiles.retain(|id, _| Some(*id) == keep);
                if let Some((id, (kind, list))) = created {
                    self.tiles.insert(id, TileEntry { title: None, kind });
                    if let Some((list_id, list)) = list {
                        self.item_lists.insert(list_id, list);
                    }
                }
                self.layout = layout;
                self.collect_lists();
                Ok(true)
            }
        }
    }

    /// Titles shown on tabs and in menus, computed once per frame. Waveforms
    /// are numbered by layout order when there are several; views sharing a
    /// list show the list number after a link glyph. Entry titles override.
    pub fn titles(&self) -> BTreeMap<TileId, String> {
        let order = self.layout.tile_order();
        let waveforms = order
            .iter()
            .filter(|id| self.tiles[id].kind.is_waveform())
            .copied()
            .collect::<Vec<_>>();
        let mut list_numbers = BTreeMap::new();
        let mut list_users = BTreeMap::<ItemListId, usize>::new();
        for id in &waveforms {
            if let Some(list) = self.tiles[id].kind.item_list() {
                let next = list_numbers.len() + 1;
                list_numbers.entry(list).or_insert(next);
                *list_users.entry(list).or_default() += 1;
            }
        }
        order
            .into_iter()
            .map(|id| {
                let entry = &self.tiles[&id];
                let title = entry.title.clone().unwrap_or_else(|| match &entry.kind {
                    TileKind::Waveform(tile) => {
                        let mut title = if waveforms.len() == 1 {
                            "Waveform".to_string()
                        } else {
                            let index = waveforms.iter().position(|w| *w == id).unwrap() + 1;
                            format!("Waveform {index}")
                        };
                        if list_users.get(&tile.items).copied().unwrap_or(0) > 1 {
                            title.push_str(&format!(
                                " {LINKED_LIST_GLYPH}{}",
                                list_numbers[&tile.items]
                            ));
                        }
                        title
                    }
                    kind => kind.default_title(),
                });
                (id, title)
            })
            .collect()
    }

    fn collect_lists(&mut self) {
        if self
            .tiles
            .values()
            .any(|tile| tile.kind.has_unknown_resources())
        {
            return;
        }
        let referenced: BTreeSet<_> = self
            .tiles
            .values()
            .filter_map(|tile| tile.kind.item_list())
            .collect();
        self.item_lists.retain(|id, _| referenced.contains(id));
    }

    pub fn from_file(file: WorkspaceFile) -> Result<Self, WorkspaceError> {
        if file.version != WORKSPACE_VERSION {
            return Err(WorkspaceError::Version(file.version));
        }
        // Validate identity exhaustion without changing the live allocator.
        WorkspaceRuntime::default()
            .install_workspace(file.tiles.keys().copied(), file.item_lists.keys().copied())?;
        for (id, list) in &file.item_lists {
            list.validate()
                .map_err(|error| WorkspaceError::ItemList(*id, error))?;
        }
        let tile_ids = file.tiles.keys().copied().collect();
        let layout = Layout::from_file(file.layout, &tile_ids)?;
        let tiles = file
            .tiles
            .into_iter()
            .map(|(id, entry)| Ok((id, TileEntry::from_file(entry)?)))
            .collect::<Result<BTreeMap<_, _>, WorkspaceError>>()?;
        let referenced: BTreeSet<_> = tiles
            .values()
            .filter_map(|tile| tile.kind.item_list())
            .collect();
        for kind in KINDS.iter().filter(|kind| kind.singleton) {
            if tiles
                .values()
                .filter(|tile| tile.kind.kind_name() == kind.name)
                .count()
                > 1
            {
                return Err(WorkspaceError::Singleton(kind.name.into()));
            }
        }
        for id in &referenced {
            if !file.item_lists.contains_key(id) {
                return Err(WorkspaceError::MissingList(*id));
            }
        }
        // An opaque payload may reference resources that this version cannot inspect.
        if !tiles.values().any(|tile| tile.kind.has_unknown_resources()) {
            for id in file.item_lists.keys() {
                if !referenced.contains(id) {
                    return Err(WorkspaceError::OrphanList(*id));
                }
            }
        }
        let workspace = Self {
            layout,
            tiles,
            item_lists: file
                .item_lists
                .into_iter()
                .map(|(id, list)| (id, list.into()))
                .collect(),
        };
        for (id, tile) in &workspace.tiles {
            if let Some(list) = tile.kind.item_list()
                && !tile
                    .kind
                    .valid_item_references(&workspace.item_lists[&list])
            {
                return Err(WorkspaceError::InvalidItemReference(*id));
            }
        }
        Ok(workspace)
    }

    pub fn to_file(&self) -> Result<WorkspaceFile, ron::Error> {
        Ok(WorkspaceFile {
            version: WORKSPACE_VERSION,
            layout: self.layout.to_file(),
            tiles: self
                .tiles
                .iter()
                .map(|(id, tile)| Ok((*id, tile.to_file()?)))
                .collect::<Result<_, ron::Error>>()?,
            item_lists: self
                .item_lists
                .iter()
                .map(|(id, list)| (*id, ItemListFile::from(list)))
                .collect(),
        })
    }

    /// Decode and validate everything before invalidating requests or replacing state.
    pub fn replace(
        &mut self,
        runtime: &mut WorkspaceRuntime,
        input: &str,
    ) -> Result<(), WorkspaceError> {
        let candidate = Self::from_file(decode(input)?)?;
        runtime.install_workspace(
            candidate.tiles.keys().copied(),
            candidate.item_lists.keys().copied(),
        )?;
        *self = candidate;
        Ok(())
    }
}

impl Serialize for Workspace {
    fn serialize<S: ::serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_file()
            .map_err(::serde::ser::Error::custom)?
            .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Workspace {
    fn deserialize<D: ::serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let file = WorkspaceFile::deserialize(deserializer)?;
        Self::from_file(file).map_err(::serde::de::Error::custom)
    }
}
