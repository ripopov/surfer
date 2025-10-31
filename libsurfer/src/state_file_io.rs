use std::path::PathBuf;

#[cfg(not(target_arch = "wasm32"))]
use camino::Utf8PathBuf;
use eyre::Context;
use rfd::FileHandle;

#[cfg(not(target_arch = "wasm32"))]
use crate::async_util::perform_async_work;

use crate::{
    async_util::AsyncJob, message::Message, wave_source::STATE_FILE_EXTENSION, SystemState,
};

impl SystemState {
    #[cfg(target_arch = "wasm32")]
    pub fn load_state_file(&mut self, path: Option<PathBuf>) {
        if path.is_some() {
            return;
        }
        let message = move |bytes: Vec<u8>| match ron::de::from_bytes(&bytes)
            .context("Failed loading state file")
        {
            Ok(s) => vec![Message::LoadState(s, path)],
            Err(e) => {
                tracing::error!("Failed to load state: {e:#?}");
                vec![]
            }
        };
        self.file_dialog_open(
            "Load state",
            (
                format!("Surfer state files (*.{STATE_FILE_EXTENSION})"),
                vec![STATE_FILE_EXTENSION.to_string()],
            ),
            message,
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_state_file(&mut self, path: Option<PathBuf>) {
        let messages = move |path: PathBuf| {
            let source = Utf8PathBuf::from_path_buf(path.clone()).unwrap();
            if let Ok(bytes) = std::fs::read(source.clone()) {
                match ron::de::from_bytes(&bytes)
                    .context(format!("Failed loading {}", source.file_name().unwrap()))
                {
                    Ok(s) => vec![Message::LoadState(s, Some(path))],
                    Err(e) => {
                        tracing::error!("Failed to load state: {e:#?}");
                        vec![]
                    }
                }
            } else {
                tracing::error!("Failed to load state file: {path:#?}");
                vec![]
            }
        };
        if let Some(path) = path {
            let sender = self.channels.msg_sender.clone();
            for message in messages(path) {
                sender.send(message).unwrap();
            }
        } else {
            self.file_dialog_open(
                "Load state",
                (
                    format!("Surfer state files (*.{STATE_FILE_EXTENSION})"),
                    vec![STATE_FILE_EXTENSION.to_string()],
                ),
                messages,
            );
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn save_state_file(&mut self, path: Option<PathBuf>) {
        let Some(encoded) = self.encode_state() else {
            return;
        };

        let messages = async move |destination: FileHandle| {
            destination
                .write(encoded.as_bytes())
                .await
                .map_err(|e| tracing::error!("Failed to write state to {destination:#?} {e:#?}"))
                .ok();
            vec![
                Message::SetStateFile(destination.path().into()),
                Message::AsyncDone(AsyncJob::SaveState),
            ]
        };
        if let Some(path) = path {
            let sender = self.channels.msg_sender.clone();
            perform_async_work(async move {
                for message in messages(path.into()).await {
                    sender.send(message).unwrap();
                }
            });
        } else {
            self.file_dialog_save(
                "Save state",
                (
                    format!("Surfer state files (*.{STATE_FILE_EXTENSION})"),
                    ([STATE_FILE_EXTENSION.to_string()]).to_vec(),
                ),
                messages,
            );
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn save_state_file(&mut self, path: Option<PathBuf>) {
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
                .map_err(|e| tracing::error!("Failed to write state to {destination:#?} {e:#?}"))
                .ok();
            vec![Message::AsyncDone(AsyncJob::SaveState)]
        };
        self.file_dialog_save(
            "Save state",
            (
                format!("Surfer state files (*.{STATE_FILE_EXTENSION})"),
                ([STATE_FILE_EXTENSION.to_string()]).to_vec(),
            ),
            messages,
        );
    }

    fn encode_state(&self) -> Option<String> {
        let opt = ron::Options::default();

        opt.to_string_pretty(&self.user, ron::ser::PrettyConfig::default())
            .context("Failed to encode state")
            .map_err(|e| tracing::error!("Failed to encode state. {e:#?}"))
            .ok()
    }
}
