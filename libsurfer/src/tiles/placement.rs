//! Stable neighboring membership anchors for restoring a moved tile.
use super::{
    TileId,
    layout::{LayoutNode, SplitDir},
};

#[derive(Clone)]
pub(crate) struct TileLocation {
    tile: TileId,
    ancestors: Vec<Ancestor>,
}

#[derive(Clone)]
enum Ancestor {
    Tabs {
        order: Vec<TileId>,
        active: usize,
    },
    Split {
        dir: SplitDir,
        shares: Vec<f32>,
        index: usize,
        siblings: Vec<Vec<TileId>>,
    },
}

fn members(node: &LayoutNode) -> Vec<TileId> {
    let mut ids = match node {
        LayoutNode::Tile(id) => vec![*id],
        LayoutNode::Tabs { children, .. } | LayoutNode::Split { children, .. } => {
            children.iter().flat_map(members).collect()
        }
    };
    ids.sort_unstable();
    ids
}

impl TileLocation {
    pub(crate) fn capture(root: &LayoutNode, tile: TileId) -> Option<Self> {
        fn capture(node: &LayoutNode, tile: TileId, path: &mut Vec<Ancestor>) -> bool {
            match node {
                LayoutNode::Tile(id) => *id == tile,
                LayoutNode::Tabs { active, children } => {
                    if !children
                        .iter()
                        .any(|node| matches!(node, LayoutNode::Tile(id) if *id == tile))
                    {
                        return false;
                    }
                    path.push(Ancestor::Tabs {
                        order: children.iter().flat_map(members).collect(),
                        active: *active,
                    });
                    true
                }
                LayoutNode::Split {
                    dir,
                    shares,
                    children,
                } => {
                    let Some(index) = children.iter().position(|child| capture(child, tile, path))
                    else {
                        return false;
                    };
                    path.push(Ancestor::Split {
                        dir: *dir,
                        shares: shares.clone(),
                        index,
                        siblings: children
                            .iter()
                            .enumerate()
                            .map(|(i, child)| {
                                if i == index {
                                    Vec::new()
                                } else {
                                    members(child)
                                }
                            })
                            .collect(),
                    });
                    true
                }
            }
        }
        let mut ancestors = Vec::new();
        capture(root, tile, &mut ancestors).then_some(Self { tile, ancestors })
    }

    pub(crate) fn restore(&self, current: &LayoutNode) -> Option<LayoutNode> {
        fn find<'a>(node: &'a LayoutNode, ids: &[TileId]) -> Option<&'a LayoutNode> {
            if members(node) == ids {
                return Some(node);
            }
            match node {
                LayoutNode::Tile(_) => None,
                LayoutNode::Tabs { children, .. } | LayoutNode::Split { children, .. } => {
                    children.iter().find_map(|child| find(child, ids))
                }
            }
        }
        let remaining = current.clone().without(self.tile);
        let mut branch = LayoutNode::Tile(self.tile);
        for ancestor in &self.ancestors {
            branch = match ancestor {
                Ancestor::Tabs { order, active } => LayoutNode::Tabs {
                    active: *active,
                    children: order.iter().copied().map(LayoutNode::Tile).collect(),
                },
                Ancestor::Split {
                    dir,
                    shares,
                    index,
                    siblings,
                } => {
                    let mut moved = Some(branch);
                    let children = siblings
                        .iter()
                        .enumerate()
                        .map(|(i, ids)| {
                            if i == *index {
                                moved.take()
                            } else {
                                find(remaining.as_ref()?, ids).cloned()
                            }
                        })
                        .collect::<Option<Vec<_>>>()?;
                    LayoutNode::Split {
                        dir: *dir,
                        shares: shares.clone(),
                        children,
                    }
                }
            };
        }
        super::history::retain_move_navigation(&mut branch, current, self.tile);
        Some(branch)
    }
}
