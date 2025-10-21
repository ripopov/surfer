//! Keyboard handling.
use egui::{Context, Event, Key, Modifiers};
use emath::Vec2;

use crate::config::ArrowKeyBindings;
use crate::message::MessageTarget;
use crate::{
    message::Message,
    wave_data::{PER_SCROLL_EVENT, SCROLL_EVENTS_PER_PAGE},
    MoveDir, SystemState,
};

impl SystemState {
    pub fn handle_pressed_keys(&self, ctx: &Context, msgs: &mut Vec<Message>) {
        if !(self.command_prompt.visible | self.user.variable_name_filter_focused) {
            self.user.shortcuts.update(ctx, msgs, self);
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
                    self.user.variable_name_filter_focused,
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
                            handle_digit(d, modifiers, msgs);
                        }
                    }
                    (Key::Home, true, false, false) => msgs.push(Message::ScrollToItem(0)),
                    (Key::End, true, false, false) => {
                        if let Some(waves) = &self.user.waves {
                            if waves.displayed_items.len() > 1 {
                                msgs.push(Message::ScrollToItem(waves.displayed_items.len() - 1));
                            }
                        }
                    }
                    (Key::Space, true, false, false) => {
                        msgs.push(Message::ShowCommandPrompt(Some("".to_string())))
                    }
                    (Key::Escape, true, true, false) => msgs.push(Message::ShowCommandPrompt(None)),
                    (Key::Escape, true, false, false) => {
                        msgs.push(Message::InvalidateCount);
                        msgs.push(Message::ItemSelectionClear);
                    }
                    (Key::Escape, true, _, true) => msgs.push(Message::SetFilterFocused(false)),
                    (Key::A, true, false, false) => {
                        if modifiers.command {
                            msgs.push(Message::ItemSelectAll);
                        } else {
                            msgs.push(Message::ToggleItemSelected(None));
                        }
                    }
                    (Key::F, true, false, false) => {
                        msgs.push(Message::ShowCommandPrompt(Some("item_focus ".to_string())))
                    }
                    (Key::G, true, true, false) => {
                        if modifiers.command {
                            msgs.push(Message::ShowCommandPrompt(None))
                        }
                    }
                    (Key::G, true, false, false) => msgs.push(Message::GroupNew {
                        name: None,
                        before: None,
                        items: None,
                    }),
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
                    (Key::L, true, false, false) => msgs.push(Message::MoveCursorToTransition {
                        next: true,
                        variable: None,
                        skip_zero: modifiers.shift,
                    }),
                    (Key::M, true, false, false) => {
                        if modifiers.alt {
                            msgs.push(Message::ToggleMenu)
                        } else if let Some(waves) = self.user.waves.as_ref() {
                            if let Some(cursor) = waves.cursor.as_ref() {
                                // Check if a marker already exists at the cursor position
                                let marker_exists = waves
                                    .markers
                                    .values()
                                    .any(|marker_time| marker_time == cursor);
                                if !marker_exists {
                                    msgs.push(Message::AddMarker {
                                        time: cursor.clone(),
                                        name: None,
                                        move_focus: self
                                            .user
                                            .config
                                            .layout
                                            .move_focus_on_inserted_marker(),
                                    });
                                }
                            }
                        }
                    }
                    (Key::N, true, true, false) => {
                        if modifiers.command {
                            msgs.push(Message::SelectNextCommand);
                        }
                    }
                    (Key::O, true, false, false) if modifiers.command => {
                        let mode = if modifiers.shift {
                            crate::file_dialog::OpenMode::Switch
                        } else {
                            crate::file_dialog::OpenMode::Open
                        };
                        msgs.push(Message::OpenFileDialog(mode));
                    }
                    (Key::P, true, true, false) => {
                        if modifiers.command {
                            msgs.push(Message::SelectPrevCommand);
                        }
                    }
                    (Key::R, true, false, false) => msgs.push(Message::ReloadWaveform(
                        self.user.config.behavior.keep_during_reload,
                    )),
                    (Key::F2, true, false, _) => {
                        if let Some(waves) = &self.user.waves {
                            msgs.push(Message::RenameItem(waves.focused_item));
                        }
                    }
                    (Key::F11, true, false, _) => msgs.push(Message::ToggleFullscreen),
                    (Key::Minus, true, false, false) => {
                        if modifiers.ctrl && cfg!(not(target_arch = "wasm32")) {
                            let mut next_factor = 0f32;
                            for factor in &self.user.config.layout.zoom_factors {
                                if *factor < self.ui_zoom_factor() && *factor > next_factor {
                                    next_factor = *factor;
                                }
                            }
                            if next_factor > 0f32 {
                                msgs.push(Message::SetUIZoomFactor(next_factor));
                            }
                        } else {
                            msgs.push(Message::CanvasZoom {
                                mouse_ptr: None,
                                delta: 2.0,
                                viewport_idx: 0,
                            });
                        }
                    }
                    (Key::Plus | Key::Equals, true, false, false) => {
                        if modifiers.ctrl && cfg!(not(target_arch = "wasm32")) {
                            let mut next_factor = f32::INFINITY;
                            for factor in &self.user.config.layout.zoom_factors {
                                if *factor > self.ui_zoom_factor() && *factor < next_factor {
                                    next_factor = *factor;
                                }
                            }
                            if next_factor != f32::INFINITY {
                                msgs.push(Message::SetUIZoomFactor(next_factor));
                            }
                        } else {
                            msgs.push(Message::CanvasZoom {
                                mouse_ptr: None,
                                delta: 0.5,
                                viewport_idx: 0,
                            });
                        }
                    }
                    (Key::PageUp, true, false, false) => msgs.push(Message::CanvasScroll {
                        delta: Vec2 {
                            x: 0.,
                            y: -PER_SCROLL_EVENT * SCROLL_EVENTS_PER_PAGE,
                        },
                        viewport_idx: 0,
                    }),
                    (Key::PageDown, true, false, false) => msgs.push(Message::CanvasScroll {
                        delta: Vec2 {
                            x: 0.,
                            y: PER_SCROLL_EVENT * SCROLL_EVENTS_PER_PAGE,
                        },
                        viewport_idx: 0,
                    }),
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
                    (Key::Delete | Key::X, true, false, false) => {
                        if let Some(waves) = &self.user.waves {
                            let mut remove_ids = waves
                                .items_tree
                                .iter_visible_selected()
                                .map(|i| i.item_ref)
                                .collect::<Vec<_>>();
                            if let Some(node) = waves
                                .focused_item
                                .and_then(|focus| waves.items_tree.get_visible(focus))
                            {
                                remove_ids.push(node.item_ref)
                            }

                            msgs.push(Message::RemoveItems(remove_ids));
                        }
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
