//! Content shared by linked waveform views. Navigation belongs to the view.

use std::{
    cell::{Ref, RefCell},
    collections::HashMap,
};

use crate::{
    annotation::Annotation,
    annotation_list::AnnotationGroup,
    displayed_item::{DisplayedItem, DisplayedItemRef},
    displayed_item_tree::{DisplayedItemTree, ItemIndex, TargetPosition, VisibleItemIndex},
    graphics::{Graphic, GraphicId},
    item_drawing_info::ItemDrawingInfo,
    variable_name_type::VariableNameType,
};

/// Selection commands capture stable row identities at the input boundary.
#[derive(Debug, serde::Deserialize)]
pub enum ItemSelection {
    Set {
        item: DisplayedItemRef,
        selected: bool,
    },
    Toggle(DisplayedItemRef),
    Range {
        from: DisplayedItemRef,
        to: DisplayedItemRef,
        selected: bool,
    },
    AllVisible(bool),
    Clear,
}

type FlattenedRowsCache =
    ahash::AHashMap<DisplayedItemRef, (u64, std::sync::Arc<Vec<crate::view::VariableFieldRow>>)>;

#[derive(Default)]
pub struct ItemLayoutCache {
    pub infos: Vec<ItemDrawingInfo>,
    pub signature: Option<u64>,
    pub total_height: f32,
}

pub struct ItemList {
    pub items_tree: DisplayedItemTree,
    pub displayed_items: HashMap<DisplayedItemRef, DisplayedItem>,
    pub display_item_ref_counter: usize,
    pub default_variable_name_type: VariableNameType,
    pub annotations: Vec<Annotation>,
    pub annotation_groups: Vec<AnnotationGroup>,
    pub annotation_counter: i32,
    pub graphics: HashMap<GraphicId, Graphic>,
    // Disposable content-space row layout, shared by views of the same list.
    pub layout_cache: RefCell<ItemLayoutCache>,
    pub(crate) flattened_rows_cache: RefCell<FlattenedRowsCache>,
}

#[derive(Debug, thiserror::Error)]
pub enum ItemEditError {
    #[error("item identity counter exhausted")]
    IdentityExhausted,
    #[error("missing item {0:?}")]
    Missing(DisplayedItemRef),
    #[error("invalid item placement")]
    Placement,
}

fn valid_nesting<'a>(
    tree: &DisplayedItemTree,
    item: impl Fn(DisplayedItemRef) -> Option<&'a DisplayedItem>,
) -> bool {
    let mut previous: Option<(u8, bool)> = None;
    for node in tree.iter() {
        let Some(item) = item(node.item_ref) else {
            return false;
        };
        match previous {
            None if node.level != 0 => return false,
            Some((level, group))
                if node.level > level && (!group || level.checked_add(1) != Some(node.level)) =>
            {
                return false;
            }
            _ => {}
        }
        previous = Some((node.level, matches!(item, DisplayedItem::Group(_))));
    }
    true
}

impl ItemList {
    /// Validate captured row identities before changing shared selection.
    /// Returns whether any selection bit changed; navigation is untouched.
    pub fn apply_selection(&mut self, command: ItemSelection) -> Result<bool, ItemEditError> {
        let mut changed = false;
        let mut set = |node: &mut crate::displayed_item_tree::Node, selected: bool| {
            changed |= node.selected != selected;
            node.selected = selected;
        };
        match command {
            ItemSelection::Set { item, selected } => {
                let index = self
                    .items_tree
                    .iter()
                    .position(|node| node.item_ref == item)
                    .ok_or(ItemEditError::Missing(item))?;
                set(self.items_tree.get_mut(ItemIndex(index)).unwrap(), selected);
            }
            ItemSelection::Toggle(item) => {
                let index = self
                    .items_tree
                    .iter()
                    .position(|node| node.item_ref == item)
                    .ok_or(ItemEditError::Missing(item))?;
                let node = self.items_tree.get_mut(ItemIndex(index)).unwrap();
                set(node, !node.selected);
            }
            ItemSelection::Range { from, to, selected } => {
                let from = self
                    .get_displayed_item_index(&from)
                    .ok_or(ItemEditError::Missing(from))?
                    .0;
                let to = self
                    .get_displayed_item_index(&to)
                    .ok_or(ItemEditError::Missing(to))?
                    .0;
                for node in self
                    .items_tree
                    .iter_visible_mut()
                    .skip(from.min(to))
                    .take(from.abs_diff(to) + 1)
                {
                    set(node, selected);
                }
            }
            ItemSelection::AllVisible(selected) => {
                for node in self.items_tree.iter_visible_mut() {
                    set(node, selected);
                }
            }
            ItemSelection::Clear => {
                for index in 0..self.items_tree.len() {
                    set(self.items_tree.get_mut(ItemIndex(index)).unwrap(), false);
                }
            }
        }
        Ok(changed)
    }

    /// Return an insert position based on item
    ///
    /// If an item is passed, and it is
    /// - an unfolded group, insert index is to the first element of the group
    /// - a folded group, insert index is to before the next sibling (if exists)
    /// - otherwise insert index is past it on the same level
    #[must_use]
    pub fn insert_position(&self, vidx: Option<VisibleItemIndex>) -> Option<TargetPosition> {
        let vidx = vidx?;
        let item_index = self.items_tree.to_displayed(vidx)?;
        let node = self.items_tree.get(item_index)?;
        let item = self.displayed_items.get(&node.item_ref)?;

        // TODO add get_next_sibling to tree?
        let (before, level) = match item {
            DisplayedItem::Group(..) if node.unfolded => (item_index.0 + 1, node.level + 1),
            DisplayedItem::Group(..) => {
                let next_idx = self.items_tree.to_displayed(VisibleItemIndex(vidx.0 + 1));
                match next_idx {
                    Some(idx) => (idx.0, node.level),
                    None => (self.items_tree.len(), node.level),
                }
            }
            _ => (item_index.0 + 1, node.level),
        };
        Some(TargetPosition {
            before: ItemIndex(before),
            level,
        })
    }

    pub fn insert_item(
        &mut self,
        item: DisplayedItem,
        position: TargetPosition,
    ) -> Result<DisplayedItemRef, ItemEditError> {
        let next = self
            .display_item_ref_counter
            .checked_add(1)
            .filter(|id| *id < usize::MAX)
            .ok_or(ItemEditError::IdentityExhausted)?;
        let id = DisplayedItemRef(next);
        if self.displayed_items.contains_key(&id) {
            return Err(ItemEditError::IdentityExhausted);
        }
        let previous = position
            .before
            .0
            .checked_sub(1)
            .and_then(|index| self.items_tree.get(ItemIndex(index)));
        if previous.is_none() && position.level != 0 {
            return Err(ItemEditError::Placement);
        }
        if let Some(previous) = previous
            && position.level > previous.level
            && (previous.level.checked_add(1) != Some(position.level)
                || !matches!(
                    self.displayed_items.get(&previous.item_ref),
                    Some(DisplayedItem::Group(_))
                ))
        {
            return Err(ItemEditError::Placement);
        }
        if let Some(next) = self.items_tree.get(position.before)
            && next.level > position.level
            && (position.level.checked_add(1) != Some(next.level)
                || !matches!(item, DisplayedItem::Group(_)))
        {
            return Err(ItemEditError::Placement);
        }
        // Tree insertion validates its index before touching the node vector.
        self.items_tree
            .insert_item(id, position)
            .map_err(|_| ItemEditError::Placement)?;
        self.displayed_items.insert(id, item);
        self.display_item_ref_counter = next;
        *self.layout_cache.get_mut() = ItemLayoutCache::default();
        Ok(id)
    }

    pub fn move_items(
        &mut self,
        ids: &[DisplayedItemRef],
        position: TargetPosition,
    ) -> Result<bool, ItemEditError> {
        if ids.is_empty() {
            return Ok(false);
        }
        let indices = ids
            .iter()
            .map(|id| {
                self.items_tree
                    .iter()
                    .position(|node| node.item_ref == *id)
                    .map(ItemIndex)
                    .ok_or(ItemEditError::Missing(*id))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut tree = self.items_tree.clone();
        tree.move_items(indices, position)
            .map_err(|_| ItemEditError::Placement)?;
        if !valid_nesting(&tree, |id| self.displayed_items.get(&id)) {
            return Err(ItemEditError::Placement);
        }
        if tree.iter().eq(self.items_tree.iter()) {
            return Ok(false);
        }
        self.items_tree = tree;
        *self.layout_cache.get_mut() = ItemLayoutCache::default();
        Ok(true)
    }
}

#[cfg(test)]
mod edit_tests {
    use super::*;
    use crate::displayed_item::DisplayedDivider;

    fn divider() -> DisplayedItem {
        DisplayedItem::Divider(DisplayedDivider {
            name: None,
            color: None,
            background_color: None,
        })
    }

    fn saved(list: &ItemList) -> String {
        ron::to_string(&crate::tiles::serde::ItemListFile::from(list)).unwrap()
    }

    #[test]
    fn selection_tracks_rows_across_reordering_and_rejects_invalid_ranges_atomically() {
        let mut list = ItemList::default();
        let first = list
            .insert_item(divider(), list.end_insert_position())
            .unwrap();
        let second = list
            .insert_item(divider(), list.end_insert_position())
            .unwrap();
        let third = list
            .insert_item(divider(), list.end_insert_position())
            .unwrap();
        let captured = ItemSelection::Set {
            item: first,
            selected: true,
        };
        list.move_items(&[first], list.end_insert_position())
            .unwrap();
        assert!(list.apply_selection(captured).unwrap());
        assert_eq!(
            list.items_tree
                .iter_visible_selected()
                .map(|node| node.item_ref)
                .collect::<Vec<_>>(),
            [first]
        );
        assert!(
            !list
                .apply_selection(ItemSelection::Set {
                    item: first,
                    selected: true
                })
                .unwrap()
        );
        let before = saved(&list);
        assert!(
            list.apply_selection(ItemSelection::Range {
                from: second,
                to: DisplayedItemRef(999),
                selected: true
            })
            .is_err()
        );
        assert_eq!(saved(&list), before);
        assert!(
            list.apply_selection(ItemSelection::Range {
                from: first,
                to: third,
                selected: true
            })
            .unwrap()
        );
        assert_eq!(
            list.items_tree
                .iter_visible_selected()
                .map(|node| node.item_ref)
                .collect::<Vec<_>>(),
            [third, first]
        );
        assert!(list.apply_selection(ItemSelection::Clear).unwrap());
        assert!(!list.apply_selection(ItemSelection::Clear).unwrap());
    }

    #[test]
    fn failed_edits_do_not_change_content_caches_or_consume_item_ids() {
        let mut list = ItemList::default();
        let first = list
            .insert_item(divider(), list.end_insert_position())
            .unwrap();
        let second = list
            .insert_item(divider(), list.end_insert_position())
            .unwrap();
        list.layout_cache.get_mut().signature = Some(77);
        let before = saved(&list);
        for position in [
            TargetPosition {
                before: ItemIndex(99),
                level: 0,
            },
            TargetPosition {
                before: ItemIndex(1),
                level: 1,
            },
        ] {
            assert!(list.insert_item(divider(), position).is_err());
            assert!(list.move_items(&[second], position).is_err());
            assert_eq!(saved(&list), before);
            assert_eq!(list.layout_cache.borrow().signature, Some(77));
        }
        assert!(
            list.move_items(&[first, DisplayedItemRef(999)], list.end_insert_position())
                .is_err()
        );
        assert_eq!(saved(&list), before);
        assert_eq!(
            list.insert_item(divider(), list.end_insert_position())
                .unwrap(),
            DisplayedItemRef(3)
        );
    }

    #[test]
    fn reordering_preserves_local_ids_and_rejects_making_a_non_group_parent() {
        let mut list = ItemList::default();
        let first = list
            .insert_item(divider(), list.end_insert_position())
            .unwrap();
        let second = list
            .insert_item(divider(), list.end_insert_position())
            .unwrap();
        assert!(
            list.move_items(&[first], list.end_insert_position())
                .unwrap()
        );
        assert_eq!(
            list.items_tree
                .iter()
                .map(|node| node.item_ref)
                .collect::<Vec<_>>(),
            vec![second, first]
        );
        assert!(
            !list
                .move_items(&[first], list.end_insert_position())
                .unwrap()
        );
        assert!(
            list.move_items(
                &[first],
                TargetPosition {
                    before: ItemIndex(1),
                    level: 1
                }
            )
            .is_err()
        );
        crate::tiles::serde::ItemListFile::from(&list)
            .validate()
            .unwrap();
    }
}

impl Default for ItemList {
    fn default() -> Self {
        Self {
            items_tree: DisplayedItemTree::default(),
            displayed_items: HashMap::new(),
            display_item_ref_counter: 0,
            default_variable_name_type: VariableNameType::Local,
            annotations: vec![],
            annotation_groups: vec![],
            annotation_counter: 0,
            graphics: HashMap::new(),
            layout_cache: RefCell::default(),
            flattened_rows_cache: RefCell::default(),
        }
    }
}

impl ItemList {
    /// Delete rows and their attached annotations. Document marker times are not list content.
    pub fn remove_items(&mut self, ids: &[DisplayedItemRef]) -> bool {
        use crate::annotation::Annotatable;
        use std::collections::BTreeSet;
        let requested = ids.iter().copied().collect::<BTreeSet<_>>();
        let indices = self
            .items_tree
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                requested
                    .contains(&node.item_ref)
                    .then_some(ItemIndex(index))
            })
            .collect::<Vec<_>>();
        if indices.is_empty() {
            return false;
        }
        let mut removed = BTreeSet::new();
        for index in indices.into_iter().rev() {
            removed.extend(self.items_tree.remove_recursive(index));
        }
        self.displayed_items.retain(|id, _| !removed.contains(id));
        self.annotations
            .retain(|annotation| !removed.iter().any(|id| annotation.is_attached(id)));
        let annotations = self
            .annotations
            .iter()
            .map(Annotatable::get_id)
            .collect::<std::collections::HashSet<_>>();
        for group in &mut self.annotation_groups {
            group.annotations.retain(|id| annotations.contains(id));
        }
        *self.layout_cache.get_mut() = ItemLayoutCache::default();
        self.flattened_rows_cache
            .get_mut()
            .retain(|id, _| !removed.contains(id));
        true
    }

    /// Independent splits copy content but never carry over cached geometry.
    pub fn copy_content(&self) -> Self {
        Self {
            items_tree: self.items_tree.clone(),
            displayed_items: self.displayed_items.clone(),
            display_item_ref_counter: self.display_item_ref_counter,
            default_variable_name_type: self.default_variable_name_type,
            annotations: self.annotations.clone(),
            annotation_groups: self.annotation_groups.clone(),
            annotation_counter: self.annotation_counter,
            graphics: self.graphics.clone(),
            ..Default::default()
        }
    }
}

impl ItemList {
    pub(crate) fn drawing_bottom(&self) -> Option<f32> {
        self.layout_cache
            .borrow()
            .infos
            .last()
            .map(ItemDrawingInfo::bottom)
    }

    pub(crate) fn drawing_bottom_at(&self, offset: f32) -> Option<f32> {
        self.layout_cache
            .borrow()
            .infos
            .last()
            .map(|info| info.bottom_at(offset))
    }

    /// Return drawing infos overlapping the visible range.
    ///
    /// `drawing_infos` is sorted by both `top()` and `bottom()`, so use binary search to determine the start and end.
    pub(crate) fn visible_drawing_infos(
        &self,
        visible_top: f32,
        visible_bottom: f32,
    ) -> Ref<'_, [ItemDrawingInfo]> {
        Ref::map(self.layout_cache.borrow(), |cache| {
            let start = cache
                .infos
                .partition_point(|info| info.bottom() < visible_top);
            let end =
                cache.infos[start..].partition_point(|info| info.top() <= visible_bottom) + start;
            &cache.infos[start..end]
        })
    }

    /// Find the row in content-space coordinates, without any tile scroll offset.
    pub(crate) fn drawing_info_at_y(&self, y: f32) -> Option<Ref<'_, ItemDrawingInfo>> {
        Ref::filter_map(self.layout_cache.borrow(), |cache| {
            if cache.infos.last()?.bottom() <= y {
                return None;
            }
            let idx = cache.infos.partition_point(|info| info.top() <= y);
            idx.checked_sub(1).map(|idx| &cache.infos[idx])
        })
        .ok()
    }

    /// Find the item at a given (canvas-local) y-location.
    #[must_use]
    pub(crate) fn get_item_at_y(&self, y: f32) -> Option<VisibleItemIndex> {
        self.drawing_info_at_y(y).map(|info| info.vidx())
    }

    /// Returns the displayed item reference located at the given canvas y-coordinate.
    #[must_use]
    pub(crate) fn item_ref_at_canvas_y(&self, y: f32) -> Option<DisplayedItemRef> {
        let vidx = self.get_item_at_y(y)?;
        let node = self.items_tree.get_visible(vidx)?;
        Some(node.item_ref)
    }

    /// Returns the displayed item reference and drawing info of the row at the given canvas
    /// y-coordinate, in a single lookup.
    #[must_use]
    pub(crate) fn item_and_drawing_info_at_y(
        &self,
        y: f32,
    ) -> Option<(DisplayedItemRef, Ref<'_, ItemDrawingInfo>)> {
        let info = self.drawing_info_at_y(y)?;
        let node = self.items_tree.get_visible(info.vidx())?;
        Some((node.item_ref, info))
    }

    /// Return insert position as last item
    #[must_use]
    pub fn end_insert_position(&self) -> TargetPosition {
        TargetPosition {
            before: ItemIndex(self.items_tree.len()),
            level: 0,
        }
    }

    #[inline]
    #[must_use]
    pub fn any_displayed(&self) -> bool {
        !self.displayed_items.is_empty()
    }

    pub fn next_displayed_item_ref(&mut self) -> DisplayedItemRef {
        self.display_item_ref_counter += 1;
        self.display_item_ref_counter.into()
    }

    #[must_use]
    pub fn get_displayed_item_index(
        &self,
        item_ref: &DisplayedItemRef,
    ) -> Option<VisibleItemIndex> {
        // TODO check where this is called since it could now fail...
        self.items_tree
            .iter_visible()
            .enumerate()
            .find_map(|(vidx, node)| {
                if node.item_ref == *item_ref {
                    Some(VisibleItemIndex(vidx))
                } else {
                    None
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::displayed_item::DisplayedDivider;
    use crate::displayed_item_tree::{ItemIndex, TargetPosition};

    #[test]
    fn content_copy_is_independent_and_starts_without_runtime_geometry() {
        let mut original = ItemList::default();
        let id = DisplayedItemRef(1);
        original
            .items_tree
            .insert_item(
                id,
                TargetPosition {
                    before: ItemIndex(0),
                    level: 0,
                },
            )
            .unwrap();
        original.displayed_items.insert(
            id,
            DisplayedItem::Divider(DisplayedDivider {
                name: Some("Shared row".into()),
                color: None,
                background_color: None,
            }),
        );
        original.display_item_ref_counter = 1;
        original.layout_cache.get_mut().signature = Some(42);
        original.layout_cache.get_mut().total_height = 100.0;
        let mut copied = original.copy_content();
        assert_eq!(copied.items_tree.len(), 1);
        assert_eq!(copied.display_item_ref_counter, 1);
        assert!(copied.layout_cache.borrow().infos.is_empty());
        assert!(copied.layout_cache.borrow().signature.is_none());
        assert_eq!(copied.layout_cache.borrow().total_height, 0.0);
        let DisplayedItem::Divider(row) = copied.displayed_items.get_mut(&id).unwrap() else {
            panic!()
        };
        row.name = Some("Independent row".into());
        let DisplayedItem::Divider(row) = &original.displayed_items[&id] else {
            panic!()
        };
        assert_eq!(row.name.as_deref(), Some("Shared row"));
    }
}
