// tests for the surfer:remote submodule

use super::snapshot::{render_and_compare, wait_for_waves_fully_loaded};
use crate::SystemState;
use crate::message::Message;
use crate::source::SourceId;
use crate::transaction_container::TransactionStreamRef;
use crate::wave_container::{ScopeRef, ScopeRefExt};
use crate::wave_source::LoadOptions;
use ftr_parser::types::{GeneratorId, StreamId};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use surver::{SurverFileInfo, SurverFileKind};

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
        let _res = runtime.block_on(surver::surver_main(
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

#[test]
fn remote_konata_pipeline() {
    let port =
        BASE_PORT + UNIQUE_PORT_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst) as u16;
    let project_root: camino::Utf8PathBuf = project_root::get_project_root()
        .unwrap()
        .try_into()
        .unwrap();
    let filenames = vec![
        project_root
            .join("examples/kanata-sample-2.ftr")
            .to_string(),
    ];
    render_and_compare(&PathBuf::from("remote/remote_konata_pipeline"), || {
        let mut state = run_with_server(DEFAULT_IP, port, DEFAULT_TOKEN, &filenames, || {
            vec![Message::OpenKonataView {
                source: SourceId::default(),
                generator: TransactionStreamRef::new_gen(
                    StreamId(1),
                    GeneratorId(10),
                    "instruction".to_string(),
                ),
            }]
        });
        let started = std::time::Instant::now();
        while !state.konata_caches_ready() {
            state.handle_async_messages();
            std::thread::sleep(std::time::Duration::from_millis(1));
            assert!(
                started.elapsed() < std::time::Duration::from_secs(10),
                "remote Konata projection did not finish"
            );
        }
        let model = state
            .konata_runtime
            .values()
            .next()
            .and_then(|runtime| runtime.entry.as_ref())
            .and_then(|entry| entry.model())
            .expect("remote Konata model");
        assert_eq!(model.row_count(), 4_041);
        let tile_id = *state.user.konata_tiles.keys().next().unwrap();
        let parent_tx = model.rows.tx_id[96];
        let expected_events = model.stages_for_row_blocking(96).len();
        state.update(Message::OpenKonataEventTable {
            tile_id,
            parent_tx: Some(parent_tx),
        });
        while !state.table_caches_ready() {
            state.handle_async_messages();
            std::thread::yield_now();
        }
        let event_table = state
            .user
            .table_tiles
            .iter()
            .find_map(|(id, tile)| {
                matches!(
                    tile.spec,
                    crate::table::TableModelSpec::KonataEvents {
                        parent_tx: Some(candidate),
                        ..
                    } if candidate == parent_tx
                )
                .then_some(*id)
            })
            .unwrap();
        assert_eq!(
            state.table_runtime[&event_table]
                .cache
                .as_ref()
                .unwrap()
                .get()
                .unwrap()
                .row_ids
                .len(),
            expected_events
        );
        let remote_transactions = state
            .user
            .waves
            .as_ref()
            .unwrap()
            .transactions_for_source(SourceId::default())
            .unwrap();
        assert!(
            !remote_transactions
                .get_stream(StreamId(1))
                .unwrap()
                .transactions_loaded
        );
        assert!(
            remote_transactions
                .get_stream(StreamId(1))
                .unwrap()
                .generators
                .iter()
                .all(|generator| remote_transactions
                    .get_generator(*generator)
                    .unwrap()
                    .transactions
                    .is_empty())
        );
        state.update(Message::RemoveTableTile {
            tile_id: event_table,
        });
        let tile = state.user.konata_tiles.values_mut().next().unwrap();
        tile.title = "Konata — instruction (remote FTR)".to_string();
        tile.viewport.top_visible_row = 96.0;
        tile.viewport.set_left_tick(640);
        super::snapshot::show_only_analysis_tiles(&mut state);
        state
    });
}

#[test]
fn incompatible_surver_refuses_remote_ftr_without_full_download() {
    let mut state = SystemState::new_default_config().unwrap();
    state.user.surver_file_infos = Some(vec![SurverFileInfo {
        bytes: 1024,
        bytes_loaded: 1024,
        filename: "pipeline.ftr".to_string(),
        kind: SurverFileKind::Transaction,
        format: None,
        reloading: false,
        last_load_ok: true,
        last_modification_time: None,
    }]);
    assert!(state.user.surver_capabilities.transaction_pages.is_none());
    state.load_surver_file(
        "http://example.invalid/token".to_string(),
        0,
        LoadOptions::Clear,
    );
    assert!(state.progress_tracker.is_none());
    assert!(state.user.waves.is_none());
    assert!(state.user.show_logs);
}

snapshot_ui_remote!(
    example_fst_renders_with_multiple_files,
    [
        "examples/counter.vcd",
        "examples/counter2.vcd",
        "examples/many_sv_datatypes.fst",
        "examples/theme_demo.ghw"
    ],
    [
        Message::LoadSurverFileByIndex(Some(2), LoadOptions::Clear),
        Message::AddScope(ScopeRef::from_strs(&["TOP"]), true),
    ]
);

snapshot_ui_remote!(
    example_ghw_renders_with_multiple_files,
    [
        "examples/counter.vcd",
        "examples/counter2.vcd",
        "examples/theme_demo.ghw",
        "examples/many_sv_datatypes.fst"
    ],
    [
        Message::LoadSurverFileByIndex(Some(2), LoadOptions::Clear),
        Message::AddScope(ScopeRef::from_strs(&["theme_demo"]), false),
    ]
);

snapshot_ui_remote!(
    multiple_files_open_second,
    ["examples/counter.vcd", "examples/counter2.vcd"],
    [
        Message::LoadSurverFileByIndex(Some(1), LoadOptions::Clear),
        Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
        Message::AddScope(ScopeRef::from_strs(&["tb", "dut"]), false),
        Message::SetToolbarVisible(true),
    ]
);
