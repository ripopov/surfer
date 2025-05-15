use std::path::PathBuf;

use color_eyre::eyre::Context;
use log::error;
use ron::ser::PrettyConfig;

use crate::{
    file_dialog::{load_state_dialog, save_state_dialog},
    message::{AsyncJob, Message},
    wasm_util::perform_async_work,
    SystemState,
};

impl SystemState {
    #[cfg(target_arch = "wasm32")]
    pub fn load_state_file(&mut self, _path: Option<PathBuf>) {}

    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_state_file(&mut self, path: Option<PathBuf>) {
        let sender = self.channels.msg_sender.clone();

        perform_async_work(async move {
            let source = if let Some(path) = path.clone() {
                Some(path.into())
            } else {
                load_state_dialog().await
            };
            let Some(source) = source else {
                return;
            };
            let bytes = source.read().await;
            let new_state = match ron::de::from_bytes(&bytes)
                .context(format!("Failed loading {}", source.file_name()))
            {
                Ok(s) => s,
                Err(e) => {
                    error!("Failed to load state: {e:#?}");
                    return;
                }
            };
            sender.send(Message::LoadState(new_state, path)).unwrap();
        });
    }

    #[cfg(target_arch = "wasm32")]
    pub fn save_state_file(&mut self, _path: Option<PathBuf>) {}

    #[cfg(not(target_arch = "wasm32"))]
    pub fn save_state_file(&mut self, path: Option<PathBuf>) {
        let sender = self.channels.msg_sender.clone();
        let Some(encoded) = self.encode_state() else {
            return;
        };

        perform_async_work(async move {
            let destination = if let Some(path) = path {
                Some(path.into())
            } else {
                save_state_dialog().await
            };
            let Some(destination) = destination else {
                return;
            };

            sender
                .send(Message::SetStateFile(destination.path().into()))
                .unwrap();
            destination
                .write(encoded.as_bytes())
                .await
                .map_err(|e| error!("Failed to write state to {destination:#?} {e:#?}"))
                .ok();
            sender
                .send(Message::AsyncDone(AsyncJob::SaveState))
                .unwrap();
        });
    }

    fn encode_state(&self) -> Option<String> {
        let opt = ron::Options::default();

        opt.to_string_pretty(&self.user, PrettyConfig::default())
            .context("Failed to encode state")
            .map_err(|e| error!("Failed to encode state. {e:#?}"))
            .ok()
    }
}
