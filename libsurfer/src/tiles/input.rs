//! Resolve chrome intent (keyboard, palette, toolbar, menus, tab bar) into
//! concrete workspace commands. Every returned command names tiles that exist
//! at resolution time; nothing here mutates state or reveals hidden tabs.

use super::{
    TileId, TileTarget,
    commands::{SplitMode, WorkspaceCommand},
    kind::KINDS,
    layout::{Direction, Placement},
    workspace::Workspace,
};

/// The tiles a chrome command acts on, captured once at the input boundary.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CommandTarget {
    /// The focused tile, for generic and kind-specific commands.
    pub tile: Option<TileId>,
    /// The declared waveform fallback (§5.1), for waveform commands from chrome.
    pub waveform: Option<TileId>,
}

/// Resolve `query` against titles computed by `Workspace::titles`.
pub fn find_tile(
    titles: &std::collections::BTreeMap<TileId, String>,
    query: &str,
) -> Option<TileId> {
    let query = query.trim();
    if let Some(id) = query.strip_prefix('#').and_then(|id| id.parse().ok()) {
        return titles.contains_key(&TileId(id)).then_some(TileId(id));
    }
    if let Some((id, _)) = titles.iter().find(|(_, title)| title.as_str() == query) {
        return Some(*id);
    }
    let lower = query.to_lowercase();
    let mut matches = titles
        .iter()
        .filter(|(_, title)| title.to_lowercase().starts_with(&lower))
        .map(|(id, _)| *id);
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

impl Workspace {
    pub fn command_target(&self) -> CommandTarget {
        CommandTarget {
            tile: self.resolve_tile(TileTarget::Focused),
            waveform: self.resolve_waveform(TileTarget::Focused),
        }
    }

    /// Split with the mode that keeps shared content: linked for waveforms and
    /// clone for kinds supporting it. `copy` requests an independent list copy.
    pub fn split_command(
        &self,
        target: TileTarget,
        dir: Direction,
        copy: bool,
    ) -> Option<WorkspaceCommand> {
        let tile = self.resolve_tile(target)?;
        let kind = &self.tiles().get(&tile)?.kind;
        let mode = match (kind.is_waveform(), copy) {
            (true, false) => SplitMode::Linked,
            (true, true) => SplitMode::Independent,
            (false, false) => SplitMode::Clone,
            (false, true) => return None,
        };
        kind.supports_split(mode)
            .then_some(WorkspaceCommand::SplitTile { tile, dir, mode })
    }

    pub fn close_command(&self, target: TileTarget) -> Option<WorkspaceCommand> {
        Some(WorkspaceCommand::CloseTile(self.resolve_tile(target)?))
    }

    pub fn close_others_command(&self, target: TileTarget) -> Option<WorkspaceCommand> {
        Some(WorkspaceCommand::CloseOtherTiles(
            self.resolve_tile(target)?,
        ))
    }

    pub fn rename_command(
        &self,
        target: TileTarget,
        title: Option<String>,
    ) -> Option<WorkspaceCommand> {
        let tile = self.resolve_tile(target)?;
        Some(WorkspaceCommand::RenameTile { tile, title })
    }

    /// Focus the visible spatial neighbor, using the last completed geometry.
    pub fn focus_neighbor_command(
        &self,
        target: TileTarget,
        dir: Direction,
    ) -> Option<WorkspaceCommand> {
        let from = self.resolve_tile(target)?;
        Some(WorkspaceCommand::FocusTile(
            self.layout().neighbor(from, dir)?,
        ))
    }

    /// Activate the adjacent tab in the target's group, wrapping around.
    pub fn cycle_tab_command(&self, target: TileTarget, delta: isize) -> Option<WorkspaceCommand> {
        let from = self.resolve_tile(target)?;
        let next = self.layout().next_in_group(from, delta)?;
        (next != from).then_some(WorkspaceCommand::FocusTile(next))
    }

    /// Move one step: past the visible neighbor in that direction, or to the
    /// workspace edge when the target already borders it.
    pub fn move_command(&self, target: TileTarget, dir: Direction) -> Option<WorkspaceCommand> {
        let tile = self.resolve_tile(target)?;
        let to = match self.layout().neighbor(tile, dir) {
            Some(neighbor) => Placement::Beside(neighbor, dir),
            None => Placement::Edge(dir),
        };
        Some(WorkspaceCommand::MoveTile { tile, to })
    }

    /// Open a kind beside the anchor (the focused tile by default), or as the
    /// root of an empty workspace. Singleton kinds are revealed instead.
    pub fn open_command(&self, kind: &str, anchor: Option<TileId>) -> Option<WorkspaceCommand> {
        if !KINDS.iter().any(|descriptor| descriptor.name == kind) {
            return None;
        }
        let placement = anchor
            .filter(|id| self.tiles().contains_key(id))
            .or_else(|| self.layout().focused())
            .map_or(Placement::Root, |id| {
                Placement::Beside(id, Direction::Right)
            });
        Some(WorkspaceCommand::OpenTile {
            kind: kind.into(),
            placement,
            focus: true,
        })
    }

    /// Keep only the target waveform (creating one when none exists).
    pub fn reset_command(&self) -> WorkspaceCommand {
        WorkspaceCommand::Reset {
            keep: self.resolve_waveform(TileTarget::Focused),
        }
    }

    /// Identify a tile by its numeric ID (`#3`), its exact display title, or a
    /// unique case-insensitive title prefix.
    pub fn find_tile(&self, query: &str) -> Option<TileId> {
        find_tile(&self.titles(), query)
    }

    /// Palette suggestions for `tile_focus`: `#id` and display titles.
    pub fn tile_suggestions(&self) -> Vec<String> {
        self.layout()
            .tile_order()
            .into_iter()
            .filter_map(|id| {
                let title = self.titles().remove(&id)?;
                Some([format!("#{}", id.0), title])
            })
            .flatten()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiles::runtime::WorkspaceRuntime;
    use std::collections::BTreeMap;

    fn create(
        workspace: &mut Workspace,
        runtime: &mut WorkspaceRuntime,
        kind: &str,
        placement: Placement,
    ) -> TileId {
        workspace
            .apply_command(
                runtime,
                WorkspaceCommand::CreateTile {
                    kind: kind.into(),
                    placement,
                    focus: true,
                },
            )
            .unwrap();
        workspace.layout().focused().unwrap()
    }

    fn rect(x: f32, y: f32) -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(100.0, 100.0))
    }

    #[test]
    fn splits_pick_the_mode_that_keeps_shared_content() {
        let mut workspace = Workspace::default();
        let mut runtime = WorkspaceRuntime::default();
        assert!(
            workspace
                .split_command(TileTarget::Focused, Direction::Right, false)
                .is_none()
        );
        let waveform = create(&mut workspace, &mut runtime, "waveform", Placement::Root);
        let memory = create(
            &mut workspace,
            &mut runtime,
            "memory",
            Placement::TabAfter(waveform),
        );
        let logs = create(
            &mut workspace,
            &mut runtime,
            "logs",
            Placement::TabAfter(memory),
        );
        assert!(matches!(
            workspace.split_command(TileTarget::Id(waveform), Direction::Down, false),
            Some(WorkspaceCommand::SplitTile { tile, dir: Direction::Down, mode: SplitMode::Linked }) if tile == waveform
        ));
        assert!(matches!(
            workspace.split_command(TileTarget::Id(waveform), Direction::Right, true),
            Some(WorkspaceCommand::SplitTile {
                mode: SplitMode::Independent,
                ..
            })
        ));
        assert!(matches!(
            workspace.split_command(TileTarget::Id(memory), Direction::Right, false),
            Some(WorkspaceCommand::SplitTile {
                mode: SplitMode::Clone,
                ..
            })
        ));
        assert!(
            workspace
                .split_command(TileTarget::Id(memory), Direction::Right, true)
                .is_none()
        );
        assert!(
            workspace
                .split_command(TileTarget::Id(logs), Direction::Right, false)
                .is_none()
        );
        assert!(
            workspace
                .split_command(TileTarget::Id(TileId(99)), Direction::Right, false)
                .is_none()
        );
        assert!(matches!(
            workspace.close_command(TileTarget::Focused),
            Some(WorkspaceCommand::CloseTile(id)) if id == logs
        ));
        assert!(
            workspace
                .close_command(TileTarget::Id(TileId(99)))
                .is_none()
        );
    }

    #[test]
    fn navigation_uses_rendered_geometry_and_moves_step_past_neighbors() {
        let mut workspace = Workspace::default();
        let mut runtime = WorkspaceRuntime::default();
        let left = create(&mut workspace, &mut runtime, "waveform", Placement::Root);
        let right = create(
            &mut workspace,
            &mut runtime,
            "waveform",
            Placement::Beside(left, Direction::Right),
        );
        let hidden = create(
            &mut workspace,
            &mut runtime,
            "waveform",
            Placement::TabAfter(right),
        );
        workspace
            .apply_command(&mut runtime, WorkspaceCommand::FocusTile(right))
            .unwrap();
        // Without geometry, moving still resolves against the workspace edge.
        assert!(matches!(
            workspace.move_command(TileTarget::Focused, Direction::Left),
            Some(WorkspaceCommand::MoveTile { tile, to: Placement::Edge(Direction::Left) }) if tile == right
        ));
        assert!(
            workspace
                .focus_neighbor_command(TileTarget::Focused, Direction::Left)
                .is_none()
        );
        workspace
            .set_geometry(
                workspace.layout().revision(),
                BTreeMap::from([(left, rect(0.0, 0.0)), (right, rect(100.0, 0.0))]),
            )
            .unwrap();
        assert!(matches!(
            workspace.focus_neighbor_command(TileTarget::Focused, Direction::Left),
            Some(WorkspaceCommand::FocusTile(id)) if id == left
        ));
        assert!(
            workspace
                .focus_neighbor_command(TileTarget::Focused, Direction::Right)
                .is_none()
        );
        assert!(matches!(
            workspace.move_command(TileTarget::Focused, Direction::Left),
            Some(WorkspaceCommand::MoveTile { tile, to: Placement::Beside(anchor, Direction::Left) }) if tile == right && anchor == left
        ));
        assert!(matches!(
            workspace.cycle_tab_command(TileTarget::Focused, 1),
            Some(WorkspaceCommand::FocusTile(id)) if id == hidden
        ));
        assert!(
            workspace
                .cycle_tab_command(TileTarget::Id(left), 1)
                .is_none()
        );
        // Hidden tabs never take part in spatial navigation.
        assert!(
            workspace
                .focus_neighbor_command(TileTarget::Id(hidden), Direction::Left)
                .is_none()
        );
    }

    #[test]
    fn opening_and_finding_tiles_uses_titles_and_ids() {
        let mut workspace = Workspace::default();
        let mut runtime = WorkspaceRuntime::default();
        assert!(matches!(
            workspace.open_command("logs", None),
            Some(WorkspaceCommand::OpenTile {
                placement: Placement::Root,
                focus: true,
                ..
            })
        ));
        assert!(workspace.open_command("nonsense", None).is_none());
        let first = create(&mut workspace, &mut runtime, "waveform", Placement::Root);
        let second = create(
            &mut workspace,
            &mut runtime,
            "waveform",
            Placement::TabAfter(first),
        );
        assert!(matches!(
            workspace.open_command("memory", Some(first)),
            Some(WorkspaceCommand::OpenTile { placement: Placement::Beside(anchor, Direction::Right), .. }) if anchor == first
        ));
        assert!(matches!(
            workspace.open_command("memory", Some(TileId(99))),
            Some(WorkspaceCommand::OpenTile { placement: Placement::Beside(anchor, Direction::Right), .. }) if anchor == second
        ));
        workspace
            .apply_command(
                &mut runtime,
                WorkspaceCommand::RenameTile {
                    tile: second,
                    title: Some("Counter".into()),
                },
            )
            .unwrap();
        assert_eq!(workspace.find_tile("#1"), Some(first));
        assert_eq!(workspace.find_tile("#99"), None);
        assert_eq!(workspace.find_tile("Counter"), Some(second));
        assert_eq!(workspace.find_tile("cou"), Some(second));
        assert_eq!(workspace.find_tile("Waveform 1"), Some(first));
        assert_eq!(workspace.find_tile("wave"), Some(first));
        assert_eq!(workspace.find_tile("zzz"), None);
        assert_eq!(
            workspace.tile_suggestions(),
            vec!["#1", "Waveform 1", "#2", "Counter"]
        );
        assert!(matches!(
            workspace.reset_command(),
            WorkspaceCommand::Reset { keep: Some(id) } if id == second
        ));
    }
}
