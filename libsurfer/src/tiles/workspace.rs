//! Workspace resource ownership and atomic replacement after decoding.

use std::collections::BTreeMap;

use ::serde::{Deserialize, Serialize};

use crate::item_list::ItemList;

use super::{
    ItemListId, TileId, TileTarget,
    commands::{SplitMode, WorkspaceCommand},
    kind::{KINDS, KindCreateError, KindDecodeError, TileEntry, TileKind, WAVEFORM},
    layout::{Layout, LayoutError, LayoutFile, Placement},
    resources::{self, Dependencies, ResourceError, ResourceId},
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
        let original = workspace.tiles[&first].kind.waveform_list().unwrap();
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
        let copied = workspace.tiles[&independent].kind.waveform_list().unwrap();
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
        let kept_list = workspace.tiles[&second].kind.waveform_list().unwrap();
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

/// Owns the aggregate; storage is accessible only to workspace implementation
/// modules, never renderers or other application modules.
///
/// ```compile_fail
/// let mut workspace = libsurfer::tiles::workspace::Workspace::default();
/// workspace.tiles().clear(); // Membership cannot be edited through a read view.
/// ```
/// ```compile_fail
/// let mut workspace = libsurfer::tiles::workspace::Workspace::default();
/// workspace.item_lists().clear(); // Shared resources cannot be removed directly.
/// ```
/// ```compile_fail
/// let mut workspace = libsurfer::tiles::workspace::Workspace::default();
/// workspace.layout().remove(libsurfer::tiles::TileId(1));
/// ```
#[derive(Default)]
pub struct Workspace {
    layout: Layout,
    tiles: BTreeMap<TileId, TileEntry>,
    item_lists: BTreeMap<ItemListId, ItemList>,
}

pub(crate) mod history;
#[cfg(test)]
mod kind_tests;
pub(crate) mod legacy;
mod operations;

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error(transparent)]
    Resource(#[from] ResourceError),
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
    #[error("item list {0:?} already exists")]
    DuplicateList(ItemListId),
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
    pub fn layout(&self) -> &Layout {
        &self.layout
    }
    pub fn tiles(&self) -> &BTreeMap<TileId, TileEntry> {
        &self.tiles
    }
    pub fn item_lists(&self) -> &BTreeMap<ItemListId, ItemList> {
        &self.item_lists
    }

    pub(crate) fn set_geometry(
        &mut self,
        revision: u64,
        rects: BTreeMap<TileId, egui::Rect>,
    ) -> Result<(), LayoutError> {
        self.layout.set_geometry(revision, rects)
    }

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
                let (kind, lists) = TileKind::create(&kind, runtime)?;
                self.insert_prepared(
                    runtime,
                    TileEntry { title: None, kind },
                    lists,
                    placement,
                    focus,
                )?;
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
                let lists = if mode == SplitMode::Independent {
                    let (copy, lists) =
                        resources::independent_copy(&kind, &self.item_lists, runtime)?;
                    kind = copy;
                    lists
                } else {
                    BTreeMap::new()
                };
                self.insert_prepared(
                    runtime,
                    TileEntry { title, kind },
                    lists,
                    Placement::Beside(tile, dir),
                    true,
                )?;
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
                    self.item_lists.extend(list);
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
            if let Some(list) = self.tiles[id].kind.waveform_list() {
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
        let dependencies =
            Dependencies::union(self.tiles.values().map(|tile| tile.kind.dependencies()));
        resources::retain(&mut self.item_lists, &dependencies);
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
        let dependencies = Dependencies::union(tiles.values().map(|tile| tile.kind.dependencies()));
        dependencies.validate(
            &file
                .item_lists
                .keys()
                .copied()
                .map(ResourceId::ItemList)
                .collect(),
        )?;
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
        for id in file.item_lists.keys() {
            if !dependencies.retains(ResourceId::ItemList(*id)) {
                return Err(WorkspaceError::OrphanList(*id));
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
            if !tile
                .kind
                .valid_resource_references(|list| workspace.item_lists.get(&list))
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

impl Workspace {
    pub(crate) fn set_default_name_type(
        &mut self,
        name_type: crate::variable_name_type::VariableNameType,
    ) {
        for list in self.item_lists.values_mut() {
            list.default_variable_name_type = name_type;
        }
    }

    pub(crate) fn ensure_annotation_groups(&mut self) {
        for list in self.item_lists.values_mut() {
            if list.annotation_groups.is_empty() {
                list.annotation_groups
                    .push(crate::annotation_list::AnnotationGroup {
                        name: crate::annotation_list::DEFAULT_GROUP_NAME.into(),
                        annotations: Vec::new(),
                    });
            }
        }
    }

    pub(crate) fn recompute_display_names(&mut self, document: &crate::wave_data::WaveData) {
        for items in self.item_lists.values_mut() {
            items
                .compute_variable_display_names(&document.inner, document.display_variable_indices);
        }
    }

    /// Install a fully prepared tile and its owned resources as one operation.
    /// All membership and reference checks precede changes to the live aggregate.
    pub(crate) fn insert_prepared(
        &mut self,
        runtime: &mut WorkspaceRuntime,
        entry: TileEntry,
        lists: BTreeMap<ItemListId, ItemList>,
        placement: Placement,
        focus: bool,
    ) -> Result<TileId, WorkspaceError> {
        let id = runtime.allocate_tile()?;
        if self.tiles.contains_key(&id) {
            return Err(LayoutError::Duplicate(id).into());
        }
        let mut layout = self.layout.clone();
        layout.insert(id, placement)?;
        if focus {
            layout.focus(id)?;
        }
        if let Some(descriptor) = entry.kind.descriptor()
            && descriptor.singleton
            && self
                .tiles
                .values()
                .any(|tile| tile.kind.kind_name() == descriptor.name)
        {
            return Err(WorkspaceError::Singleton(descriptor.name.into()));
        }
        for (list, items) in &lists {
            if self.item_lists.contains_key(list) {
                return Err(WorkspaceError::DuplicateList(*list));
            }
            WorkspaceRuntime::default().install_workspace([], [*list])?;
            ItemListFile::from(items)
                .validate()
                .map_err(|error| WorkspaceError::ItemList(*list, error))?;
        }
        let dependencies = entry.kind.dependencies();
        dependencies.validate(
            &self
                .item_lists
                .keys()
                .chain(lists.keys())
                .copied()
                .map(ResourceId::ItemList)
                .collect(),
        )?;
        if !entry.kind.valid_resource_references(|list| {
            lists.get(&list).or_else(|| self.item_lists.get(&list))
        }) {
            return Err(WorkspaceError::InvalidItemReference(id));
        }
        for list in lists.keys() {
            if !dependencies.retains(ResourceId::ItemList(*list)) {
                return Err(WorkspaceError::OrphanList(*list));
            }
        }
        self.tiles.insert(id, entry);
        self.item_lists.extend(lists);
        self.layout = layout;
        self.reconcile_waveform_scroll();
        runtime.mark_workspace_initialized();
        Ok(id)
    }
}

impl Workspace {
    pub(crate) fn restore_items(
        &mut self,
        previous: crate::CanvasState,
    ) -> Result<crate::CanvasState, Box<crate::CanvasState>> {
        let Some(items) = self.item_lists.get_mut(&previous.list) else {
            return Err(Box::new(previous));
        };
        let inverse = crate::SystemState::current_canvas_state(
            previous.list,
            items,
            previous.message.clone(),
        );
        let mut views = self
            .tiles
            .values_mut()
            .filter_map(|entry| match &mut entry.kind {
                crate::tiles::kind::TileKind::Waveform(tile) if tile.items == previous.list => {
                    Some(&mut tile.view)
                }
                _ => None,
            })
            .map(|view| {
                let focus = view.focus_snapshot(items);
                (view, focus)
            })
            .collect::<Vec<_>>();
        items.items_tree = previous.items_tree;
        items.displayed_items = previous.displayed_items;
        items.graphics = previous.graphics;
        items.default_variable_name_type = previous.default_variable_name_type;
        items.annotations = previous.annotations;
        items.annotation_groups = previous.annotation_group;
        items.annotation_counter = previous.annotation_counter;
        *items.layout_cache.get_mut() = Default::default();
        items.flattened_rows_cache.get_mut().clear();
        for (view, focus) in &mut views {
            view.reconcile_item_focus(items, *focus);
            view.reconcile_annotations(items);
            view.invalidate_draw_cache();
        }
        Ok(inverse)
    }
}

impl Workspace {
    pub(crate) fn remove_marker_rows(&mut self, id: u8) -> Vec<crate::CanvasState> {
        use crate::displayed_item::DisplayedItem;
        let affected = self
            .item_lists
            .iter()
            .filter_map(|(list, items)| {
                let rows = items
                    .displayed_items
                    .iter()
                    .filter_map(|(row, item)| {
                        matches!(item, DisplayedItem::Marker(marker) if marker.idx == id)
                            .then_some(*row)
                    })
                    .collect::<Vec<_>>();
                (!rows.is_empty()).then_some((*list, rows))
            })
            .collect::<Vec<_>>();
        let mut lists = Vec::new();
        for (list, rows) in affected {
            let items = self.item_lists.get_mut(&list).unwrap();
            lists.push(crate::SystemState::current_canvas_state(
                list,
                items,
                "Remove marker".into(),
            ));
            let mut views = self
                .tiles
                .values_mut()
                .filter_map(|entry| match &mut entry.kind {
                    crate::tiles::kind::TileKind::Waveform(tile) if tile.items == list => {
                        Some(&mut tile.view)
                    }
                    _ => None,
                })
                .map(|view| {
                    let focus = view.focus_snapshot(items);
                    (view, focus)
                })
                .collect::<Vec<_>>();
            items.remove_items(&rows);
            for (view, focus) in &mut views {
                view.reconcile_item_focus(items, *focus);
                view.reconcile_annotations(items);
                view.invalidate_draw_cache();
            }
        }
        lists
    }
}

#[cfg(test)]
mod cleanup_tests {
    use super::*;
    use crate::tiles::{kind::LOGS, layout::Direction};

    fn saved(workspace: &Workspace) -> String {
        ron::to_string(&workspace.to_file().unwrap()).unwrap()
    }
    fn assert_valid(workspace: &Workspace) {
        Workspace::from_file(workspace.to_file().unwrap()).unwrap();
        assert_eq!(
            workspace
                .layout
                .tile_order()
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>(),
            workspace.tiles.keys().copied().collect()
        );
    }

    #[test]
    fn prepared_insertions_reject_invalid_membership_resources_and_focus_atomically() {
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
        let list = workspace.tiles[&first].kind.waveform_list().unwrap();
        let original = saved(&workspace);
        let revision = workspace.layout.revision();
        let token = runtime.request(first).unwrap();
        let valid = workspace.tiles[&first].clone();
        let missing = TileEntry {
            title: None,
            kind: TileKind::Waveform(Box::new(crate::tile_kinds::waveform::WaveformTile::new(
                ItemListId(999),
            ))),
        };
        let mut invalid_focus = valid.clone();
        if let TileKind::Waveform(tile) = &mut invalid_focus.kind {
            tile.view.focused_item = Some(crate::displayed_item::DisplayedItemRef(999));
        }
        let cases = [
            (valid.clone(), BTreeMap::new(), Placement::Root),
            (missing, BTreeMap::new(), Placement::TabAfter(first)),
            (
                valid.clone(),
                BTreeMap::from([(list, ItemList::default())]),
                Placement::TabAfter(first),
            ),
            (
                valid,
                BTreeMap::from([(ItemListId(999), ItemList::default())]),
                Placement::TabAfter(first),
            ),
            (invalid_focus, BTreeMap::new(), Placement::TabAfter(first)),
        ];
        for (entry, lists, placement) in cases {
            assert!(
                workspace
                    .insert_prepared(&mut runtime, entry, lists, placement, true)
                    .is_err()
            );
            assert_eq!(saved(&workspace), original);
            assert_eq!(workspace.layout.revision(), revision);
            assert!(runtime.accepts(token, Some(token)));
            assert_valid(&workspace);
        }
        workspace
            .apply_command(
                &mut runtime,
                WorkspaceCommand::CreateTile {
                    kind: LOGS.name.into(),
                    placement: Placement::Beside(first, Direction::Right),
                    focus: true,
                },
            )
            .unwrap();
        assert_valid(&workspace);
    }

    #[test]
    fn unknown_owner_does_not_hide_a_known_missing_dependency_on_load() {
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
        let unknown =
            TileEntry::from_file(decode(include_str!("fixtures/future-tile.ron")).unwrap())
                .unwrap();
        workspace
            .insert_prepared(
                &mut runtime,
                unknown,
                BTreeMap::new(),
                Placement::Edge(Direction::Right),
                false,
            )
            .unwrap();
        let mut file = workspace.to_file().unwrap();
        file.item_lists.clear();
        assert!(matches!(
            Workspace::from_file(file),
            Err(WorkspaceError::Resource(ResourceError::Missing(_)))
        ));
    }

    #[test]
    fn unknown_owner_close_undo_redo_preserves_payload_and_conservative_resources() {
        use crate::Message;
        let mut state = crate::SystemState::new_default_config().unwrap();
        state
            .update(Message::Workspace(WorkspaceCommand::CreateTile {
                kind: WAVEFORM.name.into(),
                placement: Placement::Root,
                focus: true,
            }))
            .unwrap();
        let waveform = state.user.workspace.layout.focused().unwrap();
        let unknown =
            TileEntry::from_file(decode(include_str!("fixtures/future-tile.ron")).unwrap())
                .unwrap();
        let payload = unknown.to_file().unwrap().payload.get_ron().to_owned();
        let extra = state.workspace_runtime.allocate_list().unwrap();
        let id = state
            .user
            .workspace
            .insert_prepared(
                &mut state.workspace_runtime,
                unknown,
                [(extra, ItemList::default())].into(),
                Placement::Edge(Direction::Right),
                true,
            )
            .unwrap();
        state
            .update(Message::Workspace(WorkspaceCommand::CloseTile(waveform)))
            .unwrap();
        assert_eq!(state.user.workspace.item_lists.len(), 2);
        assert_valid(&state.user.workspace);
        state.update(Message::Undo(1)).unwrap();
        assert_valid(&state.user.workspace);
        state
            .update(Message::Workspace(WorkspaceCommand::CloseTile(id)))
            .unwrap();
        assert!(!state.user.workspace.item_lists.contains_key(&extra));
        assert_valid(&state.user.workspace);
        state.update(Message::Undo(1)).unwrap();
        assert!(state.user.workspace.item_lists.contains_key(&extra));
        assert_eq!(
            state.user.workspace.tiles[&id]
                .to_file()
                .unwrap()
                .payload
                .get_ron(),
            payload
        );
        assert_valid(&state.user.workspace);
        state.update(Message::Redo(1)).unwrap();
        assert!(!state.user.workspace.item_lists.contains_key(&extra));
        assert_valid(&state.user.workspace);
    }

    #[test]
    fn lifecycle_changes_reject_old_completions_and_never_recycle_closed_ids() {
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
        let old = runtime.request(first).unwrap();
        let file = saved(&workspace);
        workspace.replace(&mut runtime, &file).unwrap();
        assert!(!runtime.accepts(old, Some(old)));
        let current = runtime.request(first).unwrap();
        runtime.document_changed().unwrap();
        assert!(!runtime.accepts(current, Some(current)));
        workspace
            .apply_command(&mut runtime, WorkspaceCommand::CloseTile(first))
            .unwrap();
        assert_eq!(workspace.resolve_tile(TileTarget::Id(first)), None);
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
        assert_ne!(workspace.layout.focused(), Some(first));
        assert_valid(&workspace);
    }
}
