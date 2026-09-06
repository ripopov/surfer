//! Language-server sessions for tests.
//!
//! Snapshot tests replay recordings of real `slang-server` sessions stored next to the
//! Verilator examples (`examples/verilator/<design>.slang.json`), so they need no C++
//! toolchain. The ignored `record_slang_sessions` test regenerates those recordings
//! against a real server; run it after changing the scenarios, the example designs or
//! the pinned server:
//!
//! ```sh
//! SURFER_SLANG_SERVER=/path/to/slang-server \
//!   cargo test -p libsurfer --lib record_slang_sessions -- --ignored
//! ```

use crate::slang::transport::{
    ProcessTransport, RecordingTransport, ReplayScript, ReplayTransport, Transport,
};
use crate::slang::{Intent, LaunchPlan, Location, SlangClient, TokenClass};
use crate::{Message, StartupParams, SystemState, WaveSource};
use camino::{Utf8Path, Utf8PathBuf};
use project_root::get_project_root;
use std::sync::Arc;

use super::snapshot::wait_for_waves_fully_loaded;

pub(crate) fn examples_dir() -> Utf8PathBuf {
    Utf8PathBuf::from_path_buf(get_project_root().unwrap())
        .unwrap()
        .join("examples/verilator")
}

pub(crate) fn source_file(name: &str) -> Utf8PathBuf {
    examples_dir().join(format!("{name}.sv"))
}

fn recording_path(name: &str) -> Utf8PathBuf {
    examples_dir().join(format!("{name}.slang.json"))
}

/// Loads a Verilator example without starting a real language server.
pub(crate) fn load_example(name: &str) -> SystemState {
    let mut state = SystemState::new_default_config().unwrap();
    state.user.config.slang.autostart = false;
    let mut state = state.with_params(StartupParams {
        waves: Some(WaveSource::File(source_file(name).with_extension("vtr"))),
        ..Default::default()
    });
    wait_for_waves_fully_loaded(&mut state, 10);
    state
}

fn launch_plan(state: &SystemState) -> (LaunchPlan, Utf8PathBuf) {
    let index = state
        .user
        .waves
        .as_ref()
        .and_then(|w| w.inner.as_waves())
        .and_then(|w| w.source_index())
        .expect("example has a VDB companion");
    let elaboration = index
        .database
        .elaboration
        .as_ref()
        .expect("example VDB has an elaboration record");
    let plan = LaunchPlan::from_elaboration(elaboration, &index.database.top, index.base());
    let scratch =
        Utf8PathBuf::from_path_buf(std::env::temp_dir().join("surfer-slang-tests")).unwrap();
    let build_file = plan
        .write_build_file(&scratch, &index.database.design_id)
        .unwrap();
    (plan, build_file)
}

fn placeholders(build_file: &Utf8Path) -> Vec<(String, String)> {
    vec![
        (build_file.to_string(), "${BUILD_FILE}".to_owned()),
        (examples_dir().to_string(), "${ROOT}".to_owned()),
    ]
}

/// Installs a replayed session for `name` into `state` and reports whether every
/// request of the frame loop found a recorded answer.
pub(crate) fn install_replay(state: &mut SystemState, name: &str) -> Arc<ReplayTransport> {
    let script: ReplayScript = serde_json::from_str(
        &std::fs::read_to_string(recording_path(name))
            .unwrap_or_else(|e| panic!("missing recording {}: {e}", recording_path(name))),
    )
    .unwrap();
    let (plan, build_file) = launch_plan(state);
    let placeholders = placeholders(&build_file);
    let mut handle = None;
    let client = SlangClient::start(
        plan,
        build_file,
        state.channels.msg_sender.clone(),
        |deliver| {
            let transport = Arc::new(ReplayTransport::new(script, placeholders, deliver));
            handle = Some(transport.clone());
            Ok(transport as Arc<dyn Transport>)
        },
    )
    .unwrap();
    state.install_slang(client);
    // Startup exchanges are answered synchronously; apply them now.
    state.handle_async_messages();
    handle.unwrap()
}

/// Zero-based line of the first line containing `needle`.
pub(crate) fn line_of(text: &str, needle: &str) -> u32 {
    text.lines()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} not found")) as u32
}

/// Location and class of the `occurrence`-th token spelled `token` on the line that
/// contains `needle`, from the server's semantic tokens.
pub(crate) fn token_at(
    state: &SystemState,
    file: &Utf8Path,
    needle: &str,
    token: &str,
    occurrence: usize,
) -> (Location, TokenClass) {
    let text = std::fs::read_to_string(file).unwrap();
    let line = line_of(&text, needle);
    let line_text = text.lines().nth(line as usize).unwrap();
    let tokens = state
        .slang
        .as_ref()
        .and_then(|client| client.tokens(file))
        .expect("semantic tokens available");
    let span = tokens
        .line(line)
        .iter()
        .filter(|span| &line_text[span.start as usize..span.end as usize] == token)
        .nth(occurrence)
        .copied()
        .unwrap_or_else(|| panic!("token {token:?} #{occurrence} not on line {}", line + 1));
    (
        Location {
            file: file.to_owned(),
            line,
            character: span.start,
        },
        span.class,
    )
}

/// One interaction of a recorded scenario.
pub(crate) enum Step {
    /// Open the file at a one-based line in an instance context.
    Open {
        file: &'static str,
        line: u32,
        instance: Option<&'static str>,
    },
    Instance(&'static str),
    Hover {
        needle: &'static str,
        token: &'static str,
        occurrence: usize,
    },
    Activate {
        needle: &'static str,
        token: &'static str,
        occurrence: usize,
        intent: Intent,
    },
}

/// The scenario recorded for each example. Snapshot tests replay prefixes of it.
pub(crate) fn scenario(name: &str) -> Vec<Step> {
    match name {
        "features" => vec![
            Step::Open {
                file: "features",
                line: 28,
                instance: Some("top.g_lane[1].u"),
            },
            Step::Instance("top.g_lane[0].u"),
            Step::Instance("top"),
            Step::Hover {
                needle: "count != LIMIT",
                token: "LIMIT",
                occurrence: 0,
            },
            Step::Hover {
                needle: "count != LIMIT",
                token: "count",
                occurrence: 0,
            },
            Step::Hover {
                needle: "else if (en) count <= count + 1'b1;",
                token: "count",
                occurrence: 0,
            },
            Step::Hover {
                needle: "2'd1: state <= RUN;",
                token: "state",
                occurrence: 0,
            },
            Step::Hover {
                needle: "2'd1: state <= RUN;",
                token: "RUN",
                occurrence: 0,
            },
            Step::Hover {
                needle: "return tag + 4'd1;",
                token: "tag",
                occurrence: 0,
            },
            Step::Hover {
                needle: "bus.valid = pkt.valid;",
                token: "valid",
                occurrence: 0,
            },
            Step::Hover {
                needle: "parameter bit WRAP",
                token: "WRAP",
                occurrence: 0,
            },
            Step::Hover {
                needle: "if (bus.valid) seen <= bus.data;",
                token: "data",
                occurrence: 0,
            },
            Step::Activate {
                needle: "else if (en) count <= count + 1'b1;",
                token: "count",
                occurrence: 0,
                intent: Intent::AddToWaveform,
            },
            Step::Activate {
                needle: "2'd1: state <= RUN;",
                token: "state",
                occurrence: 0,
                intent: Intent::AddToWaveform,
            },
            Step::Activate {
                needle: "counter #(.W(8), .WRAP(i == 0)) u(",
                token: "u",
                occurrence: 0,
                intent: Intent::Navigate,
            },
            Step::Activate {
                needle: "pkt.tag   = inc_tag(",
                token: "inc_tag",
                occurrence: 0,
                intent: Intent::Navigate,
            },
            Step::Activate {
                needle: "bus.data  = lane_count[1];",
                token: "data",
                occurrence: 0,
                intent: Intent::Navigate,
            },
            Step::Activate {
                needle: "counter #(.W(8), .WRAP(i == 0)) u(",
                token: "counter",
                occurrence: 0,
                intent: Intent::Navigate,
            },
        ],
        "pipeline" => vec![
            Step::Open {
                file: "pipeline",
                line: 2,
                instance: Some("top.u0"),
            },
            Step::Hover {
                needle: "else if (en) q <= d;",
                token: "q",
                occurrence: 0,
            },
        ],
        other => panic!("no scenario for {other}"),
    }
}

/// Applies scenario steps through the message bus, as the tile would.
pub(crate) fn apply_step(state: &mut SystemState, step: &Step) {
    match step {
        Step::Open {
            file,
            line,
            instance,
        } => {
            state.update(Message::OpenSource {
                file: source_file(file),
                line: *line,
                column: 1,
                instance: instance.map(str::to_owned),
            });
        }
        Step::Instance(instance) => {
            state.update(Message::SourceInstance(Some((*instance).to_owned())));
        }
        Step::Hover { .. } => {}
        Step::Activate {
            needle,
            token,
            occurrence,
            intent,
        } => {
            let file = current_file(state);
            let (at, class) = token_at(state, &file, needle, token, *occurrence);
            state.update(Message::SourceActivate {
                at,
                token: (*token).to_owned(),
                class,
                intent: *intent,
            });
        }
    }
}

pub(crate) fn current_file(state: &SystemState) -> Utf8PathBuf {
    state
        .user
        .workspace
        .tiles()
        .values()
        .find_map(|entry| match &entry.kind {
            crate::tiles::kind::TileKind::SourceCode(tile) => tile.file.clone(),
            _ => None,
        })
        .expect("source tile open")
}

pub(crate) fn current_instance(state: &SystemState) -> Option<String> {
    state
        .user
        .workspace
        .tiles()
        .values()
        .find_map(|entry| match &entry.kind {
            crate::tiles::kind::TileKind::SourceCode(tile) => Some(tile.instance.clone()),
            _ => None,
        })
        .flatten()
}

pub(crate) fn current_line(state: &SystemState) -> u32 {
    state
        .user
        .workspace
        .tiles()
        .values()
        .find_map(|entry| match &entry.kind {
            crate::tiles::kind::TileKind::SourceCode(tile) => Some(tile.line),
            _ => None,
        })
        .expect("source tile open")
}

/// Runs one frame of the tile so lazily issued requests go out, then applies replies.
pub(crate) fn settle(state: &mut SystemState, context: &egui::Context, frames: usize) {
    for _ in 0..frames {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 720.0),
            )),
            ..Default::default()
        };
        let mut output = context.run_ui(raw, |ui| {
            crate::setup_custom_font(ui.ctx());
            let messages = state.draw(ui, Some(egui::vec2(1280.0, 720.0)));
            for message in messages {
                state.update(message);
            }
        });
        output.textures_delta.clear();
        state.handle_async_messages();
    }
}

fn server_binary() -> Option<String> {
    if let Ok(path) = std::env::var("SURFER_SLANG_SERVER")
        && !path.is_empty()
    {
        return Some(path);
    }
    let root = Utf8PathBuf::from_path_buf(get_project_root().unwrap()).unwrap();
    [
        root.join("../slang-server/build/bin/slang-server"),
        root.join("../../bench/build/slang-server/install/bin/slang-server"),
    ]
    .into_iter()
    .find(|path| path.exists())
    .map(|path| path.to_string())
}

/// Waits for every outstanding request of a real server, routing replies through the
/// state so caches and follow-up messages behave as in the application.
fn drain(state: &mut SystemState) {
    let start = std::time::Instant::now();
    loop {
        state.handle_async_messages();
        let pending = state.slang.as_ref().map_or(0, SlangClient::pending_count);
        if pending == 0 {
            return;
        }
        assert!(
            start.elapsed().as_secs() < 60,
            "slang-server did not answer {pending} requests"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Background tasks of the state need a tokio runtime, as in the snapshot tests.
pub(crate) fn enter_runtime() -> tokio::runtime::EnterGuard<'static> {
    let runtime = Box::leak(Box::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap(),
    ));
    let guard = runtime.enter();
    let runtime: &'static tokio::runtime::Runtime = runtime;
    std::thread::spawn(move || {
        runtime.block_on(async {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
            }
        });
    });
    guard
}

#[test]
#[ignore = "needs a slang-server binary; regenerates examples/verilator/*.slang.json"]
fn record_slang_sessions() {
    let _runtime = enter_runtime();
    let binary = server_binary().expect("set SURFER_SLANG_SERVER to a slang-server binary");
    for name in ["features", "pipeline"] {
        let mut state = load_example(name);
        let (plan, build_file) = launch_plan(&state);
        let placeholders = placeholders(&build_file);
        let workspace = plan.workspace.to_string();
        let mut recorder: Option<Arc<RecordingTransport>> = None;
        let client = SlangClient::start(
            plan,
            build_file,
            state.channels.msg_sender.clone(),
            |deliver| {
                let binary = binary.clone();
                let workspace = workspace.clone();
                let mut spawn_error = None;
                let recording = Arc::new(RecordingTransport::new(placeholders, deliver, |inner| {
                    match ProcessTransport::spawn(&binary, &workspace, inner) {
                        Ok(transport) => Arc::new(transport) as Arc<dyn Transport>,
                        Err(error) => {
                            spawn_error = Some(error);
                            Arc::new(ReplayTransport::new(
                                ReplayScript::default(),
                                vec![],
                                Arc::new(|_| {}),
                            ))
                        }
                    }
                }));
                if let Some(error) = spawn_error {
                    return Err(error);
                }
                recorder = Some(recording.clone());
                Ok(recording as Arc<dyn Transport>)
            },
        )
        .expect("start slang-server");
        state.install_slang(client);
        let context = egui::Context::default();
        drain(&mut state);
        assert_eq!(
            state.slang.as_ref().unwrap().phase(),
            crate::slang::Phase::Ready,
            "server did not reach the ready phase"
        );
        for step in scenario(name) {
            match &step {
                Step::Hover {
                    needle,
                    token,
                    occurrence,
                } => {
                    settle(&mut state, &context, 2);
                    drain(&mut state);
                    let file = current_file(&state);
                    let (at, _) = token_at(&state, &file, needle, token, *occurrence);
                    state.slang.as_ref().unwrap().hover(&at);
                }
                _ => {
                    settle(&mut state, &context, 2);
                    drain(&mut state);
                    apply_step(&mut state, &step);
                }
            }
            settle(&mut state, &context, 2);
            drain(&mut state);
        }
        settle(&mut state, &context, 2);
        drain(&mut state);
        let script = recorder.unwrap().script();
        std::fs::write(
            recording_path(name),
            serde_json::to_string_pretty(&script).unwrap() + "\n",
        )
        .unwrap();
        eprintln!("recorded {} exchanges for {name}", script.exchanges.len());
    }
}
