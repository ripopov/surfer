//! Library-independent layout contract. Mutations validate a candidate before committing.

use std::collections::{BTreeMap, BTreeSet};

use egui::Rect;
use serde::{Deserialize, Serialize};

use super::TileId;

pub const MAX_LAYOUT_DEPTH: usize = 64;
pub const MAX_LAYOUT_NODES: usize = 4096;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitDir {
    Horizontal,
    Vertical,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    fn axis(self) -> SplitDir {
        match self {
            Self::Left | Self::Right => SplitDir::Horizontal,
            Self::Up | Self::Down => SplitDir::Vertical,
        }
    }

    fn before(self) -> bool {
        matches!(self, Self::Left | Self::Up)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    TabAfter(TileId),
    Beside(TileId, Direction),
    Edge(Direction),
    Root,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum LayoutNode {
    Tile(TileId),
    Split {
        dir: SplitDir,
        shares: Vec<f32>,
        children: Vec<LayoutNode>,
    },
    Tabs {
        active: usize,
        children: Vec<LayoutNode>,
    },
}

/// Shares and active tabs are navigation, not topology.
pub(crate) fn same_topology(left: Option<&LayoutNode>, right: Option<&LayoutNode>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(LayoutNode::Tile(a)), Some(LayoutNode::Tile(b))) => a == b,
        (
            Some(LayoutNode::Tabs { children: a, .. }),
            Some(LayoutNode::Tabs { children: b, .. }),
        ) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|(a, b)| same_topology(Some(a), Some(b)))
        }
        (
            Some(LayoutNode::Split {
                dir: a_dir,
                children: a,
                ..
            }),
            Some(LayoutNode::Split {
                dir: b_dir,
                children: b,
                ..
            }),
        ) => {
            a_dir == b_dir
                && a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|(a, b)| same_topology(Some(a), Some(b)))
        }
        _ => false,
    }
}

/// Persistence contains no egui node IDs, cached geometry or adapter state.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct LayoutFile {
    pub root: Option<LayoutNode>,
    pub focused: Option<TileId>,
    pub focus_history: Vec<TileId>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LayoutError {
    #[error("layout exceeds depth or node-count limit")]
    Limit,
    #[error("invalid tile ID {0:?}")]
    InvalidId(TileId),
    #[error("tile {0:?} appears more than once")]
    Duplicate(TileId),
    #[error("layout tile IDs do not match the workspace")]
    TileSet,
    #[error("split shares must match children and be finite and positive")]
    Shares,
    #[error("tab index is out of range")]
    ActiveTab,
    #[error("tabs may contain only tile leaves")]
    NestedTabs,
    #[error("tile {0:?} does not exist")]
    Missing(TileId),
    #[error("root placement requires an empty layout")]
    NonemptyRoot,
    #[error("a tile cannot be placed relative to itself")]
    SelfPlacement,
    #[error("stale layout proposal")]
    StaleRevision,
    #[error("layout revision exhausted")]
    RevisionExhausted,
}

#[derive(Debug, Default, Clone)]
pub struct Layout {
    root: Option<LayoutNode>,
    focused: Option<TileId>,
    focus_history: Vec<TileId>,
    revision: u64,
    /// Only visible panes from a completed adapter pass at the current revision.
    geometry: BTreeMap<TileId, Rect>,
}

impl LayoutNode {
    fn visit_tiles(&self, visible_only: bool, result: &mut Vec<TileId>) {
        match self {
            Self::Tile(id) => result.push(*id),
            Self::Tabs { active, children } if visible_only => {
                if let Some(child) = children.get(*active) {
                    child.visit_tiles(true, result);
                }
            }
            Self::Tabs { children, .. } | Self::Split { children, .. } => {
                for child in children {
                    child.visit_tiles(visible_only, result);
                }
            }
        }
    }

    fn contains(&self, tile: TileId) -> bool {
        match self {
            Self::Tile(id) => *id == tile,
            Self::Tabs { children, .. } | Self::Split { children, .. } => {
                children.iter().any(|child| child.contains(tile))
            }
        }
    }

    fn group_mut(&mut self, tile: TileId) -> Option<&mut Self> {
        match self {
            Self::Tabs { .. } if self.contains(tile) => Some(self),
            Self::Split { children, .. } => {
                children.iter_mut().find_map(|child| child.group_mut(tile))
            }
            _ => None,
        }
    }

    fn group(&self, tile: TileId) -> Option<&Self> {
        match self {
            Self::Tabs { .. } if self.contains(tile) => Some(self),
            Self::Split { children, .. } => children.iter().find_map(|child| child.group(tile)),
            _ => None,
        }
    }

    fn activate(&mut self, tile: TileId) {
        if let Some(Self::Tabs { active, children }) = self.group_mut(tile) {
            *active = children
                .iter()
                .position(|child| child.contains(tile))
                .unwrap();
        }
    }

    /// Remove a leaf and empty ancestors while retaining surviving split proportions.
    pub(crate) fn without(self, tile: TileId) -> Option<Self> {
        match self {
            Self::Tile(id) => (id != tile).then_some(Self::Tile(id)),
            Self::Tabs {
                active,
                mut children,
            } => {
                if let Some(index) = children.iter().position(|child| child.contains(tile)) {
                    children.remove(index);
                    if children.is_empty() {
                        return None;
                    }
                    let active = if index < active {
                        active - 1
                    } else {
                        active.min(children.len() - 1)
                    };
                    Some(Self::Tabs { active, children })
                } else {
                    Some(Self::Tabs { active, children })
                }
            }
            Self::Split {
                dir,
                shares,
                children,
            } => {
                let (shares, children): (Vec<_>, Vec<_>) = shares
                    .into_iter()
                    .zip(children)
                    .filter_map(|(share, child)| child.without(tile).map(|child| (share, child)))
                    .unzip();
                normalize(
                    Self::Split {
                        dir,
                        shares,
                        children,
                    },
                    false,
                )
            }
        }
    }
}

fn validate_root(root: Option<&LayoutNode>) -> Result<BTreeSet<TileId>, LayoutError> {
    let mut ids = BTreeSet::new();
    let mut stack = root.into_iter().map(|node| (node, 1)).collect::<Vec<_>>();
    let mut count = 0;
    while let Some((node, depth)) = stack.pop() {
        count += 1;
        if depth > MAX_LAYOUT_DEPTH || count > MAX_LAYOUT_NODES {
            return Err(LayoutError::Limit);
        }
        match node {
            LayoutNode::Tile(id) => {
                if id.0 == 0 || id.0 == u64::MAX {
                    return Err(LayoutError::InvalidId(*id));
                }
                if !ids.insert(*id) {
                    return Err(LayoutError::Duplicate(*id));
                }
            }
            LayoutNode::Tabs { active, children } => {
                if (*active != 0 && children.is_empty())
                    || (!children.is_empty() && *active >= children.len())
                {
                    return Err(LayoutError::ActiveTab);
                }
                if children
                    .iter()
                    .any(|child| !matches!(child, LayoutNode::Tile(_)))
                {
                    return Err(LayoutError::NestedTabs);
                }
                stack.extend(children.iter().map(|child| (child, depth + 1)));
            }
            LayoutNode::Split {
                shares, children, ..
            } => {
                if shares.len() != children.len()
                    || shares.iter().any(|s| !s.is_finite() || *s <= 0.0)
                {
                    return Err(LayoutError::Shares);
                }
                stack.extend(children.iter().map(|child| (child, depth + 1)));
            }
        }
    }
    Ok(ids)
}

/// Called only after bounded validation, so recursion is bounded too.
fn normalize(node: LayoutNode, in_tabs: bool) -> Option<LayoutNode> {
    match node {
        LayoutNode::Tile(id) if !in_tabs => Some(LayoutNode::Tabs {
            active: 0,
            children: vec![LayoutNode::Tile(id)],
        }),
        LayoutNode::Tile(_) => Some(node),
        LayoutNode::Tabs { ref children, .. } => (!children.is_empty()).then_some(node),
        LayoutNode::Split {
            dir,
            shares,
            children,
        } => {
            let mut result = Vec::new();
            let mut weights = Vec::new();
            for (share, child) in shares.into_iter().zip(children) {
                if let Some(child) = normalize(child, false) {
                    result.push(child);
                    weights.push(share);
                }
            }
            match result.len() {
                0 => None,
                1 => result.pop(),
                _ => {
                    // Sum in f64 to avoid overflowing otherwise valid f32 shares.
                    let sum: f64 = weights.iter().map(|s| f64::from(*s)).sum();
                    for share in &mut weights {
                        // Keep extreme but valid shares positive after rounding.
                        *share = (f64::from(*share) / sum).max(f64::from(f32::MIN_POSITIVE)) as f32;
                    }
                    Some(LayoutNode::Split {
                        dir,
                        shares: weights,
                        children: result,
                    })
                }
            }
        }
    }
}

fn split(existing: LayoutNode, tile: TileId, direction: Direction) -> LayoutNode {
    let mut children = vec![
        existing,
        LayoutNode::Tabs {
            active: 0,
            children: vec![LayoutNode::Tile(tile)],
        },
    ];
    if direction.before() {
        children.swap(0, 1);
    }
    LayoutNode::Split {
        dir: direction.axis(),
        shares: vec![0.5, 0.5],
        children,
    }
}

impl Layout {
    /// Repairs saved stale/duplicate history and stale focus, normalizes shares and
    /// empty/redundant containers. Missing/duplicate tile IDs are always errors.
    pub fn from_file(file: LayoutFile, tiles: &BTreeSet<TileId>) -> Result<Self, LayoutError> {
        let ids = validate_root(file.root.as_ref())?;
        if &ids != tiles {
            return Err(LayoutError::TileSet);
        }
        let root = file.root.and_then(|root| normalize(root, false));
        // Wrapping bare leaves can add a level and nodes.
        validate_root(root.as_ref())?;
        let mut seen = BTreeSet::new();
        let focus_history = file
            .focus_history
            .into_iter()
            .filter(|id| ids.contains(id) && seen.insert(*id))
            .collect();
        let mut result = Self {
            root,
            focused: file.focused.filter(|id| ids.contains(id)),
            focus_history,
            ..Default::default()
        };
        result.repair_focus();
        Ok(result)
    }

    pub fn to_file(&self) -> LayoutFile {
        LayoutFile {
            root: self.root.clone(),
            focused: self.focused,
            focus_history: self.focus_history.clone(),
        }
    }

    pub fn root(&self) -> Option<&LayoutNode> {
        self.root.as_ref()
    }
    pub fn focused(&self) -> Option<TileId> {
        self.focused
    }
    pub fn focus_history(&self) -> &[TileId] {
        &self.focus_history
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn tile_order(&self) -> Vec<TileId> {
        let mut result = Vec::new();
        if let Some(root) = &self.root {
            root.visit_tiles(false, &mut result);
        }
        result
    }

    pub fn visible_tiles(&self) -> Vec<TileId> {
        let mut result = Vec::new();
        if let Some(root) = &self.root {
            root.visit_tiles(true, &mut result);
        }
        result
    }

    fn repair_focus(&mut self) {
        let all = self.tile_order();
        self.focus_history.retain(|id| all.contains(id));
        if !self.focused.is_some_and(|id| all.contains(&id)) {
            let visible = self.visible_tiles();
            self.focused = self
                .focus_history
                .iter()
                .find(|id| visible.contains(id))
                .copied()
                .or_else(|| visible.first().copied());
        }
        if let Some(id) = self.focused {
            if let Some(root) = &mut self.root {
                root.activate(id);
            }
            self.focus_history.retain(|old| *old != id);
            self.focus_history.insert(0, id);
        }
    }

    fn commit(&mut self, mut file: LayoutFile) -> Result<bool, LayoutError> {
        let ids = validate_root(file.root.as_ref())?;
        file.root = file.root.and_then(|root| normalize(root, false));
        let mut next = Self::from_file(file, &ids)?;
        if next.to_file() == self.to_file() {
            return Ok(false);
        }
        next.revision = self
            .revision
            .checked_add(1)
            .ok_or(LayoutError::RevisionExhausted)?;
        *self = next;
        Ok(true)
    }

    pub fn focus(&mut self, tile: TileId) -> Result<bool, LayoutError> {
        if !self.tile_order().contains(&tile) {
            return Err(LayoutError::Missing(tile));
        }
        let mut file = self.to_file();
        file.focused = Some(tile);
        self.commit(file)
    }

    pub fn insert(&mut self, tile: TileId, placement: Placement) -> Result<bool, LayoutError> {
        if self.tile_order().contains(&tile) {
            return Err(LayoutError::Duplicate(tile));
        }
        let mut file = self.to_file();
        Self::insert_into(&mut file.root, tile, placement)?;
        self.commit(file)
    }

    fn insert_into(
        root: &mut Option<LayoutNode>,
        tile: TileId,
        placement: Placement,
    ) -> Result<(), LayoutError> {
        match placement {
            Placement::Root => {
                if root.is_some() {
                    return Err(LayoutError::NonemptyRoot);
                }
                *root = Some(LayoutNode::Tile(tile));
            }
            Placement::Edge(dir) => {
                *root = Some(match root.take() {
                    Some(old) => split(old, tile, dir),
                    None => LayoutNode::Tile(tile),
                });
            }
            Placement::TabAfter(anchor) => {
                let group = root
                    .as_mut()
                    .and_then(|root| root.group_mut(anchor))
                    .ok_or(LayoutError::Missing(anchor))?;
                let LayoutNode::Tabs { children, active } = group else {
                    unreachable!()
                };
                let index = children
                    .iter()
                    .position(|node| node.contains(anchor))
                    .unwrap()
                    + 1;
                children.insert(index, LayoutNode::Tile(tile));
                if index <= *active {
                    *active += 1;
                }
            }
            Placement::Beside(anchor, dir) => {
                let group = root
                    .as_mut()
                    .and_then(|root| root.group_mut(anchor))
                    .ok_or(LayoutError::Missing(anchor))?;
                *group = split(group.clone(), tile, dir);
            }
        }
        Ok(())
    }

    pub fn remove(&mut self, tile: TileId) -> Result<bool, LayoutError> {
        if !self.tile_order().contains(&tile) {
            return Err(LayoutError::Missing(tile));
        }
        let mut file = self.to_file();
        file.root = file.root.and_then(|root| root.without(tile));
        self.commit(file)
    }

    pub fn move_tile(&mut self, tile: TileId, placement: Placement) -> Result<bool, LayoutError> {
        if !self.tile_order().contains(&tile) {
            return Err(LayoutError::Missing(tile));
        }
        if matches!(placement, Placement::TabAfter(id) | Placement::Beside(id, _) if id == tile) {
            return Err(LayoutError::SelfPlacement);
        }
        let mut file = self.to_file();
        file.root = file.root.and_then(|root| root.without(tile));
        Self::insert_into(&mut file.root, tile, placement)?;
        self.commit(file)
    }

    /// Adapter proposals must enumerate exactly the current tile set.
    /// History may restore removed resources. Validate the new membership before
    /// committing while retaining current focus history and monotonic revisions.
    pub(crate) fn restore_topology(
        &mut self,
        root: Option<LayoutNode>,
        tiles: &BTreeSet<TileId>,
    ) -> Result<bool, LayoutError> {
        if &validate_root(root.as_ref())? != tiles {
            return Err(LayoutError::TileSet);
        }
        self.commit(LayoutFile {
            root,
            focused: self.focused,
            focus_history: self.focus_history.clone(),
        })
    }

    pub fn apply_proposal(
        &mut self,
        revision: u64,
        root: Option<LayoutNode>,
        focused: Option<TileId>,
    ) -> Result<bool, LayoutError> {
        if revision != self.revision {
            return Err(LayoutError::StaleRevision);
        }
        let ids = validate_root(root.as_ref())?;
        if ids != self.tile_order().into_iter().collect() {
            return Err(LayoutError::TileSet);
        }
        if let Some(id) = focused
            && !ids.contains(&id)
        {
            return Err(LayoutError::Missing(id));
        }
        let file = LayoutFile {
            root,
            focused,
            focus_history: self.focus_history.clone(),
        };
        self.commit(file)
    }

    pub fn next_in_group(&self, from: TileId, delta: isize) -> Option<TileId> {
        let LayoutNode::Tabs { children, .. } = self.root.as_ref()?.group(from)? else {
            return None;
        };
        let index = children.iter().position(|child| child.contains(from))?;
        let next = (index as isize + delta.rem_euclid(children.len() as isize))
            .rem_euclid(children.len() as isize) as usize;
        let LayoutNode::Tile(id) = children[next] else {
            return None;
        };
        Some(id)
    }

    /// The adapter submits one complete set of current visible pane rectangles.
    pub fn set_geometry(
        &mut self,
        revision: u64,
        rects: BTreeMap<TileId, Rect>,
    ) -> Result<(), LayoutError> {
        if revision != self.revision {
            return Err(LayoutError::StaleRevision);
        }
        let visible = self.visible_tiles();
        self.geometry = rects
            .into_iter()
            .filter(|(id, rect)| visible.contains(id) && rect.is_finite() && rect.is_positive())
            .collect();
        Ok(())
    }

    pub fn rect(&self, tile: TileId) -> Option<Rect> {
        self.geometry.get(&tile).copied()
    }

    pub fn neighbor(&self, from: TileId, direction: Direction) -> Option<TileId> {
        let origin = self.rect(from)?;
        let center = origin.center();
        self.geometry
            .iter()
            .filter_map(|(id, rect)| {
                if *id == from {
                    return None;
                }
                let other = rect.center();
                let (forward, cross, overlap) = match direction {
                    Direction::Left => (
                        center.x - other.x,
                        (center.y - other.y).abs(),
                        origin.y_range().intersects(rect.y_range()),
                    ),
                    Direction::Right => (
                        other.x - center.x,
                        (center.y - other.y).abs(),
                        origin.y_range().intersects(rect.y_range()),
                    ),
                    Direction::Up => (
                        center.y - other.y,
                        (center.x - other.x).abs(),
                        origin.x_range().intersects(rect.x_range()),
                    ),
                    Direction::Down => (
                        other.y - center.y,
                        (center.x - other.x).abs(),
                        origin.x_range().intersects(rect.x_range()),
                    ),
                };
                (forward > 0.0).then_some((*id, !overlap, forward + cross))
            })
            .min_by(|a, b| {
                a.1.cmp(&b.1)
                    .then_with(|| a.2.total_cmp(&b.2))
                    .then_with(|| a.0.cmp(&b.0))
            })
            .map(|(id, _, _)| id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        let mut layout = Layout::default();
        layout.insert(TileId(1), Placement::Root).unwrap();
        layout
            .insert(TileId(2), Placement::TabAfter(TileId(1)))
            .unwrap();
        layout
            .insert(TileId(3), Placement::Beside(TileId(1), Direction::Right))
            .unwrap();
        layout
    }

    #[test]
    fn focus_reveals_tabs_and_navigation_wraps() {
        let mut layout = layout();
        assert_eq!(layout.visible_tiles(), [TileId(1), TileId(3)]);
        layout.focus(TileId(2)).unwrap();
        assert_eq!(layout.visible_tiles(), [TileId(2), TileId(3)]);
        assert_eq!(layout.focus_history()[0], TileId(2));
        assert_eq!(layout.next_in_group(TileId(1), -1), Some(TileId(2)));
        assert_eq!(layout.next_in_group(TileId(2), 1), Some(TileId(1)));
    }

    #[test]
    fn move_collapses_empty_containers_and_close_last_is_empty() {
        let mut layout = layout();
        layout
            .move_tile(TileId(3), Placement::TabAfter(TileId(2)))
            .unwrap();
        assert!(matches!(layout.root(), Some(LayoutNode::Tabs { .. })));
        assert_eq!(layout.tile_order(), [TileId(1), TileId(2), TileId(3)]);
        layout.focus(TileId(2)).unwrap();
        layout.remove(TileId(2)).unwrap();
        assert_eq!(layout.visible_tiles(), [TileId(3)]);
        layout.remove(TileId(1)).unwrap();
        assert!(matches!(layout.root(), Some(LayoutNode::Tabs { .. })));
        layout.remove(TileId(3)).unwrap();
        assert!(layout.root().is_none());
        assert_eq!(layout.focused(), None);
        assert!(layout.focus_history().is_empty());
    }

    #[test]
    fn failed_edits_are_atomic_and_proposals_cannot_resurrect_closed_tiles() {
        let mut layout = layout();
        let before = layout.to_file();
        assert!(
            layout
                .move_tile(TileId(1), Placement::TabAfter(TileId(90)))
                .is_err()
        );
        assert!(
            layout
                .move_tile(TileId(1), Placement::TabAfter(TileId(1)))
                .is_err()
        );
        assert!(layout.insert(TileId(4), Placement::Root).is_err());
        assert!(
            layout
                .insert(TileId(1), Placement::Edge(Direction::Down))
                .is_err()
        );
        assert!(
            layout
                .insert(TileId(0), Placement::Edge(Direction::Down))
                .is_err()
        );
        assert_eq!(layout.to_file(), before);
        let revision = layout.revision();
        layout.remove(TileId(2)).unwrap();
        assert_eq!(
            layout.apply_proposal(revision, before.root.clone(), before.focused),
            Err(LayoutError::StaleRevision)
        );
        assert_eq!(
            layout.apply_proposal(layout.revision(), before.root, before.focused),
            Err(LayoutError::TileSet)
        );
    }

    #[test]
    fn round_trip_preserves_layout_and_repairs_stale_focus() {
        let layout = layout();
        let text = ron::to_string(&layout.to_file()).unwrap();
        let decoded = ron::from_str(&text).unwrap();
        let ids = layout.tile_order().into_iter().collect();
        let restored = Layout::from_file(decoded, &ids).unwrap();
        assert_eq!(restored.to_file(), layout.to_file());
        let mut file = restored.to_file();
        file.focused = Some(TileId(90));
        file.focus_history = vec![TileId(90), TileId(3), TileId(3)];
        let repaired = Layout::from_file(file, &ids).unwrap();
        assert_eq!(repaired.focused(), Some(TileId(3)));
        assert_eq!(repaired.focus_history(), [TileId(3)]);
    }

    #[test]
    fn reject_malformed_and_oversized_layouts() {
        let tile = LayoutNode::Tile(TileId(1));
        let tabs = |active, children| LayoutNode::Tabs { active, children };
        assert_eq!(
            validate_root(Some(&tabs(0, vec![tile.clone(), tile.clone()]))),
            Err(LayoutError::Duplicate(TileId(1)))
        );
        assert_eq!(
            validate_root(Some(&tabs(1, vec![tile.clone()]))),
            Err(LayoutError::ActiveTab)
        );
        assert_eq!(
            validate_root(Some(&tabs(0, vec![tabs(0, vec![tile.clone()])]))),
            Err(LayoutError::NestedTabs)
        );
        for shares in [
            vec![],
            vec![f32::NAN],
            vec![f32::INFINITY],
            vec![0.0],
            vec![-1.0],
        ] {
            let root = LayoutNode::Split {
                dir: SplitDir::Horizontal,
                shares,
                children: vec![tile.clone()],
            };
            assert_eq!(validate_root(Some(&root)), Err(LayoutError::Shares));
        }
        let mut deep = tile;
        for _ in 0..MAX_LAYOUT_DEPTH {
            deep = LayoutNode::Split {
                dir: SplitDir::Horizontal,
                shares: vec![1.0],
                children: vec![deep],
            };
        }
        assert_eq!(validate_root(Some(&deep)), Err(LayoutError::Limit));
        let wide = tabs(
            0,
            (1..=MAX_LAYOUT_NODES as u64)
                .map(|id| LayoutNode::Tile(TileId(id)))
                .collect(),
        );
        assert_eq!(validate_root(Some(&wide)), Err(LayoutError::Limit));
    }

    #[test]
    fn normalization_preserves_single_tabs_and_finite_shares() {
        let root = LayoutNode::Split {
            dir: SplitDir::Vertical,
            shares: vec![f32::MAX, f32::MAX],
            children: vec![LayoutNode::Tile(TileId(1)), LayoutNode::Tile(TileId(2))],
        };
        let layout = Layout::from_file(
            LayoutFile {
                root: Some(root),
                ..Default::default()
            },
            &[TileId(1), TileId(2)].into(),
        )
        .unwrap();
        let LayoutNode::Split {
            shares, children, ..
        } = layout.root().unwrap()
        else {
            panic!()
        };
        assert_eq!(shares, &[0.5, 0.5]);
        assert!(
            children
                .iter()
                .all(|child| matches!(child, LayoutNode::Tabs { .. }))
        );
    }

    #[test]
    fn spatial_navigation_ignores_hidden_and_stale_rectangles() {
        let mut layout = layout();
        let rect = |x| Rect::from_min_size(egui::pos2(x, 0.0), egui::vec2(100.0, 100.0));
        layout
            .set_geometry(
                layout.revision(),
                [
                    (TileId(1), rect(0.0)),
                    (TileId(2), rect(100.0)),
                    (TileId(3), rect(200.0)),
                ]
                .into(),
            )
            .unwrap();
        assert!(layout.rect(TileId(2)).is_none());
        assert_eq!(
            layout.neighbor(TileId(1), Direction::Right),
            Some(TileId(3))
        );
        assert_eq!(layout.neighbor(TileId(3), Direction::Left), Some(TileId(1)));
        let old_revision = layout.revision();
        layout.focus(TileId(2)).unwrap();
        assert_eq!(layout.neighbor(TileId(1), Direction::Right), None);
        assert_eq!(
            layout.set_geometry(old_revision, BTreeMap::new()),
            Err(LayoutError::StaleRevision)
        );
    }
}
