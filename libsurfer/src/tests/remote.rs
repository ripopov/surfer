// tests for the surfer:remote submodule

use super::snapshot::{render_and_compare, wait_for_waves_fully_loaded};
use crate::SystemState;
use crate::message::Message;
use crate::wave_container::{ScopeRef, ScopeRefExt};
use crate::wave_source::LoadOptions;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

/// starts the remote server in a background thread
fn start_server(bind_address: &str, port: u16, token: &str, filenames: &[String]) -> String {
    let addr = format!("http://localhost:{port}/{token}");
    let token = Some(token.to_string());
    let filenames = filenames.to_vec();
    let bind_address = bind_address.to_string();
    let started = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let started_copy = started.clone();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let _res = runtime.block_on(surver::server_main(
            port,
            bind_address.to_string(),
            token,
            &filenames,
            Some(started_copy),
        ));
    });

    // wait for server to start
    while !started.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    addr
}

/// Starts a server to load the `filename`, then updates the Surfer state until the waveform
/// is loaded from the client and all custom messages have been processed. Returns the final state.
fn run_with_server(
    bind_address: &str,
    port: u16,
    token: &str,
    filenames: &[String],
    custom_messages: impl Fn() -> Vec<Message>,
) -> SystemState {
    // start server in a background thread
    let url = start_server(bind_address, port, token, filenames);
    // create state and add messages as batch commands
    let mut state = SystemState::new_default_config().unwrap();

    let msgs = vec![
        // connect to server
        Message::LoadWaveformFileFromUrl(url, LoadOptions::Clear),
        // hide GUI elements
        Message::SetMenuVisible(false),
        Message::SetSidePanelVisible(false),
        Message::SetToolbarVisible(false),
        Message::SetOverviewVisible(false),
    ];

    state.add_batch_messages(msgs);
    state.add_batch_messages(custom_messages());

    // update state until all batch commands have been processed
    wait_for_waves_fully_loaded(&mut state, 10);

    state
}

/// incremented for every test in order to create non-conflicting ports
static UNIQUE_PORT_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
const BASE_PORT: u16 = 8400;
const DEFAULT_TOKEN: &str = "1234567890";
const DEFAULT_IP: &str = "127.0.0.1";

macro_rules! snapshot_ui_remote {
    ($name:ident, $files:expr, $msgs:expr) => {
        #[test]
        fn $name() {
            let port_offset = UNIQUE_PORT_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let port = BASE_PORT + port_offset as u16;
            let bind_address = DEFAULT_IP;
            let token = DEFAULT_TOKEN;
            let project_root: camino::Utf8PathBuf = project_root::get_project_root()
                .unwrap()
                .try_into()
                .unwrap();
            let filenames = $files
                .iter()
                .map(|f| project_root.join(f).to_string())
                .collect::<Vec<_>>();
            let messages = || Vec::from($msgs);
            let mut test_name = "remote/".to_string();
            test_name.push_str(stringify!($name));

            render_and_compare(&PathBuf::from(&test_name), || {
                run_with_server(bind_address, port, token, &filenames, messages)
            })
        }
    };
}

// Actual Tests

snapshot_ui_remote!(
    example_vcd_renders,
    ["examples/counter.vcd"],
    [
        Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
        Message::AddScope(ScopeRef::from_strs(&["tb", "dut"]), false),
    ]
);

snapshot_ui_remote!(
    multiple_files_open_second,
    ["examples/counter.vcd", "examples/counter2.vcd"],
    [
        Message::LoadAndSetSurverFileIndex(Some(1), LoadOptions::Clear),
        Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
        Message::AddScope(ScopeRef::from_strs(&["tb", "dut"]), false),
        Message::SetToolbarVisible(true),
    ]
);
