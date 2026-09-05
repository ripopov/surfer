//! Log presentation belongs to a singleton tile; the recording buffer is shared.
use egui::{RichText, TextWrapMode};
use egui_extras::{Column, TableBuilder, TableRow};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LevelFilter {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    #[default]
    Trace,
}
impl LevelFilter {
    fn includes(self, level: tracing::Level) -> bool {
        match self {
            Self::Off => false,
            Self::Error => level <= tracing::Level::ERROR,
            Self::Warn => level <= tracing::Level::WARN,
            Self::Info => level <= tracing::Level::INFO,
            Self::Debug => level <= tracing::Level::DEBUG,
            Self::Trace => true,
        }
    }
}

#[derive(Debug, Deserialize)]
pub enum LogsMessage {
    SetFilter(LevelFilter),
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogsTile {
    pub filter: LevelFilter,
}
impl LogsTile {
    pub(crate) fn update(&mut self, message: LogsMessage) -> bool {
        match message {
            LogsMessage::SetFilter(filter) => {
                let changed = self.filter != filter;
                self.filter = filter;
                changed
            }
        }
    }
    pub(crate) fn ui(&self, ui: &mut egui::Ui, cx: &mut crate::tiles::view::TileCtx<'_>) {
        let mut filter = self.filter;
        egui::ComboBox::from_id_salt(cx.id("severity"))
            .selected_text(format!("{filter:?}"))
            .show_ui(ui, |ui| {
                for level in [
                    LevelFilter::Off,
                    LevelFilter::Error,
                    LevelFilter::Warn,
                    LevelFilter::Info,
                    LevelFilter::Debug,
                    LevelFilter::Trace,
                ] {
                    ui.selectable_value(&mut filter, level, format!("{level:?}"));
                }
            });
        if filter != self.filter {
            cx.send_self(crate::tiles::kind::TileMessage::Logs(
                LogsMessage::SetFilter(filter),
            ));
        }
        ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);
        let records = crate::logs::records();
        let filtered = records
            .iter()
            .filter(|record| self.filter.includes(record.level))
            .collect::<Vec<_>>();
        egui::ScrollArea::horizontal().show(ui, |ui| {
            TableBuilder::new(ui)
                .column(Column::auto().resizable(true))
                .column(Column::auto().resizable(true))
                .column(Column::remainder())
                .vscroll(true)
                .stick_to_bottom(true)
                .header(20.0, |mut header| {
                    for title in ["Level", "Source", "Message"] {
                        header.col(|ui| {
                            ui.heading(title);
                        });
                    }
                })
                .body(|body| {
                    let heights = filtered
                        .iter()
                        .map(|record| record.msg.lines().count().max(1) as f32 * 15.0);
                    body.heterogeneous_rows(heights, |mut row: TableRow| {
                        let record = filtered[row.index()];
                        row.col(|ui| {
                            let color = match record.level {
                                tracing::Level::ERROR => egui::Color32::RED,
                                tracing::Level::WARN => egui::Color32::YELLOW,
                                tracing::Level::INFO => egui::Color32::GREEN,
                                tracing::Level::DEBUG => egui::Color32::BLUE,
                                tracing::Level::TRACE => egui::Color32::GRAY,
                            };
                            ui.colored_label(color, record.level.to_string());
                        });
                        row.col(|ui| {
                            ui.label(
                                RichText::new(&record.name)
                                    .color(egui::Color32::GRAY)
                                    .monospace(),
                            );
                        });
                        row.col(|ui| {
                            ui.label(RichText::new(&record.msg).monospace());
                        });
                    });
                });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Message, SystemState,
        tiles::{
            commands::WorkspaceCommand,
            kind::{TileKind, TileMessage},
            layout::{Direction, Placement},
            render::PaneRenderer,
        },
    };

    fn open(state: &mut SystemState, focus: bool) {
        state
            .update(Message::Workspace(WorkspaceCommand::OpenTile {
                kind: "logs".into(),
                placement: Placement::Edge(Direction::Down),
                focus,
            }))
            .unwrap();
    }

    #[test]
    fn severity_filter_is_a_threshold_including_more_severe_records() {
        let levels = [
            tracing::Level::ERROR,
            tracing::Level::WARN,
            tracing::Level::INFO,
            tracing::Level::DEBUG,
            tracing::Level::TRACE,
        ];
        for (count, filter) in [
            LevelFilter::Off,
            LevelFilter::Error,
            LevelFilter::Warn,
            LevelFilter::Info,
            LevelFilter::Debug,
            LevelFilter::Trace,
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(
                levels.map(|level| filter.includes(level)),
                std::array::from_fn(|index| index < count)
            );
        }
    }

    #[test]
    fn logs_are_a_persisted_singleton_and_stale_commands_do_not_reopen_them() {
        let mut state = SystemState::new_default_config().unwrap();
        state
            .update(Message::Workspace(WorkspaceCommand::CreateTile {
                kind: "waveform".into(),
                placement: Placement::Root,
                focus: true,
            }))
            .unwrap();
        let waveform = state.user.workspace.layout.focused().unwrap();
        open(&mut state, false);
        let logs = *state
            .user
            .workspace
            .tiles
            .iter()
            .find(|(_, entry)| entry.kind.kind_name() == "logs")
            .unwrap()
            .0;
        assert_eq!(state.user.workspace.layout.focused(), Some(waveform));
        let revision = state.user.workspace.layout.revision();
        open(&mut state, false);
        assert_eq!(state.user.workspace.layout.revision(), revision);
        assert_eq!(state.user.workspace.tiles.len(), 2);
        state
            .update(Message::Workspace(WorkspaceCommand::CreateTile {
                kind: "logs".into(),
                placement: Placement::Root,
                focus: true,
            }))
            .unwrap();
        assert_eq!(state.user.workspace.layout.focused(), Some(logs));
        assert_eq!(state.user.workspace.tiles.len(), 2);
        state
            .update(Message::ToTile(
                logs,
                TileMessage::Logs(LogsMessage::SetFilter(LevelFilter::Warn)),
            ))
            .unwrap();
        let encoded = state.encode_state().unwrap();
        assert!(!encoded.contains("show_logs"));
        let restored: crate::state::UserState = crate::tiles::serde::decode(&encoded).unwrap();
        let TileKind::Logs(tile) = &restored.workspace.tiles[&logs].kind else {
            panic!()
        };
        assert_eq!(tile.filter, LevelFilter::Warn);
        assert!(
            state
                .update(Message::Workspace(WorkspaceCommand::SplitTile {
                    tile: logs,
                    dir: Direction::Right,
                    mode: crate::tiles::commands::SplitMode::Clone,
                }))
                .is_none()
        );
        state
            .update(Message::Workspace(WorkspaceCommand::CloseTile(logs)))
            .unwrap();
        assert!(
            state
                .update(Message::ToTile(
                    logs,
                    TileMessage::Logs(LogsMessage::SetFilter(LevelFilter::Off))
                ))
                .is_none()
        );
        assert_eq!(state.user.workspace.tiles.len(), 1);
        open(&mut state, true);
        assert_ne!(state.user.workspace.layout.focused(), Some(logs));
    }

    #[test]
    fn filter_widget_emits_a_captured_command_without_mutating_the_tile() {
        let mut state = SystemState::new_default_config().unwrap();
        open(&mut state, true);
        let logs = state.user.workspace.layout.focused().unwrap();
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
                    crate::tiles::kind::ApplicationPanes::new(state, false).ui(
                        logs,
                        true,
                        ui,
                        &mut messages,
                    );
                },
            );
            output.textures_delta.clear();
            (output, messages)
        };
        fn text_pos(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos + egui::vec2(5.0, 5.0))
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| text_pos(shape, label)),
                _ => None,
            }
        }
        let position = |output: &egui::FullOutput, label| {
            output
                .shapes
                .iter()
                .find_map(|shape| text_pos(&shape.shape, label))
                .unwrap()
        };
        let event = |pos, pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    pressed,
                    button: egui::PointerButton::Primary,
                    modifiers: Default::default(),
                },
            ]
        };
        let (output, _) = frame(Vec::new(), &state);
        let pos = position(&output, "Trace");
        frame(event(pos, true), &state);
        frame(event(pos, false), &state);
        let (output, _) = frame(Vec::new(), &state);
        let warn = position(&output, "Warn");
        frame(event(warn, true), &state);
        let (_, messages) = frame(event(warn, false), &state);
        assert_eq!(messages.len(), 1);
        assert!(
            matches!(&messages[0], Message::ToTile(id, TileMessage::Logs(LogsMessage::SetFilter(LevelFilter::Warn))) if *id == logs)
        );
        let TileKind::Logs(tile) = &state.user.workspace.tiles[&logs].kind else {
            panic!()
        };
        assert_eq!(tile.filter, LevelFilter::Trace);
        state
            .update(Message::Workspace(WorkspaceCommand::CreateTile {
                kind: "waveform".into(),
                placement: Placement::TabAfter(logs),
                focus: true,
            }))
            .unwrap();
        for message in messages {
            state.update(message).unwrap();
        }
        let TileKind::Logs(tile) = &state.user.workspace.tiles[&logs].kind else {
            panic!()
        };
        assert_eq!(tile.filter, LevelFilter::Warn);
        assert_ne!(state.user.workspace.layout.focused(), Some(logs));
    }
}
