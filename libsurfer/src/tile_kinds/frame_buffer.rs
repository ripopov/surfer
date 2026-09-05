//! A framebuffer owns its selected source, decoding settings and runtime caches.
use crate::{
    Message,
    frame_buffer::{
        FrameBufferArrayCache, FrameBufferColorMode, FrameBufferContent, FrameBufferPixelCache,
        FrameBufferSettings,
    },
    tiles::{TileId, kind::TileMessage},
};
use std::cell::RefCell;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "FrameBufferStateFile")]
pub struct FrameBufferState {
    pub settings: FrameBufferSettings,
    pub content: Option<FrameBufferContent>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameBufferStateFile {
    settings: FrameBufferSettings,
    content: Option<FrameBufferContent>,
}
#[derive(Debug, thiserror::Error)]
pub enum FrameBufferError {
    #[error("framebuffer width must be positive and channel widths must be at most eight bits")]
    Settings,
    #[error("invalid framebuffer array range")]
    Range,
}
impl FrameBufferState {
    pub(crate) fn validate(&self) -> Result<(), FrameBufferError> {
        let color = &self.settings.color_settings;
        if self.settings.pixels_per_row == 0
            || !(1..=8).contains(&color.grayscale_bits)
            || [
                color.r_bits,
                color.g_bits,
                color.b_bits,
                color.y_bits,
                color.cb_bits,
                color.cr_bits,
            ]
            .into_iter()
            .any(|bits| bits > 8)
        {
            return Err(FrameBufferError::Settings);
        }
        if let Some(FrameBufferContent::Array { levels, .. }) = &self.content
            && levels.iter().any(|level| {
                level.min_index > level.first_index
                    || level.first_index > level.last_index
                    || level.last_index > level.max_index
            })
        {
            return Err(FrameBufferError::Range);
        }
        Ok(())
    }
}
impl TryFrom<FrameBufferStateFile> for FrameBufferState {
    type Error = FrameBufferError;
    fn try_from(file: FrameBufferStateFile) -> Result<Self, Self::Error> {
        let state = Self {
            settings: file.settings,
            content: file.content,
        };
        state.validate()?;
        Ok(state)
    }
}

#[derive(Debug, serde::Deserialize)]
pub enum FrameBufferMessage {
    State(Box<FrameBufferState>),
    Mode(FrameBufferColorMode, u8, u8, u8),
    Width(usize),
    Range(Vec<(i64, i64)>),
}
#[derive(Default)]
struct Runtime {
    array: Option<FrameBufferArrayCache>,
    pixels: Option<FrameBufferPixelCache>,
}
#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameBufferTile {
    pub(crate) state: FrameBufferState,
    #[serde(skip)]
    runtime: RefCell<Runtime>,
}
impl Clone for FrameBufferTile {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            runtime: Default::default(),
        }
    }
}
impl FrameBufferTile {
    pub(crate) fn reset_runtime(&mut self) {
        *self.runtime.get_mut() = Default::default();
    }
    pub(crate) fn update(&mut self, message: FrameBufferMessage) -> Result<bool, FrameBufferError> {
        let mut state = self.state.clone();
        match message {
            FrameBufferMessage::State(value) => state = *value,
            FrameBufferMessage::Width(width) => state.settings.pixels_per_row = width.max(1),
            FrameBufferMessage::Mode(mode, a, b, c) => {
                let settings = &mut state.settings.color_settings;
                settings.color_mode = mode;
                match mode {
                    FrameBufferColorMode::Grayscale => settings.grayscale_bits = a.clamp(1, 8),
                    FrameBufferColorMode::Rgb => {
                        settings.r_bits = a.min(8);
                        settings.g_bits = b.min(8);
                        settings.b_bits = c.min(8);
                    }
                    FrameBufferColorMode::YCbCr => {
                        settings.y_bits = a.min(8);
                        settings.cb_bits = b.min(8);
                        settings.cr_bits = c.min(8);
                    }
                }
            }
            FrameBufferMessage::Range(ranges) => {
                let Some(FrameBufferContent::Array { levels, .. }) = &mut state.content else {
                    return Ok(false);
                };
                for (level, (a, b)) in levels.iter_mut().zip(ranges) {
                    level.first_index = a.min(b).clamp(level.min_index, level.max_index);
                    level.last_index = a.max(b).clamp(level.min_index, level.max_index);
                }
            }
        }
        state.validate()?;
        if state == self.state {
            return Ok(false);
        }
        self.state = state;
        self.reset_runtime();
        Ok(true)
    }
    pub(crate) fn ui(
        &self,
        ui: &mut egui::Ui,
        id: TileId,
        document: Option<&crate::wave_data::WaveData>,
        messages: &mut Vec<Message>,
    ) {
        let mut state = self.state.clone();
        let mut runtime = self.runtime.borrow_mut();
        let Runtime { array, pixels } = &mut *runtime;
        crate::frame_buffer::draw_frame_buffer(
            ui,
            document,
            &mut state.settings,
            &mut state.content,
            array,
            pixels,
        );
        if state != self.state {
            messages.push(Message::ToTile(
                id,
                TileMessage::FrameBuffer(FrameBufferMessage::State(Box::new(state))),
            ));
        }
    }
    pub(crate) fn attach(
        &mut self,
        document: &mut crate::wave_data::WaveData,
    ) -> Option<crate::wellen::LoadSignalsCmd> {
        self.reset_runtime();
        let container = document.inner.as_waves_mut()?;
        let variables = match self.state.content.as_mut()? {
            FrameBufferContent::Variable(variable) => {
                *variable = variable
                    .clone()
                    .map_ids(|_| Default::default(), |_| Default::default());
                vec![variable.clone()]
            }
            FrameBufferContent::Array { scope_ref, levels } => {
                scope_ref.id = Default::default();
                if !container.scope_exists(scope_ref) {
                    return None;
                }
                let (mut fresh, variables) =
                    crate::frame_buffer::build_frame_buffer_content(container, scope_ref)?;
                for (new, old) in fresh.iter_mut().zip(levels.iter()) {
                    new.first_index = old.first_index.clamp(new.min_index, new.max_index);
                    new.last_index = old
                        .last_index
                        .clamp(new.min_index, new.max_index)
                        .max(new.first_index);
                }
                *levels = fresh;
                variables
            }
        };
        container
            .load_variables(variables.iter())
            .map_err(|error| tracing::warn!("Framebuffer load failed: {error}"))
            .ok()
            .flatten()
    }
}

impl crate::SystemState {
    pub(crate) fn framebuffer_target(&self) -> Option<TileId> {
        self.user
            .workspace
            .layout
            .focused()
            .into_iter()
            .chain(self.user.workspace.layout.focus_history().iter().copied())
            .chain(self.user.workspace.layout.tile_order())
            .find(|id| {
                matches!(
                    self.user.workspace.tiles.get(id).map(|entry| &entry.kind),
                    Some(crate::tiles::kind::TileKind::FrameBuffer(_))
                )
            })
    }
    pub(crate) fn open_framebuffer(
        &mut self,
        content: Option<FrameBufferContent>,
    ) -> Option<TileId> {
        if let Some(id) = self.framebuffer_target()
            && let Some(crate::tiles::kind::TileEntry {
                kind: crate::tiles::kind::TileKind::FrameBuffer(tile),
                ..
            }) = self.user.workspace.tiles.get(&id)
            && tile.state.content.is_none()
        {
            let mut state = tile.state.clone();
            state.content = content;
            self.update(Message::ToTile(
                id,
                TileMessage::FrameBuffer(FrameBufferMessage::State(Box::new(state))),
            ))?;
            if let Some(document) = self.user.waves.as_mut() {
                let crate::tiles::kind::TileKind::FrameBuffer(tile) =
                    &mut self.user.workspace.tiles.get_mut(&id)?.kind
                else {
                    return None;
                };
                if let Some(load) = tile.attach(document) {
                    self.load_variables(load);
                }
            }
            return Some(id);
        }
        self.open_framebuffer_state(FrameBufferState {
            content,
            ..Default::default()
        })
    }
    fn open_framebuffer_state(&mut self, state: FrameBufferState) -> Option<TileId> {
        use crate::tiles::{
            commands::WorkspaceCommand,
            kind::TileKind,
            layout::{Direction, Placement},
        };
        let command = WorkspaceCommand::CreateTile {
            kind: "frame_buffer".into(),
            placement: Placement::Edge(Direction::Right),
            focus: true,
        };
        let before =
            crate::tiles::history::ResourceEditStart::capture(&self.user.workspace, &command);
        self.user
            .workspace
            .apply_command(&mut self.workspace_runtime, command)
            .ok()?;
        let id = self.user.workspace.layout.focused()?;
        let TileKind::FrameBuffer(tile) = &mut self.user.workspace.tiles.get_mut(&id)?.kind else {
            return None;
        };
        tile.state = state;
        let load = self
            .user
            .waves
            .as_mut()
            .and_then(|document| tile.attach(document));
        if let Some(record) = before.and_then(|before| before.finish(&self.user.workspace)) {
            self.record_edit(record);
        }
        if let Some(load) = load {
            self.load_variables(load);
        }
        Some(id)
    }
    pub(crate) fn update_framebuffer(&mut self, command: FrameBufferMessage) -> Option<()> {
        if let Some(target) = self.framebuffer_target() {
            self.update(Message::ToTile(target, TileMessage::FrameBuffer(command)))
        } else {
            let mut tile = FrameBufferTile::default();
            if tile.update(command).ok()? {
                self.open_framebuffer_state(tile.state)?;
            }
            Some(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiles::{commands::WorkspaceCommand, kind::TileKind};
    fn source(name: &str) -> crate::wave_container::VariableRef {
        crate::wave_container::VariableRef {
            path: crate::wave_container::ScopeRef {
                strs: vec!["dut".into()],
                id: Default::default(),
            },
            name: name.into(),
            id: Default::default(),
            index: None,
        }
    }
    fn tile(state: &crate::SystemState, id: TileId) -> &FrameBufferTile {
        let TileKind::FrameBuffer(tile) = &state.user.workspace.tiles[&id].kind else {
            panic!()
        };
        tile
    }
    #[test]
    fn invalid_framebuffer_payloads_and_commands_preserve_live_state_and_redo() {
        let mut state = crate::SystemState::new_default_config().unwrap();
        state
            .update(Message::SetFrameBufferVariable(source("pixels")))
            .unwrap();
        let id = state.framebuffer_target().unwrap();
        state
            .update(Message::ToTile(
                id,
                TileMessage::FrameBuffer(FrameBufferMessage::Width(32)),
            ))
            .unwrap();
        state.update(Message::Undo(1)).unwrap();
        let original = tile(&state, id).state.clone();
        let history = state.undo_stack.len();
        let mut invalid = vec![original.clone(); 4];
        invalid[0].settings.pixels_per_row = 0;
        invalid[1].settings.color_settings.grayscale_bits = 0;
        invalid[2].settings.color_settings.r_bits = 255;
        invalid[3].content = Some(FrameBufferContent::Array {
            scope_ref: crate::wave_container::ScopeRef {
                strs: vec!["dut".into()],
                id: Default::default(),
            },
            levels: vec![crate::frame_buffer::ArrayLevel {
                min_index: 8,
                max_index: 1,
                first_index: 8,
                last_index: 1,
            }],
        });
        for invalid in invalid {
            let payload = FrameBufferTile {
                state: invalid.clone(),
                ..Default::default()
            };
            let file =
                crate::tiles::serde::TileFile::encode(None, "frame_buffer", 1, &payload).unwrap();
            assert!(crate::tiles::kind::TileEntry::from_file(file).is_err());
            assert!(
                state
                    .update(Message::ToTile(
                        id,
                        TileMessage::FrameBuffer(FrameBufferMessage::State(Box::new(invalid)))
                    ))
                    .is_none()
            );
            assert_eq!(tile(&state, id).state, original);
            assert_eq!(state.undo_stack.len(), history);
            assert_eq!(state.redo_stack.len(), 1);
        }
        state.update(Message::Redo(1)).unwrap();
        assert_eq!(tile(&state, id).state.settings.pixels_per_row, 32);
    }

    #[test]
    fn framebuffer_configuration_and_sources_are_independent_and_undoable() {
        let mut state = crate::SystemState::new_default_config().unwrap();
        state
            .update(Message::SetFrameBufferMode(
                FrameBufferColorMode::Rgb,
                5,
                6,
                5,
            ))
            .unwrap();
        let first = state.framebuffer_target().unwrap();
        assert_eq!(state.undo_stack.len(), 1);
        state
            .update(Message::SetFrameBufferVariable(source("red")))
            .unwrap();
        assert_eq!(state.user.workspace.tiles.len(), 1);
        assert_eq!(tile(&state, first).state.settings.color_settings.r_bits, 5);
        state
            .update(Message::SetFrameBufferVariable(source("blue")))
            .unwrap();
        let second = state.framebuffer_target().unwrap();
        assert_ne!(first, second);
        state
            .update(Message::ToTile(
                first,
                TileMessage::FrameBuffer(FrameBufferMessage::Width(32)),
            ))
            .unwrap();
        assert_eq!(state.user.workspace.layout.focused(), Some(second));
        assert_eq!(tile(&state, second).state.settings.pixels_per_row, 16);
        state.update(Message::Undo(1)).unwrap();
        assert_eq!(tile(&state, first).state.settings.pixels_per_row, 16);
        state.update(Message::Redo(1)).unwrap();
        assert_eq!(tile(&state, first).state.settings.pixels_per_row, 32);
        let saved = state.encode_state().unwrap();
        let restored: crate::state::UserState = crate::tiles::serde::decode(&saved).unwrap();
        let TileKind::FrameBuffer(restored) = &restored.workspace.tiles[&first].kind else {
            panic!()
        };
        assert_eq!(restored.state, tile(&state, first).state);
        state
            .update(Message::Workspace(WorkspaceCommand::CloseTile(first)))
            .unwrap();
        assert!(
            state
                .update(Message::ToTile(
                    first,
                    TileMessage::FrameBuffer(FrameBufferMessage::Width(40))
                ))
                .is_none()
        );
        state.update(Message::Undo(1)).unwrap();
        assert_eq!(tile(&state, first).state.settings.pixels_per_row, 32);
    }
    #[test]
    fn legacy_framebuffer_preferences_move_into_a_tile_without_inventing_a_source() {
        let settings = FrameBufferSettings {
            pixels_per_row: 31,
            ..Default::default()
        };
        let old = format!("(frame_buffer: {})", ron::to_string(&settings).unwrap());
        let restored: crate::state::UserState = crate::tiles::serde::decode(&old).unwrap();
        assert_eq!(restored.workspace.tiles.len(), 1);
        let TileKind::FrameBuffer(tile) = &restored.workspace.tiles.values().next().unwrap().kind
        else {
            panic!()
        };
        assert_eq!(tile.state.settings, settings);
        assert!(tile.state.content.is_none());
        let default = format!(
            "(frame_buffer: {})",
            ron::to_string(&FrameBufferSettings::default()).unwrap()
        );
        let restored: crate::state::UserState = crate::tiles::serde::decode(&default).unwrap();
        assert!(restored.workspace.tiles.is_empty());
    }
}
