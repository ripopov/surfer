//! Read-only tile context and concretely targeted commands from tile chrome.

use std::collections::BTreeMap;

use super::{
    ItemListId, TileId,
    commands::{DocumentCommand, SplitMode, WorkspaceCommand},
    kind::{TileEntry, TileMessage},
    layout::Direction,
    runtime::WorkspaceRuntime,
};
use crate::{
    Message, config::SurferConfig, item_list::ItemList, translation::TranslatorList,
    wave_data::WaveData,
};

pub struct TileReadServices<'a> {
    pub document: Option<&'a WaveData>,
    pub item_lists: &'a BTreeMap<ItemListId, ItemList>,
    pub config: &'a SurferConfig,
    pub translators: &'a TranslatorList,
    pub runtime: &'a WorkspaceRuntime,
}

/// A tile can read shared services, but cannot mutate application state or
/// enqueue an ambient waveform command. Its identity is fixed for the pass.
pub struct TileCtx<'a> {
    services: TileReadServices<'a>,
    pub tile_id: TileId,
    pub focused: bool,
    commands: &'a mut Vec<Message>,
}

impl<'a> TileCtx<'a> {
    pub fn new(
        services: TileReadServices<'a>,
        tile_id: TileId,
        focused: bool,
        commands: &'a mut Vec<Message>,
    ) -> Self {
        Self {
            services,
            tile_id,
            focused,
            commands,
        }
    }

    pub fn waves(&self) -> Option<&WaveData> {
        self.services.document
    }
    pub fn config(&self) -> &SurferConfig {
        self.services.config
    }
    pub fn translators(&self) -> &TranslatorList {
        self.services.translators
    }
    pub fn item_list(&self, id: ItemListId) -> Option<&ItemList> {
        self.services.item_lists.get(&id)
    }
    pub fn id(&self, salt: impl std::hash::Hash + std::fmt::Debug) -> egui::Id {
        self.services.runtime.egui_id(self.tile_id, salt)
    }
    pub fn send_self(&mut self, message: TileMessage) {
        self.commands.push(Message::ToTile(self.tile_id, message));
    }
    pub fn send_document(&mut self, message: DocumentCommand) {
        self.commands.push(Message::ToDocument(message));
    }
    pub fn send_workspace(&mut self, message: WorkspaceCommand) {
        self.commands.push(Message::Workspace(message));
    }
}

pub fn tab_context_menu(entry: &TileEntry, ui: &mut egui::Ui, cx: &mut TileCtx<'_>) {
    if ui.button("Focus").clicked() {
        cx.send_workspace(WorkspaceCommand::FocusTile(cx.tile_id));
        ui.close();
    }
    ui.menu_button("Rename", |ui| {
        let id = cx.id("rename");
        let mut title = ui
            .data_mut(|data| data.get_temp::<String>(id))
            .unwrap_or_else(|| entry.display_title());
        let response = ui.text_edit_singleline(&mut title);
        let submit = response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        ui.data_mut(|data| data.insert_temp(id, title.clone()));
        if ui.button("Apply").clicked() || submit {
            cx.send_workspace(WorkspaceCommand::RenameTile {
                tile: cx.tile_id,
                title: Some(title),
            });
            ui.data_mut(|data| data.remove::<String>(id));
            ui.close();
        }
        if ui.button("Use default title").clicked() {
            cx.send_workspace(WorkspaceCommand::RenameTile {
                tile: cx.tile_id,
                title: None,
            });
            ui.data_mut(|data| data.remove::<String>(id));
            ui.close();
        }
    });
    for (label, mode) in [
        ("Split linked", SplitMode::Linked),
        ("Split independent", SplitMode::Independent),
        ("Split copy", SplitMode::Clone),
    ] {
        ui.add_enabled_ui(entry.kind.supports_split(mode), |ui| {
            ui.menu_button(label, |ui| {
                for (label, dir) in [
                    ("Left", Direction::Left),
                    ("Right", Direction::Right),
                    ("Up", Direction::Up),
                    ("Down", Direction::Down),
                ] {
                    if ui.button(label).clicked() {
                        cx.send_workspace(WorkspaceCommand::SplitTile {
                            tile: cx.tile_id,
                            dir,
                            mode,
                        });
                        ui.close();
                    }
                }
            });
        });
    }
    entry.kind.tab_context_menu(ui, cx);
    ui.separator();
    if ui.button("Close other tabs").clicked() {
        cx.send_workspace(WorkspaceCommand::CloseOtherTiles(cx.tile_id));
        ui.close();
    }
    if ui.button("Close").clicked() {
        cx.send_workspace(WorkspaceCommand::CloseTile(cx.tile_id));
        ui.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        SystemState,
        tiles::{kind::TileKind, layout::Placement},
    };

    fn create(state: &mut SystemState) -> TileId {
        let placement = state
            .user
            .workspace
            .layout
            .focused()
            .map_or(Placement::Root, Placement::TabAfter);
        state
            .update(Message::Workspace(WorkspaceCommand::CreateTile {
                kind: "waveform".into(),
                placement,
                focus: true,
            }))
            .unwrap();
        state.user.workspace.layout.focused().unwrap()
    }

    fn frame(
        state: &SystemState,
        id: TileId,
        context: &egui::Context,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Vec<Message>) {
        let mut commands = Vec::new();
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(500.0, 300.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                let mut cx = TileCtx::new(
                    TileReadServices {
                        document: None,
                        item_lists: &state.user.workspace.item_lists,
                        config: &state.user.config,
                        translators: &state.translators,
                        runtime: &state.workspace_runtime,
                    },
                    id,
                    state.user.workspace.layout.focused() == Some(id),
                    &mut commands,
                );
                state.user.workspace.tiles[&id]
                    .kind
                    .tab_context_menu(ui, &mut cx);
            },
        );
        output.textures_delta.clear();
        (output, commands)
    }

    #[test]
    fn waveform_checkbox_keeps_its_tile_after_focus_changes() {
        let mut state = SystemState::new_default_config().unwrap();
        let first = create(&mut state);
        let second = create(&mut state);
        let context = egui::Context::default();
        let (output, initial) = frame(&state, first, &context, vec![]);
        assert!(initial.is_empty());
        let position = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Name column" => {
                    Some(text.pos + egui::vec2(4.0, 4.0))
                }
                _ => None,
            })
            .expect("column checkbox label");
        let pointer = |pressed| egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(
            &state,
            first,
            &context,
            vec![egui::Event::PointerMoved(position), pointer(true)],
        );
        let (_, commands) = frame(&state, first, &context, vec![pointer(false)]);
        assert_eq!(commands.len(), 1);
        assert!(matches!(&commands[0], Message::ToTile(id, _) if *id == first));
        let TileKind::Waveform(before) = &state.user.workspace.tiles[&first].kind else {
            panic!()
        };
        assert!(before.show_name_column, "drawing must not mutate settings");
        state
            .update(Message::Workspace(WorkspaceCommand::FocusTile(second)))
            .unwrap();
        for command in commands {
            state.update(command).unwrap();
        }
        let TileKind::Waveform(first_tile) = &state.user.workspace.tiles[&first].kind else {
            panic!()
        };
        let TileKind::Waveform(second_tile) = &state.user.workspace.tiles[&second].kind else {
            panic!()
        };
        assert!(!first_tile.show_name_column);
        assert!(first_tile.show_value_column);
        assert!(second_tile.show_name_column && second_tile.show_value_column);
        assert_eq!(state.user.workspace.layout.focused(), Some(second));
    }
}
