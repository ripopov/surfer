use camino::Utf8PathBuf;
use eyre::WrapErr as _;
use rfd::FileHandle;
use tracing::error;

#[cfg(not(target_arch = "wasm32"))]
use crate::async_util::perform_async_work;
use crate::channels::{checked_send, checked_send_many};
use crate::file_dialog::FileFilter;
#[cfg(not(target_os = "macos"))]
use crate::file_dialog::STATE_FILE_FILTER;
#[cfg(any(
    target_os = "macos",
    all(target_arch = "wasm32", not(feature = "vscode"))
))]
use crate::file_dialog::STATE_FILE_FILTER_MACOS;
#[cfg(all(target_arch = "wasm32", feature = "vscode"))]
use crate::file_dialog::vscode_open_dialog_with_filter;

use crate::{
    SystemState,
    async_util::AsyncJob,
    message::Message,
    wave_source::{STATE_FILE_EXTENSION, WaveSource},
};

// JS bridge function defined in integration.js; used to post messages to the
// VS Code extension host (where `showSaveFilePicker` is not available).
#[cfg(all(target_arch = "wasm32", feature = "vscode"))]
#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    fn surfer_notify_host(message_json: &str);
}

/// Normalizes a suggested file stem into a safe, non-empty value.
///
/// Returns `fallback` when the input is blank or contains characters that
/// are broadly invalid in file names across supported platforms.
pub(crate) fn sanitize_file_stem<'a>(stem: &'a str, fallback: &'a str) -> &'a str {
    let trimmed = stem.trim_matches([' ', '.']);
    if trimmed.is_empty() {
        return fallback;
    }

    let has_illegal = trimmed
        .chars()
        .any(|c| matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'));

    if has_illegal { fallback } else { trimmed }
}

#[cfg(not(target_arch = "wasm32"))]
// Mac cannot handle multiple extensions in the file dialogs.
fn state_file_filter() -> &'static FileFilter {
    #[cfg(target_os = "macos")]
    {
        &STATE_FILE_FILTER_MACOS
    }
    #[cfg(not(target_os = "macos"))]
    {
        &STATE_FILE_FILTER
    }
}

#[cfg(all(target_arch = "wasm32", not(feature = "vscode")))]
// Mac cannot handle multiple extensions in the file dialogs.
fn state_file_filter() -> &'static FileFilter {
    if web_sys::window()
        .and_then(|w| w.navigator().platform().ok())
        .map(|p| p.starts_with("Mac"))
        .unwrap_or(false)
    {
        &STATE_FILE_FILTER_MACOS
    } else {
        &STATE_FILE_FILTER
    }
}

/// Extracts a display-friendly base name from a wave source.
///
/// For URLs, query and fragment parts are stripped before computing the stem.
pub(crate) fn source_file_stem(source: &WaveSource) -> Option<&str> {
    match source {
        WaveSource::File(path) | WaveSource::DragAndDrop(Some(path)) => path.file_stem(),
        WaveSource::Url(url) => {
            let trimmed = url.split(['?', '#']).next().unwrap_or(url.as_str());
            let filename = trimmed.rsplit('/').next()?;
            let stem = filename.rsplit_once('.').map_or(filename, |(head, _)| head);
            if stem.is_empty() { None } else { Some(stem) }
        }
        WaveSource::Data | WaveSource::DragAndDrop(None) | WaveSource::Cxxrtl(_) => None,
    }
}

impl SystemState {
    /// Builds the suggested state-file name used by save dialogs.
    ///
    /// Uses the loaded wave source stem when available and falls back to
    /// `surfer_state.surf.ron` semantics when no stable stem can be derived.
    fn default_state_file_name(&self) -> String {
        let stem = self
            .user
            .waves
            .as_ref()
            .and_then(|waves| source_file_stem(&waves.source))
            .map_or("surfer_state", |stem| {
                sanitize_file_stem(stem, "surfer_state")
            });

        format!("{stem}.{STATE_FILE_EXTENSION}")
    }

    #[cfg(all(target_arch = "wasm32", feature = "vscode"))]
    /// Opens a state file through the VS Code host bridge in wasm+vscode builds.
    pub(crate) fn load_state_file(&mut self, path: Option<Utf8PathBuf>) {
        if path.is_some() {
            return;
        }

        vscode_open_dialog_with_filter("state_file", &STATE_FILE_FILTER);
    }

    #[cfg(all(target_arch = "wasm32", not(feature = "vscode")))]
    /// Opens and decodes a state file in plain wasm/browser builds.
    pub(crate) fn load_state_file(&mut self, path: Option<Utf8PathBuf>) {
        if path.is_some() {
            return;
        }
        let message = move |bytes: Vec<u8>| match crate::tiles::serde::decode_bytes(&bytes)
            .context("Failed loading state file")
        {
            Ok(s) => vec![Message::LoadState(s, path)],
            Err(e) => {
                error!("Failed to load state: {e:#?}");
                vec![]
            }
        };
        self.file_dialog_open("Load state", state_file_filter(), message);
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Loads a state file from disk on native builds.
    ///
    /// When `path` is `None`, this opens a file picker and loads the selected file.
    pub(crate) fn load_state_file(&mut self, path: Option<Utf8PathBuf>) {
        let messages = move |source: Utf8PathBuf| match std::fs::read(source.as_std_path()) {
            Ok(bytes) => match crate::tiles::serde::decode_bytes(&bytes)
                .context(format!("Failed loading {}", source.as_str()))
            {
                Ok(s) => vec![Message::LoadState(s, Some(source))],
                Err(e) => {
                    error!("Failed to load state: {e:#?}");
                    vec![]
                }
            },
            Err(e) => {
                error!("Failed to load state file: {source:#?} {e:#?}");
                vec![]
            }
        };
        if let Some(path) = path {
            let sender = self.channels.msg_sender.clone();
            checked_send_many(&sender, messages(path));
        } else {
            self.file_dialog_open("Load state", state_file_filter(), messages);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Saves the current state to disk on native builds.
    ///
    /// When `path` is `None`, this opens a save dialog with a suggested filename.
    pub(crate) fn save_state_file(&mut self, path: Option<Utf8PathBuf>) {
        let Some(encoded) = self.encode_state() else {
            return;
        };

        let messages = async move |destination: FileHandle| {
            destination
                .write(encoded.as_bytes())
                .await
                .map_err(|e| error!("Failed to write state to {destination:#?} {e:#?}"))
                .ok();
            let state_file = match Utf8PathBuf::from_path_buf(destination.path().to_path_buf()) {
                Ok(p) => p,
                Err(p) => {
                    error!("File path '{}' contains invalid UTF-8", p.display());
                    return vec![Message::AsyncDone(AsyncJob::SaveState)];
                }
            };
            vec![
                Message::SetStateFile(state_file),
                Message::AsyncDone(AsyncJob::SaveState),
            ]
        };
        if let Some(path) = path {
            let sender = self.channels.msg_sender.clone();
            perform_async_work(async move {
                checked_send_many(&sender, messages(path.into_std_path_buf().into()).await);
            });
        } else {
            self.file_dialog_save(
                "Save state",
                state_file_filter(),
                Some(self.default_state_file_name()),
                messages,
            );
        }
    }

    #[cfg(all(target_arch = "wasm32", feature = "vscode"))]
    /// Saves state in wasm+vscode builds by sending it to the extension host.
    ///
    /// The webview cannot use `showSaveFilePicker`, so the host is responsible
    /// for showing the dialog and writing bytes.
    pub(crate) fn save_state_file(&mut self, _path: Option<Utf8PathBuf>) {
        let Some(encoded) = self.encode_state() else {
            return;
        };
        let file_name = self.default_state_file_name();

        // In the VS Code webview, `showSaveFilePicker` is not available.
        // Send the encoded state to the extension host via the JS bridge so
        // the host can show a native VS Code save dialog and write the file.
        let msg = serde_json::json!({
            "command": "vscodeSaveStateFromWasm",
            "data": encoded,
            "fileName": file_name,
        });
        surfer_notify_host(&msg.to_string());
    }

    #[cfg(all(target_arch = "wasm32", not(feature = "vscode")))]
    /// Saves state in plain wasm/browser builds via the browser save dialog.
    pub(crate) fn save_state_file(&mut self, path: Option<Utf8PathBuf>) {
        if path.is_some() {
            return;
        }
        let Some(encoded) = self.encode_state() else {
            return;
        };
        let messages = async move |destination: FileHandle| {
            destination
                .write(encoded.as_bytes())
                .await
                .map_err(|e| error!("Failed to write state to {destination:#?} {e:#?}"))
                .ok();
            vec![Message::AsyncDone(AsyncJob::SaveState)]
        };
        self.file_dialog_save(
            "Save state",
            state_file_filter(),
            Some(self.default_state_file_name()),
            messages,
        );
    }

    /// Serializes the current user state into pretty-printed RON.
    pub(crate) fn encode_state(&self) -> Option<String> {
        let opt = ron::Options::default();

        opt.to_string_pretty(&self.user, ron::ser::PrettyConfig::default())
            .context("Failed to encode state")
            .map_err(|e| error!("Failed to encode state. {e:#?}"))
            .ok()
    }

    /// Decodes RON bytes and enqueues a `LoadState` message on success.
    pub(crate) fn load_state_from_bytes(&mut self, bytes: &[u8]) {
        match crate::tiles::serde::decode_bytes(bytes).context("Failed loading state from bytes") {
            Ok(s) => {
                let sender = self.channels.msg_sender.clone();
                checked_send(&sender, Message::LoadState(s, None));
            }
            Err(e) => {
                error!("Failed to load state: {e:#?}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StartupParams;
    use crate::wave_source::WaveSource;

    #[test]
    fn legacy_state_moves_rows_views_and_pending_document_settings_into_workspace() {
        let encoded = include_str!("tiles/fixtures/legacy-state-v0.ron");
        let restored: crate::state::UserState = crate::tiles::serde::decode(encoded).unwrap();
        assert_eq!(restored.state_version, 1);
        assert!(
            restored
                .workspace
                .tiles()
                .values()
                .any(|tile| tile.kind.kind_name() == "logs")
        );
        assert!(
            restored
                .workspace
                .tiles()
                .values()
                .any(|tile| tile.kind.kind_name() == "annotation_list")
        );
        let order = restored.workspace.layout().tile_order();
        assert_eq!(order.len(), 5);
        assert!(
            restored
                .workspace
                .tiles()
                .values()
                .any(|tile| tile.kind.kind_name() == "transaction_details")
        );
        assert_eq!(restored.workspace.layout().focused(), Some(order[1]));
        assert_eq!(restored.workspace.item_lists().len(), 1);
        let (items, view) = restored.workspace.waveform_resources(order[1]).unwrap();
        assert_eq!(items.items_tree.len(), 1);
        assert_eq!(
            items.displayed_items.values().next().unwrap().name(),
            "saved row"
        );
        assert_eq!(view.viewport.curr_left, crate::viewport::Relative(0.25));
        assert_eq!(restored.waves.as_ref().unwrap().cursor, Some(45.into()));
        let native = ron::to_string(&restored).unwrap();
        assert!(native.contains("state_version:1"));
        let round_trip: crate::state::UserState = crate::tiles::serde::decode(&native).unwrap();
        assert_eq!(round_trip.workspace.layout().tile_order(), order);
        let mut state = SystemState::new_default_config().unwrap();
        state
            .update(Message::LoadState(Box::new(round_trip), None))
            .unwrap();
        assert!(state.user.waves.is_none());
        assert_eq!(
            state.user.previous_waves.as_ref().unwrap().cursor,
            Some(45.into())
        );
        assert_eq!(state.user.workspace.layout().tile_order(), order);
    }

    #[test]
    fn state_versions_are_checked_before_installing_or_discarding_presentation() {
        for invalid in ["(state_version: 2)", "(state_version: 1)"] {
            assert!(crate::tiles::serde::decode::<crate::state::UserState>(invalid).is_err());
        }
    }

    #[test]
    fn test_encode_state() {
        let state = SystemState::new_default_config()
            .unwrap()
            .with_params(StartupParams::default());
        let encoded = state.encode_state();
        assert!(encoded.is_some());
        let encoded = encoded.unwrap();
        assert!(encoded.contains("show_about"));
    }

    #[test]
    fn test_load_state_from_bytes() {
        let mut state = SystemState::new_default_config()
            .unwrap()
            .with_params(StartupParams::default());
        let encoded = state.encode_state().unwrap();
        let bytes = encoded.as_bytes();

        state.load_state_from_bytes(bytes);

        let msg = state.channels.msg_receiver.try_recv().unwrap();
        match msg {
            Message::LoadState(..) => {}
            _ => panic!("Expected LoadState message, got {:?}", msg),
        }
    }

    fn create_waveform(state: &mut SystemState) -> crate::tiles::TileId {
        use crate::tiles::{commands::WorkspaceCommand, layout::Placement};
        let placement = state
            .user
            .workspace
            .layout()
            .focused()
            .map_or(Placement::Root, Placement::TabAfter);
        state
            .update(Message::Workspace(WorkspaceCommand::CreateTile {
                kind: "waveform".into(),
                placement,
                focus: true,
            }))
            .unwrap();
        state.user.workspace.layout().focused().unwrap()
    }

    #[test]
    fn tile_commands_target_the_workspace_without_a_loaded_document() {
        use crate::{
            tile_kinds::waveform::WaveformMessage,
            tiles::{
                TileId,
                kind::{TileKind, TileMessage},
            },
        };
        let mut state = SystemState::new_default_config().unwrap();
        let first = create_waveform(&mut state);
        let second = create_waveform(&mut state);
        state
            .update(Message::ToTile(
                first,
                TileMessage::Waveform(WaveformMessage::Columns {
                    names: false,
                    values: false,
                }),
            ))
            .unwrap();
        assert!(
            state
                .update(Message::ToTile(
                    TileId(999),
                    TileMessage::Waveform(WaveformMessage::Columns {
                        names: false,
                        values: false
                    },)
                ))
                .is_none()
        );
        assert!(
            ron::from_str::<Message>(
                "ToTile(Focused, Waveform(Columns(names: false, values: true)))"
            )
            .is_err()
        );
        let command = ron::from_str(&format!(
            "ToTile({}, Waveform(Columns(names: false, values: true)))",
            ron::to_string(&second).unwrap()
        ))
        .unwrap();
        state.update(command).unwrap();
        let TileKind::Waveform(first_tile) = &state.user.workspace.tiles()[&first].kind else {
            panic!()
        };
        let TileKind::Waveform(second_tile) = &state.user.workspace.tiles()[&second].kind else {
            panic!()
        };
        assert!(!first_tile.show_name_column && !first_tile.show_value_column);
        assert!(!second_tile.show_name_column && second_tile.show_value_column);
        assert_eq!(state.user.workspace.layout().focused(), Some(second));
        assert!(state.user.waves.is_none());
    }

    #[test]
    fn application_state_round_trip_preserves_tabs_and_advances_session_identity() {
        use crate::tiles::commands::WorkspaceCommand;
        let mut state = SystemState::new_default_config().unwrap();
        let first = create_waveform(&mut state);
        let second = create_waveform(&mut state);
        state
            .update(Message::Workspace(WorkspaceCommand::RenameTile {
                tile: first,
                title: Some("Saved waveform".into()),
            }))
            .unwrap();
        let encoded = state.encode_state().unwrap();
        let saved = ron::to_string(&state.user.workspace).unwrap();
        let third = create_waveform(&mut state);
        let pending = state.workspace_runtime.request(first).unwrap();
        state.load_state_from_bytes(encoded.as_bytes());
        let message = state.channels.msg_receiver.try_recv().unwrap();
        state.update(message).unwrap();
        assert_eq!(ron::to_string(&state.user.workspace).unwrap(), saved);
        assert_eq!(state.user.workspace.layout().visible_tiles(), vec![second]);
        assert!(!state.workspace_runtime.accepts(pending, Some(pending)));
        assert!(create_waveform(&mut state).0 > third.0);
    }

    #[test]
    fn invalid_workspace_in_application_state_is_rejected_before_install() {
        let mut state = SystemState::new_default_config().unwrap();
        let first = create_waveform(&mut state);
        let pending = state.workspace_runtime.request(first).unwrap();
        let encoded = state.encode_state().unwrap();
        let invalid = encoded.replacen("version: 1", "version: 999", 1);
        assert_ne!(invalid, encoded);
        state.load_state_from_bytes(invalid.as_bytes());
        assert!(state.channels.msg_receiver.try_recv().is_err());
        assert_eq!(state.encode_state().unwrap(), encoded);
        assert!(state.workspace_runtime.accepts(pending, Some(pending)));
    }

    #[test]
    fn test_source_file_stem_from_file_and_url() {
        let file = WaveSource::File("examples/counter.vcd".into());
        assert_eq!(source_file_stem(&file), Some("counter"));

        let url = WaveSource::Url("https://example.com/some/path/demo.fst?x=1#top".to_string());
        assert_eq!(source_file_stem(&url), Some("demo"));
    }

    #[test]
    fn test_source_file_stem_url_without_filename() {
        let url = WaveSource::Url("https://example.com/some/path/".to_string());
        assert_eq!(source_file_stem(&url), None);
    }

    #[test]
    fn test_sanitize_file_stem() {
        assert_eq!(sanitize_file_stem("counter", "surfer_state"), "counter");
        assert_eq!(
            sanitize_file_stem("  counter.  ", "surfer_state"),
            "counter"
        );
        assert_eq!(sanitize_file_stem("", "surfer_state"), "surfer_state");
        assert_eq!(sanitize_file_stem("...", "surfer_state"), "surfer_state");
        assert_eq!(
            sanitize_file_stem("bad:name", "surfer_state"),
            "surfer_state"
        );
    }
}
