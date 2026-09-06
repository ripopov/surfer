//! Language-server integration for the source tile.
//!
//! `slang-server` (pinned under `ext/slang-server` in the VTR repository) is started for a
//! loaded design from the elaboration record of its VDB companion. It supplies what the
//! design database cannot: accurate token classification of every source file, symbol
//! resolution at any position, and the generate blocks an instance leaves uninstantiated.
//! The VDB and the recording supply the rest: which elaborated symbol is which recorded
//! signal, and its value at the cursor.

pub mod client;
pub mod launch;
pub mod protocol;
pub mod tokens;
pub mod transport;

pub use client::{Event, HoverInfo, Intent, Location, Phase, Resolver, SlangClient};
pub use launch::LaunchPlan;
pub use tokens::{LineTokens, Modifiers, Range, Span, TokenClass};

use camino::Utf8PathBuf;
use std::sync::Arc;

impl crate::SystemState {
    /// Starts a language server for the design described by `index`, replacing any
    /// session of a previous design. Failures are recorded for the source tile header
    /// and never prevent waveform viewing.
    pub(crate) fn start_slang(&mut self, index: Option<&crate::source_index::SourceIndex>) {
        self.slang = None;
        self.slang_error = None;
        let Some(index) = index else {
            return;
        };
        let Some(elaboration) = &index.database.elaboration else {
            self.slang_error = Some("VDB has no elaboration record".to_owned());
            return;
        };
        if !self.user.config.slang.autostart || cfg!(test) {
            // Tests replay recorded sessions (see `tests::slang`) instead of spawning.
            self.slang_error = Some("slang autostart disabled".to_owned());
            return;
        }
        let plan = LaunchPlan::from_elaboration(elaboration, &index.database.top, index.base());
        let scratch = Utf8PathBuf::from_path_buf(std::env::temp_dir().join("surfer-slang"))
            .unwrap_or_else(|_| Utf8PathBuf::from("surfer-slang"));
        let build_file = match plan.write_build_file(&scratch, &index.database.design_id) {
            Ok(path) => path,
            Err(error) => {
                self.slang_error = Some(format!("cannot write build file: {error}"));
                return;
            }
        };
        let binary = std::env::var("SURFER_SLANG_SERVER")
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| self.user.config.slang.server.clone());
        let workspace = plan.workspace.clone();
        let sender = self.channels.msg_sender.clone();
        match SlangClient::start(plan, build_file, sender, |deliver| {
            transport::ProcessTransport::spawn(&binary, workspace.as_str(), deliver)
                .map(|transport| Arc::new(transport) as Arc<dyn transport::Transport>)
        }) {
            Ok(client) => {
                tracing::info!(%binary, %workspace, "Started slang-server");
                self.slang = Some(client);
            }
            Err(error) => {
                tracing::warn!(%error, %binary, "Could not start slang-server");
                self.slang_error = Some(format!("cannot start {binary}: {error}"));
            }
        }
    }

    /// Installs an already started session, for tests that replay a recording.
    pub fn install_slang(&mut self, client: SlangClient) {
        self.slang = Some(client);
        self.slang_error = None;
    }
}
