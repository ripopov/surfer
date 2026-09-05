//! Session identity is never serialized or restored by undo.

use super::{ItemListId, TileId};

#[derive(Debug, Copy, Clone, PartialEq, Eq, thiserror::Error)]
#[error("workspace identity counter exhausted or invalid ID")]
pub struct IdentityError;

#[derive(Debug, Clone)]
struct Counter(u64);

impl Default for Counter {
    fn default() -> Self {
        Self(1)
    }
}

impl Counter {
    fn next(&mut self) -> Result<u64, IdentityError> {
        let next = self.0.checked_add(1).ok_or(IdentityError)?;
        let result = self.0;
        self.0 = next;
        Ok(result)
    }

    fn observe(&mut self, id: u64) -> Result<(), IdentityError> {
        if id == 0 {
            return Err(IdentityError);
        }
        self.0 = self.0.max(id.checked_add(1).ok_or(IdentityError)?);
        Ok(())
    }
}

/// Every request also stores its complete input key in the owning tile.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct RequestToken {
    workspace_epoch: u64,
    document_generation: u64,
    pub tile: TileId,
    request: u64,
}

#[derive(Debug, Default)]
pub struct WorkspaceRuntime {
    tiles: Counter,
    lists: Counter,
    requests: Counter,
    workspace_epoch: u64,
    document_generation: u64,
    workspace_initialized: bool,
}

impl WorkspaceRuntime {
    pub(crate) fn workspace_initialized(&self) -> bool {
        self.workspace_initialized
    }
    pub(crate) fn mark_workspace_initialized(&mut self) {
        self.workspace_initialized = true;
    }

    pub fn allocate_tile(&mut self) -> Result<TileId, IdentityError> {
        self.tiles.next().map(TileId)
    }

    pub fn allocate_list(&mut self) -> Result<ItemListId, IdentityError> {
        self.lists.next().map(ItemListId)
    }

    /// Advance only after all loaded IDs have been checked, without lowering counters.
    pub fn install_workspace(
        &mut self,
        tiles: impl IntoIterator<Item = TileId>,
        lists: impl IntoIterator<Item = ItemListId>,
    ) -> Result<(), IdentityError> {
        let mut next_tiles = self.tiles.clone();
        let mut next_lists = self.lists.clone();
        for id in tiles {
            next_tiles.observe(id.0)?;
        }
        for id in lists {
            next_lists.observe(id.0)?;
        }
        let epoch = self.workspace_epoch.checked_add(1).ok_or(IdentityError)?;
        self.tiles = next_tiles;
        self.lists = next_lists;
        self.workspace_epoch = epoch;
        self.workspace_initialized = true;
        Ok(())
    }

    pub fn document_changed(&mut self) -> Result<(), IdentityError> {
        self.document_generation = self
            .document_generation
            .checked_add(1)
            .ok_or(IdentityError)?;
        Ok(())
    }

    pub fn request(&mut self, tile: TileId) -> Result<RequestToken, IdentityError> {
        Ok(RequestToken {
            workspace_epoch: self.workspace_epoch,
            document_generation: self.document_generation,
            tile,
            request: self.requests.next()?,
        })
    }

    /// Caller additionally checks that the owner exists and its input key matches.
    pub fn accepts(&self, completion: RequestToken, pending: Option<RequestToken>) -> bool {
        completion.workspace_epoch == self.workspace_epoch
            && completion.document_generation == self.document_generation
            && pending == Some(completion)
    }

    pub fn egui_id(&self, tile: TileId, salt: impl std::hash::Hash + std::fmt::Debug) -> egui::Id {
        egui::Id::new(("workspace", self.workspace_epoch, tile, salt))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_never_reuses_an_allocated_identity() {
        let mut runtime = WorkspaceRuntime::default();
        assert_eq!(runtime.allocate_tile().unwrap(), TileId(1));
        runtime
            .install_workspace([TileId(40)], [ItemListId(9)])
            .unwrap();
        assert_eq!(runtime.allocate_tile().unwrap(), TileId(41));
        runtime
            .install_workspace([TileId(1)], [ItemListId(1)])
            .unwrap();
        assert_eq!(runtime.allocate_tile().unwrap(), TileId(42));
        assert_eq!(runtime.allocate_list().unwrap(), ItemListId(10));
    }

    #[test]
    fn invalid_load_does_not_advance_any_counter() {
        let mut runtime = WorkspaceRuntime::default();
        let token = runtime.request(TileId(1)).unwrap();
        assert!(
            runtime
                .install_workspace([TileId(500)], [ItemListId(u64::MAX)])
                .is_err()
        );
        assert!(runtime.accepts(token, Some(token)));
        assert_eq!(runtime.allocate_tile().unwrap(), TileId(1));
        assert!(runtime.install_workspace([TileId(0)], []).is_err());
    }

    #[test]
    fn completions_require_current_workspace_document_and_request() {
        let mut runtime = WorkspaceRuntime::default();
        let tile = runtime.allocate_tile().unwrap();
        let first = runtime.request(tile).unwrap();
        let second = runtime.request(tile).unwrap();
        assert!(!runtime.accepts(first, Some(second)));
        assert!(!runtime.accepts(first, None)); // closed or restored owner
        assert!(runtime.accepts(second, Some(second)));
        runtime.document_changed().unwrap();
        assert!(!runtime.accepts(second, Some(second)));
        let third = runtime.request(tile).unwrap();
        let old_id = runtime.egui_id(tile, "names");
        runtime.install_workspace([tile], []).unwrap();
        assert!(!runtime.accepts(third, Some(third)));
        assert_ne!(old_id, runtime.egui_id(tile, "names"));
    }

    #[test]
    fn exhaustion_is_an_error_without_wrapping() {
        let mut runtime = WorkspaceRuntime::default();
        runtime
            .install_workspace([TileId(u64::MAX - 1)], [])
            .unwrap();
        assert!(runtime.allocate_tile().is_err());
        assert!(runtime.allocate_tile().is_err());
        runtime.document_generation = u64::MAX;
        assert!(runtime.document_changed().is_err());
    }
}
