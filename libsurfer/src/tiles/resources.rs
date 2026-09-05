//! Typed dependencies shared by validation, copying, collection and history.
//!
//! Storage and the saved format retain concrete resource types. Add a resource
//! variant and its storage operations here when another resource type exists.
use std::collections::{BTreeMap, BTreeSet};

use super::{
    ItemListId,
    runtime::{IdentityError, WorkspaceRuntime},
};
use crate::item_list::ItemList;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceId {
    ItemList(ItemListId),
}

/// Opaque owners may refer to any resource, including ones not understood by
/// this reader. Their presence prevents collection, but never excuses a known
/// owner's missing references.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Dependencies {
    known: BTreeSet<ResourceId>,
    opaque: bool,
}

impl Dependencies {
    pub fn known(ids: impl IntoIterator<Item = ResourceId>) -> Self {
        Self {
            known: ids.into_iter().collect(),
            opaque: false,
        }
    }
    pub fn opaque() -> Self {
        Self {
            opaque: true,
            ..Self::default()
        }
    }
    pub fn references(&self) -> impl Iterator<Item = ResourceId> + '_ {
        self.known.iter().copied()
    }
    pub fn is_opaque(&self) -> bool {
        self.opaque
    }
    pub fn retains(&self, id: ResourceId) -> bool {
        self.opaque || self.known.contains(&id)
    }
    pub fn union(owners: impl IntoIterator<Item = Self>) -> Self {
        let mut result = Self::default();
        for owner in owners {
            result.known.extend(owner.known);
            result.opaque |= owner.opaque;
        }
        result
    }
    pub(crate) fn validate(&self, present: &BTreeSet<ResourceId>) -> Result<(), ResourceError> {
        if let Some(id) = self.references().find(|id| !present.contains(id)) {
            return Err(ResourceError::Missing(id));
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ResourceError {
    #[error("missing resource {0:?}")]
    Missing(ResourceId),
    #[error("opaque resource dependencies cannot be copied independently")]
    Opaque,
    #[error("incomplete resource remapping")]
    Remap,
    #[error(transparent)]
    Identity(#[from] IdentityError),
}

/// A total mapping for the resources of one independently copied owner.
/// Keeping it typed prevents accidental interchange of different resource IDs.
pub struct ResourceRemap(BTreeMap<ResourceId, ResourceId>);
impl ResourceRemap {
    pub fn item_list(&self, id: ItemListId) -> Result<ItemListId, ResourceError> {
        match self.0.get(&ResourceId::ItemList(id)) {
            Some(ResourceId::ItemList(copy)) => Ok(*copy),
            None => Err(ResourceError::Remap),
        }
    }
}

pub(crate) trait ResourceOwner: Clone {
    fn dependencies(&self) -> Dependencies;
    fn remap_resources(&mut self, mapping: &ResourceRemap) -> Result<(), ResourceError>;
}

pub(crate) fn ids(lists: &BTreeMap<ItemListId, ItemList>) -> BTreeSet<ResourceId> {
    lists.keys().copied().map(ResourceId::ItemList).collect()
}

/// Stage all resources and rewrite only a detached owner. Each distinct
/// dependency is copied once, so repeated references remain shared in the copy.
pub(crate) fn independent_copy<T: ResourceOwner>(
    owner: &T,
    lists: &BTreeMap<ItemListId, ItemList>,
    runtime: &mut WorkspaceRuntime,
) -> Result<(T, BTreeMap<ItemListId, ItemList>), ResourceError> {
    let dependencies = owner.dependencies();
    if dependencies.is_opaque() {
        return Err(ResourceError::Opaque);
    }
    dependencies.validate(&ids(lists))?;
    let mut copied = BTreeMap::new();
    let mut mapping = ResourceRemap(BTreeMap::new());
    for resource in dependencies.references() {
        match resource {
            ResourceId::ItemList(id) => {
                let new_id = runtime.allocate_list()?;
                // A caller must install loaded IDs in the runtime first. Do not
                // silently overwrite resources even if that contract was broken.
                if lists.contains_key(&new_id) {
                    return Err(ResourceError::Remap);
                }
                copied.insert(new_id, lists[&id].copy_content());
                mapping.0.insert(resource, ResourceId::ItemList(new_id));
            }
        }
    }
    let mut result = owner.clone();
    result.remap_resources(&mapping)?;
    if result.dependencies() != Dependencies::known(mapping.0.values().copied()) {
        return Err(ResourceError::Remap);
    }
    Ok((result, copied))
}

pub(crate) fn retain(lists: &mut BTreeMap<ItemListId, ItemList>, dependencies: &Dependencies) {
    lists.retain(|id, _| dependencies.retains(ResourceId::ItemList(*id)));
}

#[cfg(test)]
mod tests {
    use super::*;

    // A test owner exercises several references through the same production
    // copy algorithm without inventing a new user-facing tile kind.
    #[derive(Clone, Debug, PartialEq)]
    struct Owner(Vec<ItemListId>);
    impl ResourceOwner for Owner {
        fn dependencies(&self) -> Dependencies {
            Dependencies::known(self.0.iter().copied().map(ResourceId::ItemList))
        }
        fn remap_resources(&mut self, mapping: &ResourceRemap) -> Result<(), ResourceError> {
            self.0 = self
                .0
                .iter()
                .map(|id| mapping.item_list(*id))
                .collect::<Result<_, _>>()?;
            Ok(())
        }
    }

    #[test]
    fn multiple_dependencies_copy_once_and_preserve_internal_sharing() {
        let a = ItemListId(1);
        let b = ItemListId(2);
        let owner = Owner(vec![a, b, a]);
        let mut lists = BTreeMap::from([(a, ItemList::default()), (b, ItemList::default())]);
        lists.get_mut(&a).unwrap().display_item_ref_counter = 17;
        lists.get_mut(&b).unwrap().display_item_ref_counter = 29;
        lists[&a].layout_cache.borrow_mut().signature = Some(99);
        let mut runtime = WorkspaceRuntime::default();
        runtime.install_workspace([], [a, b]).unwrap();
        let linked = owner.clone();
        assert_eq!(linked.dependencies(), owner.dependencies());
        let (copy, copied) = independent_copy(&owner, &lists, &mut runtime).unwrap();
        assert_eq!(copied.len(), 2);
        assert_eq!(copy.0[0], copy.0[2]);
        assert_ne!(copy.0[0], copy.0[1]);
        assert!(copy.0.iter().all(|id| !lists.contains_key(id)));
        assert_eq!(copied[&copy.0[0]].display_item_ref_counter, 17);
        assert_eq!(copied[&copy.0[1]].display_item_ref_counter, 29);
        assert!(copied[&copy.0[0]].layout_cache.borrow().signature.is_none());
        assert_eq!(lists[&a].layout_cache.borrow().signature, Some(99));
        assert_eq!(owner, Owner(vec![a, b, a]));

        lists.extend(copied);
        // Closing one linked owner keeps both resources; closing the last
        // collects both, while all independently copied resources survive.
        retain(
            &mut lists,
            &Dependencies::union([linked.dependencies(), copy.dependencies()]),
        );
        assert_eq!(lists.len(), 4);
        retain(&mut lists, &copy.dependencies());
        assert_eq!(lists.len(), 2);
        copy.dependencies().validate(&ids(&lists)).unwrap();
        retain(&mut lists, &Dependencies::default());
        assert!(lists.is_empty());
    }

    #[test]
    fn missing_dependencies_and_incomplete_remaps_fail_without_changing_source() {
        let mut runtime = WorkspaceRuntime::default();
        let owner = Owner(vec![ItemListId(1), ItemListId(2)]);
        let lists = BTreeMap::from([(ItemListId(1), ItemList::default())]);
        assert!(matches!(
            independent_copy(&owner, &lists, &mut runtime),
            Err(ResourceError::Missing(ResourceId::ItemList(ItemListId(2))))
        ));
        assert_eq!(runtime.allocate_list().unwrap(), ItemListId(1));
        assert_eq!(lists.len(), 1);
        assert_eq!(owner.0, [ItemListId(1), ItemListId(2)]);

        #[derive(Clone)]
        struct BrokenOwner(Owner);
        impl ResourceOwner for BrokenOwner {
            fn dependencies(&self) -> Dependencies {
                self.0.dependencies()
            }
            fn remap_resources(&mut self, _: &ResourceRemap) -> Result<(), ResourceError> {
                Ok(())
            }
        }
        let bad = BrokenOwner(Owner(vec![ItemListId(1)]));
        assert!(matches!(
            independent_copy(&bad, &lists, &mut runtime),
            Err(ResourceError::Remap)
        ));
        assert_eq!(bad.0.0, [ItemListId(1)]);
    }

    #[test]
    fn opaque_owners_retain_every_resource_without_masking_missing_known_dependencies() {
        let mut lists = BTreeMap::from([
            (ItemListId(1), ItemList::default()),
            (ItemListId(2), ItemList::default()),
        ]);
        let dependencies = Dependencies::union([
            Dependencies::opaque(),
            Owner(vec![ItemListId(3)]).dependencies(),
        ]);
        retain(&mut lists, &dependencies);
        assert_eq!(lists.len(), 2);
        assert!(dependencies.validate(&ids(&lists)).is_err());
        retain(&mut lists, &Owner(vec![ItemListId(1)]).dependencies());
        assert_eq!(lists.keys().copied().collect::<Vec<_>>(), [ItemListId(1)]);
    }
}
