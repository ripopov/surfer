//! External access to the Surver server.
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[cfg(not(target_arch = "wasm32"))]
mod server;
#[cfg(not(target_arch = "wasm32"))]
pub use server::server_main;

pub const HTTP_SERVER_KEY: &str = "Server";
pub const HTTP_SERVER_VALUE_SURFER: &str = "Surfer";
pub const X_WELLEN_VERSION: &str = "x-wellen-version";
pub const X_SURFER_VERSION: &str = "x-surfer-version";
pub const SURFER_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const WELLEN_VERSION: &str = wellen::VERSION;

pub const WELLEN_SURFER_DEFAULT_OPTIONS: wellen::LoadOptions = wellen::LoadOptions {
    multi_thread: true,
    remove_scopes_with_empty_name: true,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct Status {
    pub bytes: u64,
    pub bytes_loaded: u64,
    pub filename: String,
    pub wellen_version: String,
    pub surfer_version: String,
    pub file_format: wellen::FileFormat,
}

pub struct BincodeOptions(OnceLock<bincode::DefaultOptions>);

impl BincodeOptions {
    fn get(&self) -> &bincode::DefaultOptions {
        self.0.get_or_init(|| bincode::DefaultOptions::new())
    }
}

impl std::ops::Deref for BincodeOptions {
    type Target = bincode::DefaultOptions;

    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

pub static BINCODE_OPTIONS: BincodeOptions = BincodeOptions(OnceLock::new());
