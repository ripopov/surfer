use egui::*;
use eyre::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

use crate::message::Message;
use crate::SystemState;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SurferShortcuts {
    #[serde(with = "keyboard_shortcuts_serde")]
    pub open: Vec<KeyboardShortcut>,
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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SurferShortcutsWindow {
    #[serde(skip)]
    pub status: String,
    #[serde(skip)]
    pub listening_for: Option<String>,
}

impl SurferShortcuts {
    #[cfg(target_arch = "wasm32")]
    pub fn new(_force_default_config: bool) -> Result<Self> {
        Self::new_from_toml(&include_str!("../../default_shortcuts.toml"))
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(force_default_config: bool) -> eyre::Result<Self> {
        use config::Config;
        use eyre::anyhow;

        let default_config = String::from(include_str!("../../default_shortcuts.toml"));
        let mut config = Config::builder().add_source(config::File::from_str(
            &default_config,
            config::FileFormat::Toml,
        ));
        let config = if !force_default_config {
            use config::{Environment, File};
            use directories::ProjectDirs;

            use crate::config::find_local_configs;

            if let Some(proj_dirs) = ProjectDirs::from("org", "surfer-project", "surfer") {
                let config_file = proj_dirs.config_dir().join("shortcuts.toml");
                config = config.add_source(File::from(config_file).required(false));
            }

            // `surfer.toml` will not be searched for upward, as it is deprecated.
            config = config.add_source(File::from(Path::new("surfer.toml")).required(false));

            // Add configs from most top-level to most local. This allows overwriting of
            // higher-level settings with a local `.surfer` directory.
            find_local_configs()
                .into_iter()
                .fold(config, |c, p| {
                    c.add_source(File::from(p.join("shortcuts.toml")).required(false))
                })
                .add_source(Environment::with_prefix("surfer")) // Add environment finally
        } else {
            config
        };

        config
            .build()?
            .try_deserialize()
            .map_err(|e| anyhow!("Failed to parse config {e}"))
    }

    pub fn new_from_toml(config: &str) -> Result<Self> {
        Ok(toml::from_str(config)?)
    }

    pub fn pressed(&self, ctx: &Context, shortcuts: &[KeyboardShortcut]) -> bool {
        shortcuts
            .iter()
            .any(|shortcut| ctx.input_mut(|i| i.consume_shortcut(shortcut)))
    }

    pub fn save(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let toml_string = toml::to_string_pretty(&self)?;
        fs::write(path, toml_string)?;
        Ok(())
    }

    pub fn update(&self, ctx: &Context, msgs: &mut Vec<Message>, state: &SystemState) {
        if self.pressed(ctx, &self.open) {
            log::info!("Open shortcut pressed");
        }
        if self.pressed(ctx, &self.redo) {
            msgs.push(Message::Redo(state.get_count()));
        }
        if self.pressed(ctx, &self.undo) {
            msgs.push(Message::Undo(state.get_count()));
        }
        if self.pressed(ctx, &self.toggle_side_panel) {
            msgs.push(Message::ToggleSidePanel);
        }
        if self.pressed(ctx, &self.toggle_toolbar) {
            msgs.push(Message::ToggleToolbar);
        }
        if self.pressed(ctx, &self.goto_end) {
            msgs.push(Message::GoToEnd { viewport_idx: 0 });
        }
        if self.pressed(ctx, &self.goto_end) {
            msgs.push(Message::GoToStart { viewport_idx: 0 });
        }
        if self.pressed(ctx, &self.save_state_file) {
            msgs.push(Message::SaveStateFile(state.user.state_file.clone()));
        }
        if self.pressed(ctx, &self.goto_top) {
            msgs.push(Message::ScrollToItem(0));
        }
        if self.pressed(ctx, &self.goto_bottom) {
            if let Some(waves) = &state.user.waves {
                if waves.displayed_items.len() > 1 {
                    msgs.push(Message::ScrollToItem(waves.displayed_items.len() - 1));
                }
            }
        }
    }
}

impl Default for SurferShortcuts {
    fn default() -> Self {
        Self::new(false).expect("Failed to load default config")
    }
}

// Custom serialization/deserialization for Vec<KeyboardShortcut>
mod keyboard_shortcuts_serde {
    use serde::{Deserializer, Serializer};

    use super::*;

    pub fn serialize<S>(shortcuts: &Vec<KeyboardShortcut>, serializer: S) -> Result<S::Ok, S::Error>
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
        let mut parts = Vec::new();

        if modifiers.contains(Modifiers::CTRL) {
            parts.push("Ctrl".to_string());
        }
        if modifiers.contains(Modifiers::SHIFT) {
            parts.push("Shift".to_string());
        }
        if modifiers.contains(Modifiers::ALT) {
            parts.push("Alt".to_string());
        }
        if modifiers.contains(Modifiers::MAC_CMD) {
            parts.push("Mac_cmd".to_string());
        }
        if modifiers.contains(Modifiers::COMMAND) {
            parts.push("Command".to_string());
        }

        let key_name = format!("{:?}", logical_key);
        parts.push(key_name);

        parts.join("+")
    }

    fn parse_binding(binding: &str) -> Result<KeyboardShortcut, String> {
        let parts: Vec<&str> = binding.split('+').map(|s| s.trim()).collect();

        if parts.is_empty() {
            return Err("Empty binding".to_string());
        }

        let key_str = parts[parts.len() - 1];
        let logical_key =
            Key::from_name(key_str).ok_or_else(|| format!("Unknown key: {}", key_str))?;

        let mut modifiers = Modifiers::NONE;
        for modifier_str in &parts[..parts.len() - 1] {
            match modifier_str.to_lowercase().as_str() {
                "ctrl" => modifiers |= Modifiers::CTRL,
                "shift" => modifiers |= Modifiers::SHIFT,
                "alt" => modifiers |= Modifiers::ALT,
                "mac_cmd" => modifiers |= Modifiers::MAC_CMD,
                "command" | "cmd" => modifiers |= Modifiers::COMMAND,
                _ => return Err(format!("Unknown modifier: {}", modifier_str)),
            }
        }

        Ok(KeyboardShortcut::new(modifiers, logical_key))
    }
}
