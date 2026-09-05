//! Singleton inspector following the resolved waveform's marker rows.
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkersTile {}

impl MarkersTile {
    pub(crate) fn ui(
        &self,
        ui: &mut egui::Ui,
        waves: Option<crate::wave_data::WaveformRead<'_>>,
        services: &super::waveform_services::WaveformReadServices<'_>,
        messages: &mut Vec<crate::Message>,
    ) {
        if let Some(waves) = waves {
            services.draw_marker_table(&waves, ui, messages);
        } else {
            ui.label("Open a waveform tile to inspect its markers.");
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        Message, SystemState,
        tiles::{
            commands::WorkspaceCommand,
            layout::{Direction, Placement},
            render::PaneRenderer,
        },
    };

    #[test]
    fn legacy_marker_window_becomes_a_singleton_tile() {
        for flag in ["show_cursor_window", "show_marker_window"] {
            let state: crate::state::UserState =
                crate::tiles::serde::decode(&format!("({flag}: true)")).unwrap();
            assert_eq!(state.workspace.tiles.len(), 1);
            assert_eq!(
                state
                    .workspace
                    .tiles
                    .values()
                    .next()
                    .unwrap()
                    .kind
                    .kind_name(),
                "markers"
            );
            let encoded = ron::to_string(&state).unwrap();
            assert!(!encoded.contains(flag));
            let restored: crate::state::UserState = crate::tiles::serde::decode(&encoded).unwrap();
            assert_eq!(restored.workspace.tiles.len(), 1);
        }
    }

    #[test]
    fn markers_follow_waveform_history_and_clicks_keep_the_captured_waveform() {
        let mut state = SystemState::new_default_config().unwrap();
        state.user.waves = Some(crate::wave_data::WaveData {
            inner: crate::data_container::DataContainer::Empty,
            source: crate::wave_source::WaveSource::Data,
            format: crate::wave_source::WaveFormat::Vcd,
            active_scope: None,
            cursor: None,
            markers: [(0, 10.into())].into_iter().collect(),
            display_variable_indices: false,
            old_max_timestamp: None,
            cache_generation: 0,
            inflight_caches: Default::default(),
            cached_time_range: Default::default(),
        });
        let mut waveforms = Vec::new();
        for name in ["Alpha", "Beta"] {
            state
                .update(Message::Workspace(WorkspaceCommand::CreateTile {
                    kind: "waveform".into(),
                    placement: waveforms
                        .last()
                        .copied()
                        .map_or(Placement::Root, Placement::TabAfter),
                    focus: true,
                }))
                .unwrap();
            let id = state.user.workspace.layout.focused().unwrap();
            let waves = state.user.waveform_edit_at(id).unwrap();
            waves
                .items
                .insert_item(
                    crate::displayed_item::DisplayedItem::Marker(
                        crate::displayed_item::DisplayedMarker {
                            idx: 0,
                            name: Some(name.into()),
                            color: None,
                            background_color: None,
                        },
                    ),
                    waves.items.end_insert_position(),
                )
                .unwrap();
            waveforms.push(id);
        }
        state
            .update(Message::Workspace(WorkspaceCommand::OpenTile {
                kind: "markers".into(),
                placement: Placement::Edge(Direction::Right),
                focus: true,
            }))
            .unwrap();
        let markers = state.user.workspace.layout.focused().unwrap();
        let ctx = egui::Context::default();
        let frame = |events, state: &SystemState| {
            let mut messages = Vec::new();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600.0, 400.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    crate::tiles::kind::ApplicationPanes {
                        state,
                        focus_ids: false,
                    }
                    .ui(markers, true, ui, &mut messages);
                },
            );
            output.textures_delta.clear();
            (output, messages)
        };
        fn text_pos(shape: &egui::Shape, name: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text().contains(name) => {
                    Some(text.pos + egui::vec2(5.0, 5.0))
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| text_pos(shape, name)),
                _ => None,
            }
        }
        let pos = |output: &egui::FullOutput, name| {
            output
                .shapes
                .iter()
                .find_map(|shape| text_pos(&shape.shape, name))
        };
        let (output, _) = frame(Vec::new(), &state);
        assert!(pos(&output, "Beta").is_some());
        assert!(pos(&output, "Alpha").is_none());
        state
            .update(Message::Workspace(WorkspaceCommand::FocusTile(
                waveforms[0],
            )))
            .unwrap();
        state
            .update(Message::Workspace(WorkspaceCommand::FocusTile(markers)))
            .unwrap();
        let (output, _) = frame(Vec::new(), &state);
        let click_pos = pos(&output, "Alpha").unwrap();
        assert!(pos(&output, "Beta").is_none());
        let events = |pressed| {
            vec![
                egui::Event::PointerMoved(click_pos),
                egui::Event::PointerButton {
                    pos: click_pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ]
        };
        frame(events(true), &state);
        let (_, messages) = frame(events(false), &state);
        assert_eq!(messages.len(), 1);
        assert!(matches!(messages[0], Message::GoToMarkerPosition(0, id) if id == waveforms[0]));
        state
            .update(Message::Workspace(WorkspaceCommand::CloseTile(
                waveforms[0],
            )))
            .unwrap();
        for message in messages {
            assert!(state.update(message).is_none());
        }
        assert_eq!(
            state
                .user
                .workspace
                .resolve_waveform(crate::tiles::TileTarget::Focused),
            Some(waveforms[1])
        );
    }
}
