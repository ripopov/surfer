use std::future::Future;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

#[cfg(not(target_arch = "wasm32"))]
use camino::Utf8PathBuf;
use rfd::{AsyncFileDialog, FileHandle};
use serde::Deserialize;
#[cfg(all(target_arch = "wasm32", feature = "vscode"))]
use wasm_bindgen::prelude::*;

use crate::SystemState;
use crate::async_util::perform_async_work;
use crate::channels::checked_send_many;
use crate::message::Message;
use crate::transactions::TRANSACTIONS_FILE_EXTENSION;
use crate::wave_source::LoadOptions;

// JS entry points that must be provided by the VS Code extension's webview setup
// (e.g. inside the SURFER_SETUP_HOOKS block or integration.js).
//
// `vscode_show_open_dialog(kind, filters_json)` – asks the extension host to show
//   a native open-file picker.  `kind` is an opaque tag the host echoes back in
//   the inject_message it fires once the user confirms:
//
//   | kind                       | injected Message                               |
//   |----------------------------|------------------------------------------------|
//   | `"waveform_clear"`         | `LoadWaveformFileFromUrl(url, Clear)`          |
//   | `"waveform_keep_available"`| `LoadWaveformFileFromUrl(url, KeepAvailable)`  |
//   | `"waveform_keep_all"`      | `LoadWaveformFileFromUrl(url, KeepAll)`        |
//   | `"command_file"`           | `LoadCommandFileFromUrl(url)`                  |
//   | "state_file"             | `LoadStateFromData(bytes)`                     |
//
//   `filters_json` is a JSON array of `{"name":str,"extensions":[str]}` objects.
//
#[cfg(all(target_arch = "wasm32", feature = "vscode"))]
#[wasm_bindgen]
extern "C" {
    fn vscode_show_open_dialog(kind: &str, filters_json: &str);
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub enum OpenMode {
    Open,
    Switch,
    AddSource,
    OpenMultipleSources,
}

impl SystemState {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn file_dialog_open<F>(
        &mut self,
        title: &'static str,
        filter: (String, Vec<String>),
        messages: F,
    ) where
        F: FnOnce(PathBuf) -> Vec<Message> + Send + 'static,
    {
        let sender = self.channels.msg_sender.clone();

        perform_async_work(async move {
            if let Some(file) = create_file_dialog(filter, title).pick_file().await {
                checked_send_many(&sender, messages(file.path().to_path_buf()));
            }
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn file_dialog_open_many<F>(
        &mut self,
        title: &'static str,
        filter: (String, Vec<String>),
        messages: F,
    ) where
        F: FnOnce(Vec<PathBuf>) -> Vec<Message> + Send + 'static,
    {
        let sender = self.channels.msg_sender.clone();

        perform_async_work(async move {
            if let Some(files) = create_file_dialog(filter, title).pick_files().await {
                checked_send_many(
                    &sender,
                    messages(
                        files
                            .into_iter()
                            .map(|file| file.path().to_path_buf())
                            .collect(),
                    ),
                );
            }
        });
    }

    #[cfg(all(target_arch = "wasm32", not(feature = "vscode")))]
    pub(crate) fn file_dialog_open<F>(
        &mut self,
        title: &'static str,
        filter: (String, Vec<String>),
        messages: F,
    ) where
        F: FnOnce(Vec<u8>) -> Vec<Message> + 'static,
    {
        let sender = self.channels.msg_sender.clone();

        perform_async_work(async move {
            if let Some(file) = create_file_dialog(filter, title).pick_file().await {
                checked_send_many(&sender, messages(file.read().await));
            }
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn file_dialog_save<F, Fut>(
        &mut self,
        title: &'static str,
        filter: (String, Vec<String>),
        default_file_name: Option<String>,
        messages: F,
    ) where
        F: FnOnce(FileHandle) -> Fut + Send + 'static,
        Fut: Future<Output = Vec<Message>> + Send + 'static,
    {
        let sender = self.channels.msg_sender.clone();

        perform_async_work(async move {
            let mut dialog = create_file_dialog(filter, title);
            if let Some(file_name) = default_file_name {
                dialog = dialog.set_file_name(&file_name);
            }
            if let Some(file) = dialog.save_file().await {
                checked_send_many(&sender, messages(file).await);
            }
        });
    }

    #[cfg(all(target_arch = "wasm32", not(feature = "vscode")))]
    pub(crate) fn file_dialog_save<F, Fut>(
        &mut self,
        title: &'static str,
        filter: (String, Vec<String>),
        default_file_name: Option<String>,
        messages: F,
    ) where
        F: FnOnce(FileHandle) -> Fut + 'static,
        Fut: Future<Output = Vec<Message>> + 'static,
    {
        let sender = self.channels.msg_sender.clone();

        perform_async_work(async move {
            let mut dialog = create_file_dialog(filter, title);
            if let Some(file_name) = default_file_name {
                dialog = dialog.set_file_name(&file_name);
            }
            if let Some(file) = dialog.save_file().await {
                checked_send_many(&sender, messages(file).await);
            }
        });
    }

    pub(crate) fn open_file_dialog(&mut self, mode: OpenMode) {
        let filter = (
            "Waveform/Transaction-files (*.vcd, *.fst, *.ghw, *.ftr)".to_string(),
            vec![
                "vcd".to_string(),
                "fst".to_string(),
                "ghw".to_string(),
                TRANSACTIONS_FILE_EXTENSION.to_string(),
            ],
        );

        match mode {
            OpenMode::Open | OpenMode::Switch => {
                let load_options: LoadOptions =
                    (mode, self.user.config.behavior.keep_during_reload).into();

                #[cfg(all(target_arch = "wasm32", feature = "vscode"))]
                {
                    let kind = match load_options {
                        LoadOptions::Clear => "waveform_clear",
                        LoadOptions::KeepAvailable => "waveform_keep_available",
                        LoadOptions::KeepAll => "waveform_keep_all",
                    };
                    vscode_open_dialog_with_filter(kind, &filter);
                }

                #[cfg(not(target_arch = "wasm32"))]
                let message = move |file: PathBuf| match Utf8PathBuf::from_path_buf(file.clone()) {
                    Ok(utf8_path) => vec![Message::LoadFile(utf8_path, load_options)],
                    Err(_) => {
                        vec![Message::Error(eyre::eyre!(
                            "File path '{}' contains invalid UTF-8",
                            file.display()
                        ))]
                    }
                };

                #[cfg(all(target_arch = "wasm32", not(feature = "vscode")))]
                let message = move |file: Vec<u8>| vec![Message::LoadFromData(file, load_options)];

                #[cfg(not(all(target_arch = "wasm32", feature = "vscode")))]
                self.file_dialog_open("Open waveform file", filter, message);
            }
            OpenMode::AddSource => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let message =
                        move |file: PathBuf| match Utf8PathBuf::from_path_buf(file.clone()) {
                            Ok(utf8_path) => {
                                vec![Message::LoadFileWithIntent(
                                    utf8_path,
                                    crate::wave_source::LoadIntent::AddSource,
                                )]
                            }
                            Err(_) => {
                                vec![Message::Error(eyre::eyre!(
                                    "File path '{}' contains invalid UTF-8",
                                    file.display()
                                ))]
                            }
                        };
                    self.file_dialog_open("Add source", filter, message);
                }

                #[cfg(target_arch = "wasm32")]
                self.update(Message::Error(eyre::eyre!(
                    "Add Source file dialog is not supported on this platform yet"
                )));
            }
            OpenMode::OpenMultipleSources => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let message = move |files: Vec<PathBuf>| {
                        let mut messages = Vec::new();
                        let mut load_requests = Vec::new();
                        for file in files {
                            match Utf8PathBuf::from_path_buf(file.clone()) {
                                Ok(utf8_path) => {
                                    let intent = if load_requests.is_empty() {
                                        crate::wave_source::LoadIntent::ReplaceSession
                                    } else {
                                        crate::wave_source::LoadIntent::AddSource
                                    };
                                    load_requests.push((utf8_path, intent));
                                }
                                Err(_) => messages.push(Message::Error(eyre::eyre!(
                                    "File path '{}' contains invalid UTF-8",
                                    file.display()
                                ))),
                            }
                        }
                        if !load_requests.is_empty() {
                            messages.push(Message::LoadFilesWithIntents(load_requests));
                        }
                        messages
                    };
                    self.file_dialog_open_many("Open waveform + transactions", filter, message);
                }

                #[cfg(target_arch = "wasm32")]
                self.update(Message::Error(eyre::eyre!(
                    "Multi-source file dialog is not supported on this platform yet"
                )));
            }
        }
    }

    pub(crate) fn open_command_file_dialog(&mut self) {
        let filter = (
            "Command-file (*.sucl)".to_string(),
            vec!["sucl".to_string()],
        );

        #[cfg(all(target_arch = "wasm32", feature = "vscode"))]
        {
            vscode_open_dialog_with_filter("command_file", &filter);
        }

        #[cfg(not(target_arch = "wasm32"))]
        let message = move |file: PathBuf| match Utf8PathBuf::from_path_buf(file.clone()) {
            Ok(utf8_path) => vec![Message::LoadCommandFile(utf8_path)],
            Err(_) => {
                vec![Message::Error(eyre::eyre!(
                    "File path '{}' contains invalid UTF-8",
                    file.display()
                ))]
            }
        };

        #[cfg(all(target_arch = "wasm32", not(feature = "vscode")))]
        let message = move |file: Vec<u8>| vec![Message::LoadCommandFromData(file)];

        #[cfg(not(all(target_arch = "wasm32", feature = "vscode")))]
        self.file_dialog_open("Open command file", filter, message);
    }

    #[cfg(feature = "python")]
    pub(crate) fn open_python_file_dialog(&mut self) {
        self.file_dialog_open(
            "Open Python translator file",
            ("Python files (*.py)".to_string(), vec!["py".to_string()]),
            |file| match Utf8PathBuf::from_path_buf(file.clone()) {
                Ok(utf8_path) => vec![Message::LoadPythonTranslator(utf8_path)],
                Err(_) => {
                    vec![Message::Error(eyre::eyre!(
                        "File path '{}' contains invalid UTF-8",
                        file.display()
                    ))]
                }
            },
        );
    }
}

#[cfg(not(all(target_arch = "wasm32", feature = "vscode")))]
#[cfg(not(target_os = "macos"))]
fn create_file_dialog(filter: (String, Vec<String>), title: &'static str) -> AsyncFileDialog {
    AsyncFileDialog::new()
        .set_title(title)
        .add_filter(filter.0, &filter.1)
        .add_filter("All files", &["*"])
}

#[cfg(not(all(target_arch = "wasm32", feature = "vscode")))]
#[cfg(target_os = "macos")]
fn create_file_dialog(filter: (String, Vec<String>), title: &'static str) -> AsyncFileDialog {
    AsyncFileDialog::new()
        .set_title(title)
        .add_filter(filter.0, &filter.1)
}

/// Serialise a `(name, extensions)` filter pair into the JSON array expected by
/// `vscode_show_open_dialog`.
///
/// Example output: `[{"name":"Waveform files","extensions":["vcd","fst"]}]`
#[cfg(all(target_arch = "wasm32", feature = "vscode"))]
pub(crate) fn vscode_open_dialog_with_filter(kind: &str, filter: &(String, Vec<String>)) {
    let filters_json = filters_to_json(filter);
    vscode_show_open_dialog(kind, &filters_json);
}

#[cfg(all(target_arch = "wasm32", feature = "vscode"))]
fn filters_to_json(filter: &(String, Vec<String>)) -> String {
    let exts = filter
        .1
        .iter()
        .map(|e| format!("{e:?}"))
        .collect::<Vec<_>>()
        .join(",");
    format!("[{{\"name\":{:?},\"extensions\":[{exts}]}}]", filter.0)
}
