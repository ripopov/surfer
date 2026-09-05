//! Keyboard handling.
use egui::{Context, Event, Key, Modifiers};
use emath::Vec2;

use crate::config::ArrowKeyBindings;
use crate::message::MessageTarget;
use crate::{MoveDir, SystemState, message::Message, wave_data::PER_SCROLL_EVENT};

impl SystemState {
    pub fn handle_pressed_keys(&self, ctx: &Context, msgs: &mut Vec<Message>) {
        let any_widget_focused = self.text_edit_focused.values().any(|&v| v);

        if !(self.command_prompt.visible || any_widget_focused) {
            self.user.config.shortcuts.process(ctx, msgs, self);
        }
        ctx.input(|i| {
            i.events.iter().for_each(|event| match event {
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
                    any_widget_focused,
                ) {
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
                            handle_digit(
                                d,
                                modifiers,
                                self.user
                                    .workspace
                                    .resolve_waveform(crate::tiles::TileTarget::Focused),
                                msgs,
                            );
                        }
                    }
                    (Key::Escape, true, true, false) => msgs.push(Message::HideCommandPrompt),
                    (Key::Escape, true, false, false) => {
                        msgs.push(Message::InvalidateCount);
                        msgs.push(Message::ItemSelectionClear);
                    }
                    (Key::Escape, true, _, true) => {
                        msgs.push(Message::ClearAllTextEditFocuses);
                    }
                    (Key::G, true, true, false) if modifiers.command => {
                        msgs.push(Message::HideCommandPrompt);
                    }
                    (Key::H, true, false, false) => msgs.push(Message::MoveCursorToTransition {
                        next: false,
                        variable: None,
                        skip_zero: modifiers.shift,
                    }),
                    (Key::J, true, false, false) => {
                        if modifiers.alt {
                            msgs.push(Message::MoveFocus(
                                MoveDir::Down,
                                self.get_count(),
                                modifiers.shift,
                            ));
                        } else if modifiers.command {
                            msgs.push(Message::MoveFocusedItem(MoveDir::Down, self.get_count()));
                        } else {
                            msgs.extend(self.scroll_rows_message(true, self.get_count()));
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
                            msgs.extend(self.scroll_rows_message(false, self.get_count()));
                        }
                        msgs.push(Message::InvalidateCount);
                    }
                    (Key::L, true, false, false) => msgs.push(Message::MoveCursorToTransition {
                        next: true,
                        variable: None,
                        skip_zero: modifiers.shift,
                    }),
                    (Key::N, true, true, false) if modifiers.command => {
                        msgs.push(Message::SelectNextCommand);
                    }
                    (Key::P, true, true, false) if modifiers.command => {
                        msgs.push(Message::SelectPrevCommand);
                    }
                    (Key::F11, true, false, _) => msgs.push(Message::ToggleFullscreen),
                    (Key::ArrowRight, true, false, false) => {
                        msgs.extend(match self.user.config.behavior.arrow_key_bindings() {
                            ArrowKeyBindings::Edge => Some(Message::MoveCursorToTransition {
                                next: true,
                                variable: None,
                                skip_zero: modifiers.shift,
                            }),
                            ArrowKeyBindings::Scroll => self
                                .user
                                .workspace
                                .resolve_waveform(crate::tiles::TileTarget::Focused)
                                .map(|tile_id| Message::CanvasScroll {
                                    delta: Vec2 {
                                        x: 0.,
                                        y: -PER_SCROLL_EVENT,
                                    },
                                    tile_id,
                                }),
                        });
                    }
                    (Key::ArrowLeft, true, false, false) => {
                        msgs.extend(match self.user.config.behavior.arrow_key_bindings() {
                            ArrowKeyBindings::Edge => Some(Message::MoveCursorToTransition {
                                next: false,
                                variable: None,
                                skip_zero: modifiers.shift,
                            }),
                            ArrowKeyBindings::Scroll => self
                                .user
                                .workspace
                                .resolve_waveform(crate::tiles::TileTarget::Focused)
                                .map(|tile_id| Message::CanvasScroll {
                                    delta: Vec2 {
                                        x: 0.,
                                        y: PER_SCROLL_EVENT,
                                    },
                                    tile_id,
                                }),
                        });
                    }
                    (Key::ArrowDown, true, true, false) => msgs.push(Message::SelectNextCommand),
                    (Key::ArrowDown, true, false, false) => {
                        if modifiers.alt {
                            msgs.push(Message::MoveFocus(
                                MoveDir::Down,
                                self.get_count(),
                                modifiers.shift,
                            ));
                        } else if modifiers.command {
                            msgs.push(Message::MoveFocusedItem(MoveDir::Down, self.get_count()));
                        } else {
                            msgs.extend(self.scroll_rows_message(true, self.get_count()));
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
                            msgs.extend(self.scroll_rows_message(false, self.get_count()));
                        }
                        msgs.push(Message::InvalidateCount);
                    }
                    _ => {}
                },
                Event::Copy => msgs.push(Message::VariableValueToClipbord(
                    MessageTarget::CurrentSelection,
                )),
                _ => {}
            });
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

fn handle_digit(
    digit: u8,
    modifiers: &Modifiers,
    tile_id: Option<crate::tiles::TileId>,
    msgs: &mut Vec<Message>,
) {
    if modifiers.alt {
        // Convert 0..9 to '0'..'9' safely and clearly
        if let Some(c) = std::char::from_digit(u32::from(digit), 10) {
            msgs.push(Message::AddCount(c));
        }
    } else if modifiers.command {
        msgs.push(Message::MoveMarkerToCursor(digit));
    } else if let Some(tile_id) = tile_id {
        msgs.push(Message::GoToMarkerPosition(digit, tile_id));
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
