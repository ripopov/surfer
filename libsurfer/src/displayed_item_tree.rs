use itertools::Itertools;
use serde::{Deserialize, Serialize};

use crate::displayed_item::DisplayedItemRef;
use crate::MoveDir;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Node {
    pub item: DisplayedItemRef,
    /// Nesting level of the node.
    pub level: u8,
    /// Whether a subtree of this node (if it exists) is shown
    pub unfolded: bool,
    pub selected: bool,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum MoveResult {
    InvalidIndex,
    InvalidLevel,
    CircularMove,
}

/// N-th visible item, becomes invalid after any add/remove/move/fold/unfold operation
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct VisibleItemIndex(pub usize);

/// N-th item, may currently be invisible, becomes invalid after any add/remove/move operation
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Serialize, Deserialize)]
pub struct ItemIndex(pub usize);

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum TargetPosition {
    Before(ItemIndex),
    After(ItemIndex),
    Into(ItemIndex),
    End(u8),
}

#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct VisibleItemIterator<'a> {
    items: std::slice::Iter<'a, Node>,
    last: Option<&'a Node>,
}

impl<'a> Iterator for VisibleItemIterator<'a> {
    type Item = &'a Node;
    fn next(&mut self) -> Option<Self::Item> {
        let (skip, skip_to) = match self.last {
            Some(x) => (!x.unfolded, x.level),
            None => (false, 0),
        };

        if skip {
            for x in self.items.by_ref() {
                if x.level <= skip_to {
                    self.last = Some(x);
                    return self.last;
                }
            }
            self.last = None;
            return None;
        }

        self.last = self.items.next();
        self.last
    }
}

#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct VisibleItemIteratorMut<'a> {
    items: &'a mut DisplayedItemTree,
    next_idx: usize,
    skip_to: Option<u8>,
}

impl<'a> Iterator for VisibleItemIteratorMut<'a> {
    type Item = &'a mut Node;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(skip_to) = self.skip_to {
            while self.next_idx < self.items.len() {
                if self.items.items[self.next_idx].level <= skip_to {
                    break;
                }
                self.next_idx += 1;
            }
            self.skip_to = None;
        }
        if self.next_idx < self.items.items.len() {
            let idx = self.next_idx;

            self.next_idx += 1;
            if !self.items.items[idx].unfolded {
                self.skip_to = Some(self.items.items[idx].level)
            }
            let ptr = self.items.items.as_mut_ptr();
            // access is safe since we
            // - do access within bounds
            // - know that we won't generate two equal references (next call, next item)
            // - know that no second iterator or other access can happen while the references/iterator exist
            Some(unsafe { &mut *ptr.add(idx) })
        } else {
            None
        }
    }
}

#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct VisibleItemIteratorExtraInfo<'a> {
    tree: &'a DisplayedItemTree,
    /// Index of the next element to return, not guaranteed to be in-bounds
    next_idx: usize,
}

impl<'a> Iterator for VisibleItemIteratorExtraInfo<'a> {
    type Item = (&'a Node, ItemIndex, bool);

    fn next(&mut self) -> Option<Self::Item> {
        let (skip, skip_to) = match self.next_idx {
            0 => (false, 0),
            _ => match self.tree.items.get(self.next_idx - 1) {
                None => return None,
                Some(item) => (!item.unfolded, item.level),
            },
        };

        let idx = if skip {
            let mut candidate_idx = self.next_idx;
            loop {
                if candidate_idx >= self.tree.items.len()
                    || self.tree.items[candidate_idx].level <= skip_to
                {
                    break;
                }
                candidate_idx += 1;
            }
            candidate_idx
        } else {
            self.next_idx
        };

        self.next_idx = idx + 1;
        let result = self.tree.items.get(idx);
        // we can unwrap in map bec. if the next element exists then this element exists as well...
        let has_child = self
            .tree
            .items
            .get(self.next_idx)
            .map(|next| next.level > result.map(|this| this.level).unwrap())
            .unwrap_or(false);

        result.map(|x| (x, ItemIndex(idx), has_child))
    }
}

// TODO ancestor path iterator

/// Tree if items to be displayed
///
/// Items are stored in a flat list, with the level property indicating the nesting level
/// of the item. The items are stored in-order.
/// For documentation on the properties of a node, see the [Node] struct.
///
/// Note also infos on the [VisibleItemIndex] and [ItemIndex] types w.r.t. stability of these
/// index types.
///
/// Invariants:
/// - The nesting levels of the tree must monotonically increase (but may jump levels going down)
#[derive(Default, Serialize, Deserialize, Debug, Clone)]
pub struct DisplayedItemTree {
    items: Vec<Node>,
}

impl DisplayedItemTree {
    pub fn new() -> Self {
        DisplayedItemTree { items: vec![] }
    }

    pub fn push_item(&mut self, item: DisplayedItemRef) {
        self.items.push(Node {
            item,
            level: 0,
            unfolded: true,
            selected: false,
        });
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Node> + use<'_> {
        self.items.iter()
    }

    /// Iterate through all visible items
    pub fn iter_visible(&self) -> VisibleItemIterator {
        VisibleItemIterator {
            items: self.items.iter(),
            last: None,
        }
    }

    pub fn iter_visible_mut(&mut self) -> VisibleItemIteratorMut {
        VisibleItemIteratorMut {
            items: self,
            next_idx: 0,
            skip_to: None,
        }
    }

    pub fn iter_visible_extra(&self) -> VisibleItemIteratorExtraInfo {
        VisibleItemIteratorExtraInfo {
            tree: self,
            next_idx: 0,
        }
    }

    pub fn iter_selected(&self) -> impl Iterator<Item = &Node> + use<'_> {
        self.iter().filter(|i| i.selected)
    }

    pub fn iter_visible_selected(&self) -> impl Iterator<Item = &Node> + use<'_> {
        self.iter_visible().filter(|i| i.selected)
    }

    /// Iterate through items, skipping invisible items, return index of n-th visible item
    pub fn get_visible(&self, index: VisibleItemIndex) -> Option<&Node> {
        self.iter_visible().nth(index.0)
    }

    pub fn get(&self, index: ItemIndex) -> Option<&Node> {
        self.items.get(index.0)
    }

    pub fn get_mut(&mut self, index: ItemIndex) -> Option<&mut Node> {
        self.items.get_mut(index.0)
    }

    pub fn to_displayed(&self, index: VisibleItemIndex) -> Option<ItemIndex> {
        // TODO make this more efficient
        let item = self.get_visible(index)?;
        self.items
            .iter()
            .position(|x| std::ptr::eq(x, item))
            .map(ItemIndex)
        //let base_ptr = vec.as_ptr();
        //let element_ptr = element as *const T;
        //if element_ptr >= base_ptr && element_ptr < unsafe { base_ptr.add(vec.len()) } {
        //    Some(unsafe { element_ptr.offset_from(base_ptr) as usize })
        //} else {
        //    None
        //}
    }

    /// insert item after offset visible items (either in root or in unfolded parent)
    pub fn insert_item(
        &mut self,
        item: DisplayedItemRef,
        position: TargetPosition,
    ) -> Result<ItemIndex, MoveResult> {
        self.is_valid_position(position)?;

        let target_idx = match position {
            TargetPosition::Before(ref_idx) => ref_idx.0,
            TargetPosition::After(ref_idx) => ref_idx.0 + 1,
            TargetPosition::Into(ref_idx) => ref_idx.0 + 1,
            TargetPosition::End(_) => self.items.len(),
        };
        let target_level = match position {
            TargetPosition::Before(ref_idx) => self.items[ref_idx.0].level,
            TargetPosition::After(ref_idx) => self.items[ref_idx.0].level,
            TargetPosition::Into(ref_idx) => self.items[ref_idx.0].level + 1,
            TargetPosition::End(level) => level,
        };

        self.items.insert(
            target_idx,
            Node {
                item,
                level: target_level,
                unfolded: true,
                selected: false,
            },
        );

        Ok(ItemIndex(target_idx))
    }

    /// Return the index past the end of the subtree started by `idx`
    fn subtree_end(&self, start_idx: usize) -> usize {
        let level = self.items[start_idx].level;
        self.items
            .iter()
            .skip(start_idx + 1)
            .enumerate()
            .filter_map(|(idx, x)| (x.level <= level).then_some(idx + start_idx + 1))
            .next()
            .unwrap_or(self.items.len())
    }

    pub fn remove_recursive(&mut self, ItemIndex(item): ItemIndex) -> Vec<DisplayedItemRef> {
        let end = self.subtree_end(item);
        self.items.drain(item..end).map(|x| x.item).collect_vec()
    }

    pub fn remove_dissolve(&mut self, ItemIndex(item): ItemIndex) -> DisplayedItemRef {
        let end = self.subtree_end(item);
        self.items[item + 1..end]
            .iter_mut()
            .for_each(|x| x.level -= 1);
        self.items.remove(item).item
    }

    pub fn extract_recursive_if<F>(&mut self, f: F) -> Vec<DisplayedItemRef>
    where
        F: Fn(&Node) -> bool,
    {
        let mut removed = vec![];

        let mut idx = 0;
        while idx < self.items.len() {
            if f(self.items.get(idx).unwrap()) {
                let end = self.subtree_end(idx);
                removed.extend(self.items.drain(idx..end).map(|x| x.item));
            } else {
                idx += 1;
            }
        }

        removed
    }

    pub fn move_item(
        &mut self,
        ItemIndex(item): ItemIndex,
        direction: MoveDir,
    ) -> Result<(), MoveResult> {
        if item >= self.items.len() {
            return Err(MoveResult::InvalidIndex);
        }

        match direction {
            MoveDir::Down => {
                let end = self.subtree_end(item);
                if end == self.items.len() {
                    return Ok(());
                }
                self.move_items(vec![ItemIndex(item)], TargetPosition::After(ItemIndex(end)))
            }
            MoveDir::Up => {
                // find previous sibling or parent
                let level = self.items[item].level;
                let prev = self.items[..item]
                    .iter()
                    .rposition(|x| x.level <= level)
                    .unwrap_or(0);
                self.move_items(
                    vec![ItemIndex(item)],
                    TargetPosition::After(ItemIndex(prev)),
                )
            }
        }
    }

    /// Move multiple items to a specified location
    ///
    /// Indices may be unsorted and contain duplicates, but must be valid.
    ///
    /// Deals with any combination of items to move, except the error
    /// cases below. Rules that are followed:
    /// - When calculating the "next" node, visibility is taken into account
    /// - The relative order of element should be the same, before and after the move
    /// - If the root of a subtree is moved, the whole subtree is moved
    /// - If a node inside a subtree is moved, then it's moved out of that subtree
    /// - If both the root and a node of a subtree are moved, the node is moved out
    ///   of the subtree and ends up after the root node
    /// - When moving an element to "After" the last element of a subtree, it goes into the subtree
    /// - When moving an element to "Before" the first element of a subtree (not the root) it also
    ///   goes into the subtree
    /// - When moving to "After" a subtree root, we move into the subtree
    ///
    /// Possible errors:
    /// - trying to move an element into itself -> errors out
    /// - move nests too deep for our level count -> clips items to level 255
    /// - invalid indices -> errors out
    pub fn move_items(
        &mut self,
        mut indices: Vec<ItemIndex>,
        target: TargetPosition,
    ) -> Result<(), MoveResult> {
        indices.sort();
        let indices = indices.into_iter().dedup().collect_vec();

        self.is_valid_position(target)?;
        if let Some(idx) = indices.last() {
            if idx.0 >= self.items.len() {
                return Err(MoveResult::InvalidIndex);
            }
        }

        let candidate_swivel = match target {
            TargetPosition::Before(item_index) => item_index.0,
            TargetPosition::After(item_index) if self.has_visible_children(item_index) => {
                item_index.0 + 1
            }
            TargetPosition::After(item_index) => self.subtree_end(item_index.0),
            TargetPosition::Into(item_index) => item_index.0 + 1,
            TargetPosition::End(_) => self.items.len(),
        };
        let target_level = match target {
            TargetPosition::Before(item_index) => self.items[item_index.0].level,
            TargetPosition::After(item_index) if self.has_visible_children(item_index) => {
                self.items[item_index.0].level + 1
            }
            TargetPosition::After(item_index) => self.items[item_index.0].level,
            TargetPosition::Into(item_index) => self.items[item_index.0].level + 1,
            TargetPosition::End(level) => level,
        };

        let (pre_indices, post_indices) = indices.split_at(
            indices
                .iter()
                .position(|x| x.0 > candidate_swivel)
                .unwrap_or(indices.len()),
        );

        // if the target element itself is being moved we exclude it from the items to be moved
        // to reduce special handling below. Insert points for pre/post indices must be adapted accordingly
        // the target node will be part of the pre_indices, remove from that list
        let (pre_indices, mut pre_index_insert, post_index_insert) = match (target, pre_indices) {
            (TargetPosition::After(item_index), [.., last])
                if *last == item_index && !self.has_visible_children(item_index) =>
            {
                (
                    &pre_indices[..pre_indices.len() - 1],
                    candidate_swivel - 1,
                    candidate_swivel,
                )
            }
            (TargetPosition::Before(item_index), [.., last]) if *last == item_index => (
                &pre_indices[..pre_indices.len() - 1],
                candidate_swivel,
                candidate_swivel + 1,
            ),
            // no case for TargetPosition::Into needed, because that would be a circular move
            _ => (pre_indices, candidate_swivel, candidate_swivel),
        };

        if self.path_to_root_would_intersect(pre_indices, candidate_swivel, target_level) {
            return Err(MoveResult::CircularMove);
        }

        // move all items that are before the target index
        // - do so in reverse -> if a subtree and a node from inside this subtree is moved we move
        //   that subnode correctly and don't need to adjust indices
        // - we need to adjust the insertion point since we go in reverse
        //
        // Note: due to intersect check above we can't have a subtree that crosses over the target_idx
        for &ItemIndex(from_start) in pre_indices.iter().rev() {
            let from_end = self.subtree_end(from_start);

            dbg!(from_start, from_end);
            shift_subtree_to_level(&mut self.items[from_start..from_end], target_level);

            let cnt = from_end - from_start;
            self.items[from_start..pre_index_insert].rotate_left(cnt);
            pre_index_insert -= cnt;
        }

        // move all items that are after the target index
        // - again go in reverse to correctly handle nested items to be moved
        // - all indices need to be adjusted for the number of nodes that we moved in front
        //   of them
        let mut idx_offset = 0;
        for &ItemIndex(orig_start) in post_indices.iter().rev() {
            let from_start = orig_start + idx_offset;
            let from_end = self.subtree_end(from_start);

            dbg!(from_start, from_end);
            shift_subtree_to_level(&mut self.items[from_start..from_end], target_level);

            let cnt = from_end - from_start;
            self.items[post_index_insert..from_end].rotate_right(cnt);
            idx_offset += cnt;
        }

        Ok(())
    }

    fn is_valid_position(&self, position: TargetPosition) -> Result<(), MoveResult> {
        match position {
            TargetPosition::Before(ItemIndex(idx))
            | TargetPosition::After(ItemIndex(idx))
            | TargetPosition::Into(ItemIndex(idx))
                if idx >= self.items.len() =>
            {
                Err(MoveResult::InvalidIndex)
            }
            TargetPosition::End(level) => match self.items.last() {
                Some(last) if (level as i16) - (last.level as i16) > 1 => {
                    Err(MoveResult::InvalidLevel)
                }
                None if level != 0 => Err(MoveResult::InvalidLevel),
                _ => Ok(()),
            },
            _ => Ok(()),
        }
    }

    fn has_visible_children(&self, ItemIndex(index): ItemIndex) -> bool {
        self.items[index].unfolded
            && self
                .items
                .get(index + 1)
                .map(|x| x.level > self.items[index].level)
                .unwrap_or(false)
    }

    pub fn is_visible(&self, ItemIndex(index): ItemIndex) -> bool {
        self.path_to_root(index, self.items[index].level)
            .iter()
            .all(|x| self.items[*x].unfolded)
    }

    /// Check whether the path from the imaginary node `idx/level` to the root would
    /// intersect with any of the indices in the list.
    ///
    /// Precondition: `indices` must be sorted in ascending order
    fn path_to_root_would_intersect(&self, indices: &[ItemIndex], idx: usize, level: u8) -> bool {
        let mut would_be_parents = self.path_to_root(idx, level);
        would_be_parents.reverse();

        let mut i = 0;
        let mut j = 0;
        while i < indices.len() && j < would_be_parents.len() {
            match indices[i].0.cmp(&would_be_parents[j]) {
                std::cmp::Ordering::Equal => return true,
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
            }
        }

        false
    }

    /// Indices of parents, assuming that `index` and `level` are a valid node
    fn path_to_root(&self, index: usize, mut level: u8) -> Vec<usize> {
        let mut result = vec![];
        for (idx, x) in self.items[..index].iter().enumerate().rev() {
            if x.level < level {
                result.push(idx);
                level = x.level;
            }
            if level == 0 {
                break;
            }
        }
        result
    }

    pub fn xfold(&mut self, ItemIndex(item): ItemIndex, unfolded: bool) {
        self.items[item].unfolded = unfolded;
        if !unfolded {
            self.xselect_subtree(ItemIndex(item), false);
        }
    }

    pub fn xfold_all(&mut self, unfolded: bool) {
        for x in &mut self.items {
            x.unfolded = unfolded;
            if !unfolded && x.level > 0 {
                x.selected = false;
            }
        }
    }

    pub fn xfold_subtree(&mut self, ItemIndex(item): ItemIndex, unfolded: bool) {
        let end = self.subtree_end(item);
        for x in &mut self.items[item..end] {
            x.unfolded = unfolded;
            if !unfolded && x.level > 0 {
                x.selected = false;
            }
        }
    }

    // TODO should these functions fail if the item is not visible?
    pub fn xselect(&mut self, ItemIndex(item): ItemIndex, selected: bool) {
        self.items[item].selected = selected;
    }

    pub fn xselect_all(&mut self, selected: bool) {
        for x in &mut self.items {
            x.selected = selected;
        }
    }

    pub fn xselect_subtree(&mut self, ItemIndex(item): ItemIndex, selected: bool) {
        let end = self.subtree_end(item);
        for x in &mut self.items[item..end] {
            x.selected = selected;
        }
    }

    /// Change selection for visible items, in inclusive range
    pub fn xselect_visible_range(
        &mut self,
        VisibleItemIndex(from): VisibleItemIndex,
        VisibleItemIndex(to): VisibleItemIndex,
        selected: bool,
    ) {
        let (from, to) = if from < to {
            (from, to + 1)
        } else {
            (to, from + 1)
        };
        for node in self.iter_visible_mut().skip(from).take(to - from) {
            node.selected = selected
        }
    }

    pub fn for_each_mut<F>(&mut self, mut f: F)
    where
        F: FnMut(&mut Node),
    {
        for x in &mut self.items {
            f(x);
        }
    }

    pub fn for_each_subtree_mut<F>(&mut self, ItemIndex(item): ItemIndex, mut f: F)
    where
        F: FnMut(&mut Node),
    {
        let end = self.subtree_end(item);
        for x in &mut self.items[item..end] {
            f(x);
        }
    }

    pub fn retain_recursive(&mut self, mut f: impl FnMut(&Node) -> bool) {
        self.items.retain(|x| f(x));
    }

    // TODO retain
    // TODO move item up/down
}

fn shift_subtree_to_level(nodes: &mut [Node], target_level: u8) {
    let Some(from_level) = nodes.get(0).map(|node| node.level) else {
        return;
    };
    let level_corr = (target_level as i16) - (from_level as i16);
    for elem in nodes.iter_mut() {
        elem.level = TryInto::<u8>::try_into(elem.level as i16 + level_corr).unwrap_or(255);
    }
}

#[cfg(test)]
mod tests {
    use itertools::Itertools;

    use super::*;

    fn build_tree(nodes: &[(usize, u8, bool, bool)]) -> DisplayedItemTree {
        let mut tree = DisplayedItemTree::new();
        for &(item, level, unfolded, selected) in nodes {
            tree.items.push(Node {
                item: DisplayedItemRef(item),
                level,
                unfolded,
                selected,
            })
        }
        tree
    }

    /// common tes tree
    /// ```text
    /// 0: 0
    /// 1: 1
    /// 2: 2       < folded
    /// 3:   20
    /// 4:     200
    /// 5: 3
    /// 6:   30
    /// 7:   31
    /// 8: 4
    /// 9: 5
    /// ```
    fn test_tree() -> DisplayedItemTree {
        build_tree(&[
            (0, 0, true, false),
            (1, 0, false, false),
            (2, 0, false, false),
            (20, 1, true, false),
            (200, 2, true, false),
            (3, 0, true, false),
            (30, 1, true, false),
            (31, 1, true, false),
            (4, 0, true, false),
            (5, 0, true, false),
        ])
    }

    #[test]
    fn test_iter_visible() {
        let tree = test_tree();
        assert_eq!(
            tree.iter_visible().map(|x| x.item.0).collect_vec(),
            vec![0, 1, 2, 3, 30, 31, 4, 5]
        );
    }

    #[test]
    fn test_iter_visible_extra() {
        let tree = test_tree();
        assert_eq!(
            tree.iter_visible_extra()
                .map(|(x, idx, child)| (x.item.0, idx.0, child))
                .collect_vec(),
            vec![
                (0, 0, false),
                (1, 1, false),
                (2, 2, true),
                (3, 5, true),
                (30, 6, false),
                (31, 7, false),
                (4, 8, false),
                (5, 9, false),
            ]
        )
    }

    #[test]
    fn test_insert_item_before_first() {
        let mut tree = test_tree();
        tree.insert_item(DisplayedItemRef(0xff), TargetPosition::Before(ItemIndex(0)))
            .expect("insert_item must succeed");
        assert_eq!(
            tree.items.iter().map(|x| x.item.0).collect_vec(),
            vec![0xff, 0, 1, 2, 20, 200, 3, 30, 31, 4, 5]
        );
        assert_eq!(tree.items[0].level, 0);
        assert_eq!(tree.items[0].selected, false);
        assert_eq!(tree.items[0].unfolded, true);
    }

    #[test]
    /// Test that inserting an element "after" the last element of a subtree
    /// does insert into the subtree, after said element
    fn test_insert_item_after_into_subtree() {
        let mut tree = test_tree();
        tree.insert_item(DisplayedItemRef(0xff), TargetPosition::After(ItemIndex(7)))
            .expect("insert_item must succeed");
        assert_eq!(
            tree.items.iter().map(|x| x.item.0).collect_vec(),
            vec![0, 1, 2, 20, 200, 3, 30, 31, 0xff, 4, 5]
        );
        assert_eq!(tree.items[7].level, 1);
        assert_eq!(tree.items[7].selected, false);
        assert_eq!(tree.items[7].unfolded, true);
    }

    #[test]
    fn test_insert_item_into() {
        let mut tree = test_tree();
        tree.insert_item(DisplayedItemRef(0xff), TargetPosition::Into(ItemIndex(6)))
            .expect("insert_item must succeed");
        assert_eq!(
            tree.items.iter().map(|x| x.item.0).collect_vec(),
            vec![0, 1, 2, 20, 200, 3, 30, 0xff, 31, 4, 5]
        );
        assert_eq!(tree.items[7].level, 2);
        assert_eq!(tree.items[7].selected, false);
        assert_eq!(tree.items[7].unfolded, true);
    }

    #[test]
    fn test_insert_item_end() {
        let mut tree = test_tree();
        tree.insert_item(DisplayedItemRef(0xff), TargetPosition::End(1))
            .expect("insert_item must succeed");
        assert_eq!(
            tree.items.iter().map(|x| x.item.0).collect_vec(),
            vec![0, 1, 2, 20, 200, 3, 30, 31, 4, 5, 0xff]
        );
        assert_eq!(tree.items[10].level, 1);
    }

    #[test]
    fn test_remove_recursive_no_children() {
        let mut tree = test_tree();
        let removed = tree.remove_recursive(ItemIndex(0));
        assert_eq!(removed, vec![DisplayedItemRef(0)]);
        assert_eq!(
            tree.items.iter().map(|x| x.item.0).collect_vec(),
            vec![1, 2, 20, 200, 3, 30, 31, 4, 5]
        );
    }

    #[test]
    fn test_remove_recursive_with_children() {
        let mut tree = test_tree();
        let removed = tree.remove_recursive(ItemIndex(2));
        assert_eq!(
            removed,
            vec![
                DisplayedItemRef(2),
                DisplayedItemRef(20),
                DisplayedItemRef(200)
            ]
        );
        assert_eq!(
            tree.items.iter().map(|x| x.item.0).collect_vec(),
            vec![0, 1, 3, 30, 31, 4, 5]
        );
    }

    #[test]
    fn test_remove_dissolve_with_children() {
        let mut tree = test_tree();
        let removed = tree.remove_dissolve(ItemIndex(5));
        assert_eq!(removed, DisplayedItemRef(3));
        assert_eq!(
            tree.items.iter().map(|x| x.item.0).collect_vec(),
            vec![0, 1, 2, 20, 200, 30, 31, 4, 5]
        );
        assert_eq!(tree.items[5].level, 0);
        assert_eq!(tree.items[6].level, 0);
    }

    #[test]
    fn test_move_items_single_to_start() {
        let mut tree = test_tree();
        tree.move_items(vec![ItemIndex(8)], TargetPosition::Before(ItemIndex(0)))
            .expect("move_items must succeed");
        assert_eq!(
            tree.items.iter().map(|x| x.item.0).collect_vec(),
            vec![4, 0, 1, 2, 20, 200, 3, 30, 31, 5]
        );
        assert_eq!(tree.items[0].level, 0);
    }

    #[test]
    fn test_move_items_single_to_end() {
        let mut tree = test_tree();
        tree.move_items(vec![ItemIndex(4)], TargetPosition::After(ItemIndex(9)))
            .expect("move_items must succeed");
        assert_eq!(
            tree.items.iter().map(|x| x.item.0).collect_vec(),
            vec![0, 1, 2, 20, 3, 30, 31, 4, 5, 200]
        );
        assert_eq!(tree.items[9].level, 0);
    }

    #[test]
    fn test_move_items_multiple_connected() {
        let mut tree = test_tree();
        tree.move_items(
            vec![ItemIndex(8), ItemIndex(9)],
            TargetPosition::After(ItemIndex(0)),
        )
        .expect("move_items must succeed");
        assert_eq!(
            tree.items.iter().map(|x| x.item.0).collect_vec(),
            vec![0, 4, 5, 1, 2, 20, 200, 3, 30, 31]
        );
        assert_eq!(tree.items[1].level, 0);
        assert_eq!(tree.items[2].level, 0);
    }

    #[test]
    fn test_move_items_multiple_different_levels() {
        let mut tree = test_tree();
        tree.move_items(
            vec![ItemIndex(7), ItemIndex(8)],
            TargetPosition::After(ItemIndex(0)),
        )
        .expect("move_items must succeed");
        assert_eq!(
            tree.items.iter().map(|x| x.item.0).collect_vec(),
            vec![0, 31, 4, 1, 2, 20, 200, 3, 30, 5]
        );
        assert_eq!(tree.items[1].level, 0);
        assert_eq!(tree.items[2].level, 0);
    }

    #[test]
    fn test_move_items_multiple_unconnected() {
        let mut tree = test_tree();
        tree.move_items(
            vec![ItemIndex(1), ItemIndex(8)],
            TargetPosition::Before(ItemIndex(5)),
        )
        .expect("move_items must succeed");
        assert_eq!(
            tree.items.iter().map(|x| x.item.0).collect_vec(),
            vec![0, 2, 20, 200, 1, 4, 3, 30, 31, 5]
        );
        assert_eq!(tree.items[4].level, 0);
        assert_eq!(tree.items[5].level, 0);
    }

    #[test]
    fn test_move_items_multiple_into() {
        let mut tree = test_tree();
        tree.move_items(
            vec![ItemIndex(1), ItemIndex(8)],
            TargetPosition::Before(ItemIndex(4)),
        )
        .expect("move_items must succeed");
        assert_eq!(
            tree.items.iter().map(|x| x.item.0).collect_vec(),
            vec![0, 2, 20, 1, 4, 200, 3, 30, 31, 5]
        );
        assert_eq!(tree.items[4].level, 2);
        assert_eq!(tree.items[5].level, 2);
    }

    #[test]
    fn test_move_items_subtree_into() {
        let mut tree = test_tree();
        tree.move_items(vec![ItemIndex(3)], TargetPosition::Into(ItemIndex(6)))
            .expect("move_items must succeed");
        assert_eq!(
            tree.items.iter().map(|x| x.item.0).collect_vec(),
            vec![0, 1, 2, 3, 30, 20, 200, 31, 4, 5]
        );
        assert_eq!(tree.items[5].level, 2);
        assert_eq!(tree.items[6].level, 3);
    }

    #[test]
    /// Moving "after" a node that has visible children moves into the subtree
    fn test_move_items_after_subtree_root() {
        // usual tree with all items visible
        let mut tree = test_tree();
        tree.items[2].unfolded = true;

        tree.move_items(vec![ItemIndex(9)], TargetPosition::After(ItemIndex(2)))
            .expect("move_items must succeed");
        assert_eq!(
            tree.items.iter().map(|x| x.item.0).collect_vec(),
            vec![0, 1, 2, 5, 20, 200, 3, 30, 31, 4]
        );
        assert_eq!(tree.items[3].level, 1);
        assert_eq!(tree.items[4].level, 1);
    }

    #[test]
    /// Moving "after" a node only moves into the node if the children are visible.
    /// Here they are not, so treat the same as if those children were not there.
    fn test_move_items_after_subtree_root_folded() {
        let mut tree = test_tree();

        tree.move_items(vec![ItemIndex(9)], TargetPosition::After(ItemIndex(2)))
            .expect("move_items must succeed");
        assert_eq!(
            tree.items.iter().map(|x| x.item.0).collect_vec(),
            vec![0, 1, 2, 20, 200, 5, 3, 30, 31, 4]
        );
        assert_eq!(tree.items[5].level, 0);
    }

    #[test]
    /// Moving "after" a node w/o children moves nodes to the same level,
    /// so it's fine and natural that the node itself can be included in the selection
    fn test_move_items_after_self() {
        let mut tree = test_tree();
        tree.move_items(
            vec![ItemIndex(0), ItemIndex(4), ItemIndex(9)],
            TargetPosition::After(ItemIndex(4)),
        )
        .expect("move_items must succeed");
        assert_eq!(
            tree.items.iter().map(|x| x.item.0).collect_vec(),
            vec![1, 2, 20, 0, 200, 5, 3, 30, 31, 4]
        );
        assert_eq!(tree.items[3].level, 2);
        assert_eq!(tree.items[4].level, 2);
        assert_eq!(tree.items[5].level, 2);
    }

    #[test]
    /// Moving "after" a node w/o children moves nodes to the same level,
    /// so it's fine and natural that the node itself can be included in the selection
    fn test_move_items_after_self_last() {
        let mut tree = test_tree();
        tree.move_items(
            vec![ItemIndex(0), ItemIndex(4), ItemIndex(9)],
            TargetPosition::After(ItemIndex(9)),
        )
        .expect("move_items must succeed");
        assert_eq!(
            tree.items.iter().map(|x| x.item.0).collect_vec(),
            vec![1, 2, 20, 3, 30, 31, 4, 0, 200, 5]
        );
        assert_eq!(tree.items[7].level, 0);
        assert_eq!(tree.items[8].level, 0);
        assert_eq!(tree.items[9].level, 0);
    }

    #[test]
    fn test_move_items_before_self() {
        let mut tree = test_tree();
        tree.move_items(
            vec![ItemIndex(0), ItemIndex(3), ItemIndex(9)],
            TargetPosition::Before(ItemIndex(3)),
        )
        .expect("move_items must succeed");
        assert_eq!(
            tree.items.iter().map(|x| x.item.0).collect_vec(),
            vec![1, 2, 0, 20, 5, 200, 3, 30, 31, 4]
        );
        assert_eq!(tree.items[2].level, 1);
        assert_eq!(tree.items[3].level, 1);
        assert_eq!(tree.items[4].level, 1);
    }

    #[test]
    fn test_move_items_empty_list() {
        let mut tree = test_tree();
        tree.move_items(vec![], TargetPosition::After(ItemIndex(4)))
            .expect("move_items must succeed");
        assert_eq!(
            tree.items.iter().map(|x| x.item.0).collect_vec(),
            vec![0, 1, 2, 20, 200, 3, 30, 31, 4, 5]
        );
        assert_eq!(tree.items, test_tree().items);
    }

    #[test]
    /// Moving "after" a node that has children moves into the subtree,
    /// so we must error out if the node itself should be moved
    fn test_move_items_reject_after_self_into_subtree() {
        let mut tree = test_tree();
        let result = tree.move_items(
            vec![ItemIndex(0), ItemIndex(3), ItemIndex(9)],
            TargetPosition::After(ItemIndex(3)),
        );
        assert_eq!(result, Err(MoveResult::CircularMove));
        assert_eq!(tree.items, test_tree().items);
    }

    #[test]
    fn test_move_items_reject_into_subtree() {
        let mut tree = test_tree();
        let result = tree.move_items(vec![ItemIndex(3)], TargetPosition::Into(ItemIndex(3)));
        assert_eq!(result, Err(MoveResult::CircularMove));
        assert_eq!(tree.items, test_tree().items);
    }

    #[test]
    /// Test that the subtree check before moving is done correctly.
    /// The valid subtree element 100 also being moved prevents simpler
    /// checks (like checking only the first pre-index) from passing incorrectly.
    fn test_move_items_reject_into_subtree_distant() {
        let reference = build_tree(&[
            (1, 0, true, false),
            (10, 1, true, false),
            (100, 2, true, false),
            (11, 3, true, false),
        ]);
        let mut tree = reference.clone();
        let result = tree.move_items(
            vec![ItemIndex(1), ItemIndex(2)],
            TargetPosition::Into(ItemIndex(3)),
        );
        assert_eq!(result, Err(MoveResult::CircularMove));
        assert_eq!(tree.items, reference.items);
    }

    #[test]
    fn test_move_items_reject_into_self() {
        let mut tree = test_tree();
        let result = tree.move_items(vec![ItemIndex(0)], TargetPosition::Into(ItemIndex(0)));
        assert_eq!(result, Err(MoveResult::CircularMove));
        assert_eq!(tree.items, test_tree().items);
    }
}
