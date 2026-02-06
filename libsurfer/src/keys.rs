//! Keyboard handling.
use egui::{Context, Event, Key, Modifiers};
use emath::Vec2;

use crate::config::ArrowKeyBindings;
use crate::message::MessageTarget;
use crate::{MoveDir, SystemState, message::Message, wave_data::PER_SCROLL_EVENT};

impl SystemState {
    fn table_copy_target(&self, ctx: &Context) -> Option<crate::table::TableTileId> {
        // Prefer the table that currently has keyboard focus.
        let focused_target = self.user.table_tiles.keys().copied().find(|tile_id| {
            let table_area_id = egui::Id::new(("table_area", tile_id.0));
            ctx.memory(|mem| mem.has_focus(table_area_id))
                && self
                    .table_runtime
                    .get(tile_id)
                    .is_some_and(|runtime| !runtime.selection.is_empty())
        });
        if focused_target.is_some() {
            return focused_target;
        }

        // Fallback: if focus is not tracked reliably, use any table with a non-empty selection.
        self.user
            .table_tiles
            .keys()
            .copied()
            .filter(|tile_id| {
                self.table_runtime
                    .get(tile_id)
                    .is_some_and(|runtime| !runtime.selection.is_empty())
            })
            .min_by_key(|tile_id| tile_id.0)
    }

    pub fn handle_pressed_keys(&self, ctx: &Context, msgs: &mut Vec<Message>) {
        if !(self.command_prompt.visible | self.user.variable_name_filter_focused) {
            self.user.config.shortcuts.process(ctx, msgs, self);
        }
        let table_copy_target = self.table_copy_target(ctx);
        ctx.input(|i| {
            let mut copy_handled = false;
            for event in &i.events {
                match event {
                    Event::Key {
                        key,
                        repeat: _,
                        pressed,
                        modifiers,
                        physical_key: _,
                    } => match (
                        key,
                        pressed,
                        self.command_prompt.visible,
                        self.user.variable_name_filter_focused,
                    ) {
                        (Key::C, true, false, false) if modifiers.command => {
                            if let Some(tile_id) = table_copy_target {
                                msgs.push(Message::TableCopySelection {
                                    tile_id,
                                    include_header: modifiers.shift,
                                });
                            } else {
                                msgs.push(Message::VariableValueToClipbord(
                                    MessageTarget::CurrentSelection,
                                ));
                            }
                            copy_handled = true;
                        }
                        // Consolidate numeric key handling into a single arm using helper
                        (k, true, false, false)
                            if matches!(
                                k,
                                Key::Num0
                                    | Key::Num1
                                    | Key::Num2
                                    | Key::Num3
                                    | Key::Num4
                                    | Key::Num5
                                    | Key::Num6
                                    | Key::Num7
                                    | Key::Num8
                                    | Key::Num9
                            ) =>
                        {
                            if let Some(d) = key_to_digit(k) {
                                handle_digit(d, modifiers, msgs);
                            }
                        }
                        (Key::Escape, true, true, false) => msgs.push(Message::HideCommandPrompt),
                        (Key::Escape, true, false, false) => {
                            msgs.push(Message::InvalidateCount);
                            msgs.push(Message::ItemSelectionClear);
                        }
                        (Key::Escape, true, _, true) => msgs.push(Message::SetFilterFocused(false)),
                        (Key::G, true, true, false) => {
                            if modifiers.command {
                                msgs.push(Message::HideCommandPrompt);
                            }
                        }
                        (Key::H, true, false, false) => {
                            msgs.push(Message::MoveCursorToTransition {
                                next: false,
                                variable: None,
                                skip_zero: modifiers.shift,
                            })
                        }
                        (Key::J, true, false, false) => {
                            if modifiers.alt {
                                msgs.push(Message::MoveFocus(
                                    MoveDir::Down,
                                    self.get_count(),
                                    modifiers.shift,
                                ));
                            } else if modifiers.command {
                                msgs.push(Message::MoveFocusedItem(
                                    MoveDir::Down,
                                    self.get_count(),
                                ));
                            } else {
                                msgs.push(Message::VerticalScroll(MoveDir::Down, self.get_count()));
                            }
                            msgs.push(Message::InvalidateCount);
                        }
                        (Key::K, true, false, false) => {
                            if modifiers.alt {
                                msgs.push(Message::MoveFocus(
                                    MoveDir::Up,
                                    self.get_count(),
                                    modifiers.shift,
                                ));
                            } else if modifiers.command {
                                msgs.push(Message::MoveFocusedItem(MoveDir::Up, self.get_count()));
                            } else {
                                msgs.push(Message::VerticalScroll(MoveDir::Up, self.get_count()));
                            }
                            msgs.push(Message::InvalidateCount);
                        }
                        (Key::L, true, false, false) => {
                            msgs.push(Message::MoveCursorToTransition {
                                next: true,
                                variable: None,
                                skip_zero: modifiers.shift,
                            })
                        }
                        (Key::N, true, true, false) => {
                            if modifiers.command {
                                msgs.push(Message::SelectNextCommand);
                            }
                        }
                        (Key::P, true, true, false) => {
                            if modifiers.command {
                                msgs.push(Message::SelectPrevCommand);
                            }
                        }
                        (Key::F11, true, false, _) => msgs.push(Message::ToggleFullscreen),
                        (Key::ArrowRight, true, false, false) => {
                            msgs.push(match self.user.config.behavior.arrow_key_bindings {
                                ArrowKeyBindings::Edge => Message::MoveCursorToTransition {
                                    next: true,
                                    variable: None,
                                    skip_zero: modifiers.shift,
                                },
                                ArrowKeyBindings::Scroll => Message::CanvasScroll {
                                    delta: Vec2 {
                                        x: 0.,
                                        y: -PER_SCROLL_EVENT,
                                    },
                                    viewport_idx: 0,
                                },
                            });
                        }
                        (Key::ArrowLeft, true, false, false) => {
                            msgs.push(match self.user.config.behavior.arrow_key_bindings {
                                ArrowKeyBindings::Edge => Message::MoveCursorToTransition {
                                    next: false,
                                    variable: None,
                                    skip_zero: modifiers.shift,
                                },
                                ArrowKeyBindings::Scroll => Message::CanvasScroll {
                                    delta: Vec2 {
                                        x: 0.,
                                        y: PER_SCROLL_EVENT,
                                    },
                                    viewport_idx: 0,
                                },
                            });
                        }
                        (Key::ArrowDown, true, true, false) => {
                            msgs.push(Message::SelectNextCommand)
                        }
                        (Key::ArrowDown, true, false, false) => {
                            if modifiers.alt {
                                msgs.push(Message::MoveFocus(
                                    MoveDir::Down,
                                    self.get_count(),
                                    modifiers.shift,
                                ));
                            } else if modifiers.command {
                                msgs.push(Message::MoveFocusedItem(
                                    MoveDir::Down,
                                    self.get_count(),
                                ));
                            } else {
                                msgs.push(Message::VerticalScroll(MoveDir::Down, self.get_count()));
                            }
                            msgs.push(Message::InvalidateCount);
                        }
                        (Key::ArrowUp, true, true, false) => msgs.push(Message::SelectPrevCommand),
                        (Key::ArrowUp, true, false, false) => {
                            if modifiers.alt {
                                msgs.push(Message::MoveFocus(
                                    MoveDir::Up,
                                    self.get_count(),
                                    modifiers.shift,
                                ));
                            } else if modifiers.command {
                                msgs.push(Message::MoveFocusedItem(MoveDir::Up, self.get_count()));
                            } else {
                                msgs.push(Message::VerticalScroll(MoveDir::Up, self.get_count()));
                            }
                            msgs.push(Message::InvalidateCount);
                        }
                        _ => {}
                    },
                    Event::Copy if !copy_handled => {
                        if let Some(tile_id) = table_copy_target {
                            msgs.push(Message::TableCopySelection {
                                tile_id,
                                include_header: false,
                            });
                        } else {
                            msgs.push(Message::VariableValueToClipbord(
                                MessageTarget::CurrentSelection,
                            ));
                        }
                        copy_handled = true;
                    }
                    _ => {}
                }
            }
        });
    }

    pub fn get_count(&self) -> usize {
        self.user
            .count
            .as_deref()
            .map(str::trim)
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(1)
    }
}

fn handle_digit(digit: u8, modifiers: &Modifiers, msgs: &mut Vec<Message>) {
    if modifiers.alt {
        // Convert 0..9 to '0'..'9' safely and clearly
        if let Some(c) = std::char::from_digit(digit as u32, 10) {
            msgs.push(Message::AddCount(c));
        }
    } else if modifiers.command {
        msgs.push(Message::MoveMarkerToCursor(digit));
    } else {
        msgs.push(Message::GoToMarkerPosition(digit, 0));
    }
}

fn key_to_digit(key: &Key) -> Option<u8> {
    match key {
        Key::Num0 => Some(0),
        Key::Num1 => Some(1),
        Key::Num2 => Some(2),
        Key::Num3 => Some(3),
        Key::Num4 => Some(4),
        Key::Num5 => Some(5),
        Key::Num6 => Some(6),
        Key::Num7 => Some(7),
        Key::Num8 => Some(8),
        Key::Num9 => Some(9),
        _ => None,
    }
}
