use egui::{KeyboardShortcut, ModifierNames, Modifiers, Vec2};
use eyre::Result;
use serde::{Deserialize, Deserializer, Serialize};

use crate::SystemState;
use crate::message::{Message, MessageTarget};
use crate::wave_data::{PER_SCROLL_EVENT, SCROLL_EVENTS_PER_PAGE};

impl SystemState {
    /// Resolve a tile shortcut against the focused tile; a missing target is a no-op.
    fn tile_command(
        &self,
        resolve: impl FnOnce(
            &crate::tiles::workspace::Workspace,
        ) -> Option<crate::tiles::commands::WorkspaceCommand>,
    ) -> Option<Message> {
        resolve(&self.user.workspace).map(Message::Workspace)
    }
}

const NUMBER_OF_SHORTCUTS: usize = 46; // Update to reflect the actual number of shortcut actions defined in ShortcutAction enum

// Table-driven dispatch action enum
#[derive(Clone, Copy, Debug)]
pub enum ShortcutAction {
    OpenFile,
    SwitchFile,
    Redo,
    Undo,
    ToggleSidePanel,
    ToggleToolbar,
    GoToEnd,
    GoToStart,
    SaveStateFile,
    GoToTop,
    GoToBottom,
    ItemFocus,
    GroupNew,
    SelectAll,
    SelectToggle,
    ReloadWaveform,
    ZoomIn,
    ZoomOut,
    UiZoomIn,
    UiZoomOut,
    ScrollUp,
    ScrollDown,
    DeleteSelected,
    MarkerAdd,
    ToggleMenu,
    ShowCommandPrompt,
    RenameItem,
    DividerAdd,
    ZoomToFit,
    ZoomToCursor,
    GoToTime,
    FocusVariableNameFilter,
    TileSplitRight,
    TileSplitDown,
    TileClose,
    TileNext,
    TilePrev,
    TileFocusLeft,
    TileFocusRight,
    TileFocusUp,
    TileFocusDown,
    TileMoveLeft,
    TileMoveRight,
    TileMoveUp,
    TileMoveDown,
    ShowLogs,
}

// Cached dispatch table entry: (action, modifier_priority)
#[derive(Clone, Debug)]
struct DispatchEntry {
    action: ShortcutAction,
    priority: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SurferShortcuts {
    #[serde(with = "keyboard_shortcuts_serde")]
    pub open_file: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub switch_file: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub undo: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub redo: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub toggle_side_panel: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub toggle_toolbar: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub goto_end: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub goto_start: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub save_state_file: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub goto_top: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub goto_bottom: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub group_new: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub item_focus: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub select_all: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub select_toggle: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub reload_waveform: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub zoom_in: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub zoom_out: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub ui_zoom_in: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub ui_zoom_out: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub scroll_up: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub scroll_down: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub delete_selected: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub marker_add: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub toggle_menu: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub show_command_prompt: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub rename_item: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub divider_add: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub zoom_to_fit: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub zoom_to_cursor: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub go_to_time: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub focus_variable_name_filter: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub tile_split_right: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub tile_split_down: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub tile_close: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub tile_next: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub tile_prev: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub tile_focus_left: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub tile_focus_right: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub tile_focus_up: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub tile_focus_down: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub tile_move_left: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub tile_move_right: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub tile_move_up: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub tile_move_down: Vec<KeyboardShortcut>,
    #[serde(with = "keyboard_shortcuts_serde")]
    pub show_logs: Vec<KeyboardShortcut>,

    #[serde(skip)]
    cached_dispatch_table: Vec<DispatchEntry>,
}

pub fn deserialize_shortcuts<'de, D>(deserializer: D) -> Result<SurferShortcuts, D::Error>
where
    D: Deserializer<'de>,
{
    let mut shortcuts = SurferShortcuts::deserialize(deserializer)?;
    shortcuts.cached_dispatch_table = shortcuts.build_dispatch_table();
    Ok(shortcuts)
}

impl SurferShortcuts {
    #[must_use]
    pub fn format_shortcut(&self, action: ShortcutAction) -> String {
        // Determine if the target OS is macOS for formatting purposes.
        // Skip macOS formatting for tests.
        #[cfg(any(not(target_os = "macos"), test))]
        let is_mac = false;
        #[cfg(all(target_os = "macos", not(test)))]
        let is_mac = true;
        self.shortcuts_for_action(action)
            .iter()
            .map(|kb| kb.format(&ModifierNames::NAMES, is_mac))
            .collect::<Vec<String>>()
            .join("/")
    }

    fn build_dispatch_table(&self) -> Vec<DispatchEntry> {
        // Pre-allocate with known capacity and build entries
        let mut dispatch_table = Vec::with_capacity(NUMBER_OF_SHORTCUTS);

        // Create entry for each action with its priority
        dispatch_table.extend_from_slice(&[
            DispatchEntry {
                action: ShortcutAction::OpenFile,
                priority: modifier_priority(&self.open_file),
            },
            DispatchEntry {
                action: ShortcutAction::SwitchFile,
                priority: modifier_priority(&self.switch_file),
            },
            DispatchEntry {
                action: ShortcutAction::Redo,
                priority: modifier_priority(&self.redo),
            },
            DispatchEntry {
                action: ShortcutAction::Undo,
                priority: modifier_priority(&self.undo),
            },
            DispatchEntry {
                action: ShortcutAction::ToggleSidePanel,
                priority: modifier_priority(&self.toggle_side_panel),
            },
            DispatchEntry {
                action: ShortcutAction::ToggleToolbar,
                priority: modifier_priority(&self.toggle_toolbar),
            },
            DispatchEntry {
                action: ShortcutAction::GoToEnd,
                priority: modifier_priority(&self.goto_end),
            },
            DispatchEntry {
                action: ShortcutAction::GoToStart,
                priority: modifier_priority(&self.goto_start),
            },
            DispatchEntry {
                action: ShortcutAction::SaveStateFile,
                priority: modifier_priority(&self.save_state_file),
            },
            DispatchEntry {
                action: ShortcutAction::GoToTop,
                priority: modifier_priority(&self.goto_top),
            },
            DispatchEntry {
                action: ShortcutAction::GoToBottom,
                priority: modifier_priority(&self.goto_bottom),
            },
            DispatchEntry {
                action: ShortcutAction::GroupNew,
                priority: modifier_priority(&self.group_new),
            },
            DispatchEntry {
                action: ShortcutAction::ItemFocus,
                priority: modifier_priority(&self.item_focus),
            },
            DispatchEntry {
                action: ShortcutAction::SelectAll,
                priority: modifier_priority(&self.select_all),
            },
            DispatchEntry {
                action: ShortcutAction::SelectToggle,
                priority: modifier_priority(&self.select_toggle),
            },
            DispatchEntry {
                action: ShortcutAction::ReloadWaveform,
                priority: modifier_priority(&self.reload_waveform),
            },
            DispatchEntry {
                action: ShortcutAction::ZoomIn,
                priority: modifier_priority(&self.zoom_in),
            },
            DispatchEntry {
                action: ShortcutAction::ZoomOut,
                priority: modifier_priority(&self.zoom_out),
            },
            DispatchEntry {
                action: ShortcutAction::UiZoomIn,
                priority: modifier_priority(&self.ui_zoom_in),
            },
            DispatchEntry {
                action: ShortcutAction::UiZoomOut,
                priority: modifier_priority(&self.ui_zoom_out),
            },
            DispatchEntry {
                action: ShortcutAction::ScrollUp,
                priority: modifier_priority(&self.scroll_up),
            },
            DispatchEntry {
                action: ShortcutAction::ScrollDown,
                priority: modifier_priority(&self.scroll_down),
            },
            DispatchEntry {
                action: ShortcutAction::DeleteSelected,
                priority: modifier_priority(&self.delete_selected),
            },
            DispatchEntry {
                action: ShortcutAction::MarkerAdd,
                priority: modifier_priority(&self.marker_add),
            },
            DispatchEntry {
                action: ShortcutAction::ToggleMenu,
                priority: modifier_priority(&self.toggle_menu),
            },
            DispatchEntry {
                action: ShortcutAction::ShowCommandPrompt,
                priority: modifier_priority(&self.show_command_prompt),
            },
            DispatchEntry {
                action: ShortcutAction::RenameItem,
                priority: modifier_priority(&self.rename_item),
            },
            DispatchEntry {
                action: ShortcutAction::DividerAdd,
                priority: modifier_priority(&self.divider_add),
            },
            DispatchEntry {
                action: ShortcutAction::ZoomToFit,
                priority: modifier_priority(&self.zoom_to_fit),
            },
            DispatchEntry {
                action: ShortcutAction::ZoomToCursor,
                priority: modifier_priority(&self.zoom_to_cursor),
            },
            DispatchEntry {
                action: ShortcutAction::GoToTime,
                priority: modifier_priority(&self.go_to_time),
            },
            DispatchEntry {
                action: ShortcutAction::FocusVariableNameFilter,
                priority: modifier_priority(&self.focus_variable_name_filter),
            },
            DispatchEntry {
                action: ShortcutAction::TileSplitRight,
                priority: modifier_priority(&self.tile_split_right),
            },
            DispatchEntry {
                action: ShortcutAction::TileSplitDown,
                priority: modifier_priority(&self.tile_split_down),
            },
            DispatchEntry {
                action: ShortcutAction::TileClose,
                priority: modifier_priority(&self.tile_close),
            },
            DispatchEntry {
                action: ShortcutAction::TileNext,
                priority: modifier_priority(&self.tile_next),
            },
            DispatchEntry {
                action: ShortcutAction::TilePrev,
                priority: modifier_priority(&self.tile_prev),
            },
            DispatchEntry {
                action: ShortcutAction::TileFocusLeft,
                priority: modifier_priority(&self.tile_focus_left),
            },
            DispatchEntry {
                action: ShortcutAction::TileFocusRight,
                priority: modifier_priority(&self.tile_focus_right),
            },
            DispatchEntry {
                action: ShortcutAction::TileFocusUp,
                priority: modifier_priority(&self.tile_focus_up),
            },
            DispatchEntry {
                action: ShortcutAction::TileFocusDown,
                priority: modifier_priority(&self.tile_focus_down),
            },
            DispatchEntry {
                action: ShortcutAction::TileMoveLeft,
                priority: modifier_priority(&self.tile_move_left),
            },
            DispatchEntry {
                action: ShortcutAction::TileMoveRight,
                priority: modifier_priority(&self.tile_move_right),
            },
            DispatchEntry {
                action: ShortcutAction::TileMoveUp,
                priority: modifier_priority(&self.tile_move_up),
            },
            DispatchEntry {
                action: ShortcutAction::TileMoveDown,
                priority: modifier_priority(&self.tile_move_down),
            },
            DispatchEntry {
                action: ShortcutAction::ShowLogs,
                priority: modifier_priority(&self.show_logs),
            },
        ]);

        debug_assert!(
            dispatch_table.len() == NUMBER_OF_SHORTCUTS,
            "Dispatch table length does not match the number of defined shortcut actions. Update the NUMBER_OF_SHORTCUTS constant accordingly."
        );

        // Sort by modifier priority (lower number = higher priority)
        dispatch_table.sort_by_key(|entry| entry.priority);
        dispatch_table
    }

    /// Get the keyboard shortcuts for a given action.
    fn shortcuts_for_action(&self, action: ShortcutAction) -> &[KeyboardShortcut] {
        match action {
            ShortcutAction::OpenFile => &self.open_file,
            ShortcutAction::SwitchFile => &self.switch_file,
            ShortcutAction::Undo => &self.undo,
            ShortcutAction::Redo => &self.redo,
            ShortcutAction::ToggleSidePanel => &self.toggle_side_panel,
            ShortcutAction::ToggleToolbar => &self.toggle_toolbar,
            ShortcutAction::GoToEnd => &self.goto_end,
            ShortcutAction::GoToStart => &self.goto_start,
            ShortcutAction::SaveStateFile => &self.save_state_file,
            ShortcutAction::GoToTop => &self.goto_top,
            ShortcutAction::GoToBottom => &self.goto_bottom,
            ShortcutAction::ItemFocus => &self.item_focus,
            ShortcutAction::GroupNew => &self.group_new,
            ShortcutAction::SelectAll => &self.select_all,
            ShortcutAction::SelectToggle => &self.select_toggle,
            ShortcutAction::ReloadWaveform => &self.reload_waveform,
            ShortcutAction::ZoomIn => &self.zoom_in,
            ShortcutAction::ZoomOut => &self.zoom_out,
            ShortcutAction::UiZoomIn => &self.ui_zoom_in,
            ShortcutAction::UiZoomOut => &self.ui_zoom_out,
            ShortcutAction::ScrollUp => &self.scroll_up,
            ShortcutAction::ScrollDown => &self.scroll_down,
            ShortcutAction::DeleteSelected => &self.delete_selected,
            ShortcutAction::MarkerAdd => &self.marker_add,
            ShortcutAction::ToggleMenu => &self.toggle_menu,
            ShortcutAction::ShowCommandPrompt => &self.show_command_prompt,
            ShortcutAction::RenameItem => &self.rename_item,
            ShortcutAction::DividerAdd => &self.divider_add,
            ShortcutAction::ZoomToFit => &self.zoom_to_fit,
            ShortcutAction::ZoomToCursor => &self.zoom_to_cursor,
            ShortcutAction::GoToTime => &self.go_to_time,
            ShortcutAction::FocusVariableNameFilter => &self.focus_variable_name_filter,
            ShortcutAction::TileSplitRight => &self.tile_split_right,
            ShortcutAction::TileSplitDown => &self.tile_split_down,
            ShortcutAction::TileClose => &self.tile_close,
            ShortcutAction::TileNext => &self.tile_next,
            ShortcutAction::TilePrev => &self.tile_prev,
            ShortcutAction::TileFocusLeft => &self.tile_focus_left,
            ShortcutAction::TileFocusRight => &self.tile_focus_right,
            ShortcutAction::TileFocusUp => &self.tile_focus_up,
            ShortcutAction::TileFocusDown => &self.tile_focus_down,
            ShortcutAction::TileMoveLeft => &self.tile_move_left,
            ShortcutAction::TileMoveRight => &self.tile_move_right,
            ShortcutAction::TileMoveUp => &self.tile_move_up,
            ShortcutAction::TileMoveDown => &self.tile_move_down,
            ShortcutAction::ShowLogs => &self.show_logs,
        }
    }

    /// Execute the action corresponding to the given shortcut.
    fn execute_action(&self, action: ShortcutAction, msgs: &mut Vec<Message>, state: &SystemState) {
        use crate::tiles::{
            TileTarget::Focused,
            commands::WorkspaceCommand,
            layout::{Direction, Placement},
        };
        match action {
            ShortcutAction::OpenFile => {
                msgs.push(Message::OpenFileDialog(crate::file_dialog::OpenMode::Open));
            }
            ShortcutAction::SwitchFile => {
                msgs.push(Message::OpenFileDialog(
                    crate::file_dialog::OpenMode::Switch,
                ));
            }
            ShortcutAction::Redo => {
                msgs.push(Message::Redo(state.get_count()));
            }
            ShortcutAction::Undo => {
                msgs.push(Message::Undo(state.get_count()));
            }
            ShortcutAction::ToggleSidePanel => {
                msgs.push(Message::SetSidePanelVisible(!state.show_hierarchy()));
            }
            ShortcutAction::ToggleToolbar => {
                msgs.push(Message::SetToolbarVisible(!state.show_toolbar()));
            }
            ShortcutAction::GoToEnd => {
                if let Some(tile_id) = state
                    .user
                    .workspace
                    .resolve_waveform(crate::tiles::TileTarget::Focused)
                {
                    msgs.push(Message::GoToEnd { tile_id });
                }
            }
            ShortcutAction::GoToStart => {
                if let Some(tile_id) = state
                    .user
                    .workspace
                    .resolve_waveform(crate::tiles::TileTarget::Focused)
                {
                    msgs.push(Message::GoToStart { tile_id });
                }
            }
            ShortcutAction::SaveStateFile => {
                msgs.push(Message::SaveStateFile(state.user.state_file.clone()));
            }
            ShortcutAction::GoToTop => {
                if let Some(target) = state
                    .user
                    .workspace
                    .resolve_waveform(crate::tiles::TileTarget::Focused)
                {
                    msgs.push(Message::ToTile(
                        target,
                        crate::tiles::kind::TileMessage::Waveform(
                            crate::tile_kinds::waveform::WaveformMessage::ScrollEdge { end: false },
                        ),
                    ));
                }
            }
            ShortcutAction::GoToBottom => {
                if let Some(waves) = state.user.waveform_read()
                    && waves.items.displayed_items.len() > 1
                {
                    msgs.push(Message::ToTile(
                        waves.tile_id,
                        crate::tiles::kind::TileMessage::Waveform(
                            crate::tile_kinds::waveform::WaveformMessage::ScrollEdge { end: true },
                        ),
                    ));
                }
            }
            ShortcutAction::GroupNew => {
                msgs.push(Message::GroupNew {
                    name: None,
                    before: None,
                    items: None,
                });
                msgs.push(Message::ShowCommandPrompt("item_rename ".to_owned(), None));
            }
            ShortcutAction::ItemFocus => {
                msgs.push(Message::ShowCommandPrompt("item_focus ".to_string(), None));
            }
            ShortcutAction::SelectAll => {
                msgs.push(Message::ItemSelectAll);
            }
            ShortcutAction::SelectToggle => {
                msgs.push(Message::ToggleItemSelected(None));
            }
            ShortcutAction::ReloadWaveform => {
                msgs.push(Message::ReloadWaveform(
                    state.user.config.behavior.keep_during_reload,
                ));
            }
            ShortcutAction::ZoomIn => {
                if let Some(tile_id) = state
                    .user
                    .workspace
                    .resolve_waveform(crate::tiles::TileTarget::Focused)
                {
                    msgs.push(Message::CanvasZoom {
                        mouse_ptr: None,
                        delta: 0.5,
                        tile_id,
                    });
                }
            }
            ShortcutAction::ZoomOut => {
                if let Some(tile_id) = state
                    .user
                    .workspace
                    .resolve_waveform(crate::tiles::TileTarget::Focused)
                {
                    msgs.push(Message::CanvasZoom {
                        mouse_ptr: None,
                        delta: 2.0,
                        tile_id,
                    });
                }
            }
            ShortcutAction::UiZoomIn => {
                let mut next_factor = f32::INFINITY;
                for factor in &state.user.config.layout.zoom_factors {
                    if *factor > state.ui_zoom_factor() && *factor < next_factor {
                        next_factor = *factor;
                    }
                }
                if next_factor != f32::INFINITY {
                    msgs.push(Message::SetUIZoomFactor(next_factor));
                }
            }
            ShortcutAction::UiZoomOut => {
                let mut next_factor = 0f32;
                for factor in &state.user.config.layout.zoom_factors {
                    if *factor < state.ui_zoom_factor() && *factor > next_factor {
                        next_factor = *factor;
                    }
                }
                if next_factor > 0f32 {
                    msgs.push(Message::SetUIZoomFactor(next_factor));
                }
            }
            ShortcutAction::ScrollUp => {
                if let Some(tile_id) = state
                    .user
                    .workspace
                    .resolve_waveform(crate::tiles::TileTarget::Focused)
                {
                    msgs.push(Message::CanvasScroll {
                        delta: Vec2 {
                            x: 0.,
                            y: -PER_SCROLL_EVENT * SCROLL_EVENTS_PER_PAGE,
                        },
                        tile_id,
                    });
                }
            }
            ShortcutAction::ScrollDown => {
                if let Some(tile_id) = state
                    .user
                    .workspace
                    .resolve_waveform(crate::tiles::TileTarget::Focused)
                {
                    msgs.push(Message::CanvasScroll {
                        delta: Vec2 {
                            x: 0.,
                            y: PER_SCROLL_EVENT * SCROLL_EVENTS_PER_PAGE,
                        },
                        tile_id,
                    });
                }
            }
            ShortcutAction::DeleteSelected => {
                msgs.push(Message::RemoveVisibleItems(MessageTarget::CurrentSelection));
            }
            ShortcutAction::MarkerAdd => {
                if let Some(waves) = state.user.waves.as_ref()
                    && let Some(cursor) = waves.cursor.as_ref()
                {
                    // Check if a marker already exists at the cursor position
                    let marker_exists = waves
                        .markers
                        .values()
                        .any(|marker_time| marker_time == cursor);
                    if !marker_exists {
                        msgs.push(Message::AddMarker {
                            time: cursor.clone(),
                            name: None,
                            move_focus: state.user.config.layout.move_focus_on_inserted_marker(),
                        });
                    }
                }
            }
            ShortcutAction::ToggleMenu => {
                msgs.push(Message::SetMenuVisible(!state.show_menu()));
            }
            ShortcutAction::ShowCommandPrompt => {
                msgs.push(Message::ShowCommandPrompt(String::new(), None));
            }
            ShortcutAction::RenameItem => {
                if let Some(waves) = state.user.waveform_read()
                    && waves.view.focused_index(waves.items).is_some()
                {
                    msgs.push(Message::ShowCommandPrompt("item_rename ".to_owned(), None));
                }
            }
            ShortcutAction::DividerAdd => {
                msgs.push(Message::AddDivider(None, None));
            }
            ShortcutAction::ZoomToFit => {
                if let Some(tile_id) = state
                    .user
                    .workspace
                    .resolve_waveform(crate::tiles::TileTarget::Focused)
                {
                    msgs.push(Message::ZoomToFit { tile_id });
                }
            }
            ShortcutAction::ZoomToCursor => {
                if let Some(tile_id) = state
                    .user
                    .workspace
                    .resolve_waveform(crate::tiles::TileTarget::Focused)
                {
                    msgs.push(Message::ZoomToCursor {
                        delta: 0.5,
                        tile_id,
                    });
                }
            }
            ShortcutAction::GoToTime => {
                msgs.push(Message::SetRequestTextEditFocus(
                    crate::toolbar::TOOLBAR_TIME_ID.to_string(),
                    true,
                ));
            }
            ShortcutAction::FocusVariableNameFilter => {
                msgs.push(Message::SetRequestTextEditFocus(
                    crate::variable_filter::VARIABLE_FILTER_ID.to_string(),
                    true,
                ));
            }
            ShortcutAction::TileSplitRight => {
                msgs.extend(
                    state.tile_command(|w| w.split_command(Focused, Direction::Right, false)),
                );
            }
            ShortcutAction::TileSplitDown => {
                msgs.extend(
                    state.tile_command(|w| w.split_command(Focused, Direction::Down, false)),
                );
            }
            ShortcutAction::TileClose => {
                msgs.extend(state.tile_command(|w| w.close_command(Focused)));
            }
            ShortcutAction::TileNext => {
                msgs.extend(state.tile_command(|w| w.cycle_tab_command(Focused, 1)));
            }
            ShortcutAction::TilePrev => {
                msgs.extend(state.tile_command(|w| w.cycle_tab_command(Focused, -1)));
            }
            ShortcutAction::TileFocusLeft => {
                msgs.extend(
                    state.tile_command(|w| w.focus_neighbor_command(Focused, Direction::Left)),
                );
            }
            ShortcutAction::TileFocusRight => {
                msgs.extend(
                    state.tile_command(|w| w.focus_neighbor_command(Focused, Direction::Right)),
                );
            }
            ShortcutAction::TileFocusUp => {
                msgs.extend(
                    state.tile_command(|w| w.focus_neighbor_command(Focused, Direction::Up)),
                );
            }
            ShortcutAction::TileFocusDown => {
                msgs.extend(
                    state.tile_command(|w| w.focus_neighbor_command(Focused, Direction::Down)),
                );
            }
            ShortcutAction::TileMoveLeft => {
                msgs.extend(state.tile_command(|w| w.move_command(Focused, Direction::Left)));
            }
            ShortcutAction::TileMoveRight => {
                msgs.extend(state.tile_command(|w| w.move_command(Focused, Direction::Right)));
            }
            ShortcutAction::TileMoveUp => {
                msgs.extend(state.tile_command(|w| w.move_command(Focused, Direction::Up)));
            }
            ShortcutAction::TileMoveDown => {
                msgs.extend(state.tile_command(|w| w.move_command(Focused, Direction::Down)));
            }
            ShortcutAction::ShowLogs => {
                msgs.push(Message::Workspace(WorkspaceCommand::OpenTile {
                    kind: "logs".into(),
                    placement: Placement::Edge(Direction::Down),
                    focus: true,
                }));
            }
        }
    }

    /// Process the keyboard shortcuts and execute the corresponding actions.
    pub fn process(&self, ctx: &egui::Context, msgs: &mut Vec<Message>, state: &SystemState) {
        // Execute actions matching pressed shortcuts using cached dispatch table
        for entry in &self.cached_dispatch_table {
            if self
                .shortcuts_for_action(entry.action)
                .iter()
                .any(|shortcut| ctx.input_mut(|i| i.consume_shortcut(shortcut)))
            {
                self.execute_action(entry.action, msgs, state);
            }
        }
    }
}

/// Determine the priority of the keyboard shortcuts based on their modifiers.
///
/// This is because egui's shortcut handling ignores modifiers to some extent,
/// so they must be checked in order of priority.
///
/// The egui design choice makes sense, since depending on keyboard you may not
/// be able to press certain keys without modifiers, so one would like those
/// symbols to match independent of modifiers. However, that leads to that the
/// more modifiers, the earlier the short cut must be checked.
fn modifier_priority(shortcuts: &[KeyboardShortcut]) -> u8 {
    shortcuts
        .iter()
        .find_map(|shortcut| {
            let has_shift = shortcut.modifiers.contains(Modifiers::SHIFT);
            let has_alt = shortcut.modifiers.contains(Modifiers::ALT);

            match (has_shift, has_alt) {
                (true, true) => Some(0), // Shift+Alt highest priority
                (_, true) => Some(1),    // Alt second priority
                (true, _) => Some(2),    // Shift third priority
                _ => None,
            }
        })
        .unwrap_or(3) // Rest lowest priority
}

// Custom serialization/deserialization for Vec<KeyboardShortcut>
mod keyboard_shortcuts_serde {
    use egui::Key;
    use serde::{Deserializer, Serializer};

    use super::{Deserialize, KeyboardShortcut, Modifiers, Result, Serialize};

    pub fn serialize<S>(shortcuts: &[KeyboardShortcut], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let bindings: Vec<String> = shortcuts
            .iter()
            .map(|s| format_binding(s.modifiers, s.logical_key))
            .collect();
        bindings.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<KeyboardShortcut>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bindings: Vec<String> = Vec::deserialize(deserializer)?;
        bindings
            .iter()
            .map(|s| parse_binding(s).map_err(serde::de::Error::custom))
            .collect()
    }

    fn format_binding(modifiers: Modifiers, logical_key: Key) -> String {
        const MODIFIER_NAMES: &[(Modifiers, &str)] = &[
            (Modifiers::CTRL, "Ctrl"),
            (Modifiers::SHIFT, "Shift"),
            (Modifiers::ALT, "Alt"),
            (Modifiers::MAC_CMD, "Mac_cmd"),
            (Modifiers::COMMAND, "Command"),
        ];

        // Pre-allocate with capacity for max 6 items (5 modifiers + key)
        let mut parts = Vec::with_capacity(6);

        for (modifier, name) in MODIFIER_NAMES {
            if modifiers.contains(*modifier) {
                parts.push(*name);
            }
        }
        let key_name = format!("{logical_key:?}");
        parts.push(&key_name);
        parts.join("+")
    }

    fn parse_binding(binding: &str) -> Result<KeyboardShortcut, String> {
        const MODIFIER_MAP: &[(&str, Modifiers)] = &[
            ("ctrl", Modifiers::CTRL),
            ("shift", Modifiers::SHIFT),
            ("alt", Modifiers::ALT),
            ("mac_cmd", Modifiers::MAC_CMD),
            ("command", Modifiers::COMMAND),
            ("cmd", Modifiers::COMMAND),
        ];

        let parts: Vec<&str> = binding.split('+').map(str::trim).collect();

        // Use slice pattern to extract key and modifiers
        let (modifier_parts, key_str) = match parts.as_slice() {
            [modifiers @ .., key] => (modifiers, *key),
            [] => return Err("Empty binding".to_string()),
        };

        let logical_key =
            Key::from_name(key_str).ok_or_else(|| format!("Unknown key: {key_str}"))?;

        // Use fold to accumulate modifiers
        let modifiers = modifier_parts
            .iter()
            .try_fold(Modifiers::NONE, |acc, &modifier_str| {
                let lower = modifier_str.to_lowercase();
                MODIFIER_MAP
                    .iter()
                    .find(|(name, _)| name == &lower)
                    .map(|(_, mod_bit)| acc | *mod_bit)
                    .ok_or_else(|| format!("Unknown modifier: {modifier_str}"))
            })?;

        Ok(KeyboardShortcut::new(modifiers, logical_key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiles::{
        TileId,
        commands::{SplitMode, WorkspaceCommand},
        layout::{Direction, Placement},
    };
    use egui::Key;

    fn press(state: &SystemState, key: Key, modifiers: Modifiers) -> Vec<Message> {
        let ctx = egui::Context::default();
        let mut msgs = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            },
            |ui| {
                state
                    .user
                    .config
                    .shortcuts
                    .process(ui.ctx(), &mut msgs, state)
            },
        );
        output.textures_delta.clear();
        msgs
    }

    fn create(state: &mut SystemState, kind: &str, placement: Placement) -> TileId {
        state
            .update(Message::Workspace(WorkspaceCommand::CreateTile {
                kind: kind.into(),
                placement,
                focus: true,
            }))
            .unwrap();
        state.user.workspace.layout().focused().unwrap()
    }

    #[test]
    fn tile_shortcuts_resolve_against_the_focused_tile() {
        let mut state = SystemState::new_default_config().unwrap();
        assert!(press(&state, Key::Backslash, Modifiers::COMMAND).is_empty());
        let waveform = create(&mut state, "waveform", Placement::Root);
        let logs = create(&mut state, "logs", Placement::TabAfter(waveform));
        state
            .update(Message::Workspace(WorkspaceCommand::FocusTile(waveform)))
            .unwrap();
        let msgs = press(&state, Key::Backslash, Modifiers::COMMAND);
        assert!(matches!(
            msgs.as_slice(),
            [Message::Workspace(WorkspaceCommand::SplitTile { tile, dir: Direction::Right, mode: SplitMode::Linked })] if *tile == waveform
        ));
        let msgs = press(
            &state,
            Key::Backslash,
            Modifiers::COMMAND | Modifiers::SHIFT,
        );
        assert!(matches!(
            msgs.as_slice(),
            [Message::Workspace(WorkspaceCommand::SplitTile { tile, dir: Direction::Down, .. })] if *tile == waveform
        ));
        assert!(matches!(
            press(&state, Key::W, Modifiers::COMMAND).as_slice(),
            [Message::Workspace(WorkspaceCommand::CloseTile(tile))] if *tile == waveform
        ));
        assert!(matches!(
            press(&state, Key::PageDown, Modifiers::COMMAND).as_slice(),
            [Message::Workspace(WorkspaceCommand::FocusTile(tile))] if *tile == logs
        ));
        assert!(matches!(
            press(&state, Key::PageUp, Modifiers::COMMAND).as_slice(),
            [Message::Workspace(WorkspaceCommand::FocusTile(tile))] if *tile == logs
        ));
        assert!(matches!(
            press(&state, Key::ArrowRight, Modifiers::COMMAND | Modifiers::ALT).as_slice(),
            [Message::Workspace(WorkspaceCommand::MoveTile { tile, to: Placement::Edge(Direction::Right) })] if *tile == waveform
        ));
        // No rendered geometry: spatial focus has no neighbor and stays silent.
        assert!(
            press(
                &state,
                Key::ArrowRight,
                Modifiers::COMMAND | Modifiers::SHIFT
            )
            .is_empty()
        );
        assert!(matches!(
            press(&state, Key::L, Modifiers::COMMAND | Modifiers::SHIFT).as_slice(),
            [Message::Workspace(WorkspaceCommand::OpenTile { kind, placement: Placement::Edge(Direction::Down), focus: true })] if kind == "logs"
        ));
        // Plain PageDown still scrolls the waveform rather than switching tabs.
        assert!(matches!(
            press(&state, Key::PageDown, Modifiers::NONE).as_slice(),
            [Message::CanvasScroll { tile_id, .. }] if *tile_id == waveform
        ));
        state
            .update(Message::Workspace(WorkspaceCommand::FocusTile(logs)))
            .unwrap();
        assert!(press(&state, Key::Backslash, Modifiers::COMMAND).is_empty());
        assert!(matches!(
            press(&state, Key::W, Modifiers::COMMAND).as_slice(),
            [Message::Workspace(WorkspaceCommand::CloseTile(tile))] if *tile == logs
        ));
    }
}
