//! Each memory tile owns its preferences, navigation and disposable row cache.
use crate::{
    Message,
    memory_viewer::{MemoryViewerCache, MemoryViewerNavigation, MemoryViewerSettings},
    tiles::{TileId, kind::TileMessage},
};
use std::cell::RefCell;

#[derive(Debug, serde::Deserialize)]
pub enum MemoryMessage {
    Settings(Box<MemoryViewerSettings>),
}

#[derive(Default)]
struct MemoryRuntime {
    navigation: MemoryViewerNavigation,
    cache: Option<MemoryViewerCache>,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryTile {
    pub(crate) settings: MemoryViewerSettings,
    #[serde(skip)]
    runtime: RefCell<MemoryRuntime>,
}
impl Clone for MemoryTile {
    fn clone(&self) -> Self {
        Self {
            settings: self.settings.clone(),
            runtime: Default::default(),
        }
    }
}
impl MemoryTile {
    pub(crate) fn attach(
        &mut self,
        document: &mut crate::wave_data::WaveData,
    ) -> Option<crate::wellen::LoadSignalsCmd> {
        self.reset_runtime();
        let scope = self.settings.scope.as_mut()?;
        scope.id = Default::default();
        let container = document.inner.as_waves_mut()?;
        if !container.scope_exists(scope) {
            return None;
        }
        let variables = container.variables_in_scope(scope);
        container
            .load_variables(variables.iter())
            .map_err(|error| tracing::warn!("Memory array load failed: {error}"))
            .ok()
            .flatten()
    }

    pub(crate) fn reset_runtime(&mut self) {
        *self.runtime.get_mut() = Default::default();
    }
    pub(crate) fn update(&mut self, message: MemoryMessage) -> bool {
        let MemoryMessage::Settings(settings) = message;
        if self.settings == *settings {
            return false;
        }
        let changed_source = self.settings.scope != settings.scope;
        self.settings = *settings;
        if changed_source {
            self.reset_runtime();
        } else {
            self.runtime.get_mut().cache = None;
        }
        true
    }
    pub(crate) fn ui(
        &self,
        ui: &mut egui::Ui,
        id: TileId,
        document: Option<&crate::wave_data::WaveData>,
        translators: &crate::translation::TranslatorList,
        theme: &crate::config::SurferTheme,
        messages: &mut Vec<Message>,
    ) {
        let mut settings = self.settings.clone();
        let mut runtime = self.runtime.borrow_mut();
        let MemoryRuntime { navigation, cache } = &mut *runtime;
        crate::memory_viewer::draw_memory_viewer(
            ui,
            &mut settings,
            navigation,
            cache,
            document,
            translators,
            theme,
        );
        if settings != self.settings {
            messages.push(Message::ToTile(
                id,
                TileMessage::Memory(MemoryMessage::Settings(Box::new(settings))),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiles::{
        commands::{SplitMode, WorkspaceCommand},
        kind::TileKind,
        layout::Direction,
    };

    fn open(state: &mut crate::SystemState, array: &str) -> TileId {
        state
            .update(Message::OpenMemoryViewer {
                scope: crate::wave_container::ScopeRef {
                    strs: vec!["dut".into(), array.into()],
                    id: Default::default(),
                },
                name: Some(array.into()),
                placement: None,
            })
            .unwrap();
        state.user.workspace.layout.focused().unwrap()
    }
    fn memory(state: &crate::SystemState, id: TileId) -> &MemoryTile {
        let TileKind::Memory(tile) = &state.user.workspace.tiles[&id].kind else {
            panic!()
        };
        tile
    }

    #[test]
    fn memory_instances_save_independent_sources_and_initialization_is_one_undo_entry() {
        let mut state = crate::SystemState::new_default_config().unwrap();
        let first = open(&mut state, "instructions");
        assert_eq!(state.undo_stack.len(), 1);
        state.update(Message::Undo(1)).unwrap();
        assert!(state.user.workspace.tiles.is_empty());
        state.update(Message::Redo(1)).unwrap();
        assert_eq!(
            memory(&state, first).settings.scope.as_ref().unwrap().strs,
            ["dut", "instructions"]
        );
        let second = open(&mut state, "data");
        assert_ne!(first, second);
        assert!(state.user.workspace.item_lists.is_empty());
        let mut edited = memory(&state, first).settings.clone();
        edited.name = Some("Instructions edited".into());
        state
            .update(Message::ToTile(
                first,
                TileMessage::Memory(MemoryMessage::Settings(Box::new(edited))),
            ))
            .unwrap();
        assert_eq!(state.user.workspace.layout.focused(), Some(second));
        state.update(Message::Undo(1)).unwrap();
        assert_eq!(
            memory(&state, first).settings.name.as_deref(),
            Some("instructions")
        );
        state.update(Message::Redo(1)).unwrap();
        assert_eq!(
            memory(&state, second).settings.name.as_deref(),
            Some("data")
        );
        let saved = state.encode_state().unwrap();
        let restored: crate::state::UserState = crate::tiles::serde::decode(&saved).unwrap();
        assert_eq!(restored.workspace.tiles.len(), 2);
        let TileKind::Memory(tile) = &restored.workspace.tiles[&first].kind else {
            panic!()
        };
        assert_eq!(tile.settings.name.as_deref(), Some("Instructions edited"));
        assert_eq!(
            tile.settings.scope.as_ref().unwrap().id,
            crate::wave_container::ScopeId::None
        );
    }

    #[test]
    fn cloned_and_reopened_memory_tiles_clear_caches_and_stale_commands_do_not_redirect() {
        let mut state = crate::SystemState::new_default_config().unwrap();
        let first = open(&mut state, "data");
        let tile = memory(&state, first);
        tile.runtime.borrow_mut().cache = Some(MemoryViewerCache {
            key: crate::memory_viewer::MemoryViewerCacheKey {
                scope: tile.settings.scope.clone().unwrap(),
                cursor: 0u32.into(),
                value_format: "Hexadecimal".into(),
                filter_mode: crate::memory_viewer::ChangeModes::AllValues,
                highlight_mode: crate::memory_viewer::ChangeModes::AllValues,
                document_generation: 0,
                filter_range: None,
                highlight_range: None,
            },
            rows: std::sync::Arc::new(Vec::new()),
        });
        state
            .update(Message::Workspace(WorkspaceCommand::SplitTile {
                tile: first,
                dir: Direction::Right,
                mode: SplitMode::Clone,
            }))
            .unwrap();
        let copy = state.user.workspace.layout.focused().unwrap();
        assert_ne!(first, copy);
        assert!(memory(&state, copy).runtime.borrow().cache.is_none());
        assert!(memory(&state, first).runtime.borrow().cache.is_some());
        let settings = memory(&state, first).settings.clone();
        state
            .update(Message::Workspace(WorkspaceCommand::CloseTile(first)))
            .unwrap();
        assert!(
            state
                .update(Message::ToTile(
                    first,
                    TileMessage::Memory(MemoryMessage::Settings(Box::new(settings)))
                ))
                .is_none()
        );
        state.update(Message::Undo(1)).unwrap();
        assert!(memory(&state, first).runtime.borrow().cache.is_none());
        assert_eq!(state.user.workspace.layout.focused(), Some(copy));
    }
}
