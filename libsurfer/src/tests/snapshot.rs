use crate::tiles::{
    TileId,
    commands::{DocumentCommand, SplitMode, WorkspaceCommand},
    layout::Direction as TileDirection,
};
use std::{env, fs::File, io::IsTerminal};

use crate::{arrow::WavePoint, graphics::Anchor};
use base64::{Engine, engine::general_purpose};
use camino::{Utf8Path, Utf8PathBuf};
use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect};
use egui_skia_renderer::{EguiSkia, EncodedImageFormat, create_surface, draw_onto_surface};
use emath::Vec2;
use ftr_parser::types::{GeneratorId, StreamId, TransactionId};
use image::{DynamicImage, ImageFormat};
use num::{BigInt, bigint::ToBigInt};
use project_root::get_project_root;
use test_log::test;
use tracing::info;

use crate::{
    Message, MoveDir, StartupParams, SystemState, WaveSource,
    async_util::AsyncJob,
    clock_highlighting::ClockHighlightType,
    config::{FocusHighlight, SurferConfig, TransitionValue},
    displayed_item::{DisplayedFieldRef, DisplayedItemRef},
    displayed_item_tree::VisibleItemIndex,
    graphics::{Direction, GrPoint, Graphic, GraphicId, GraphicsY},
    hierarchy::{HierarchyStyle, ParameterDisplayLocation, ScopeExpandType},
    message::MessageTarget,
    setup_custom_font,
    state::UserState,
    trace_style::TraceStyle,
    transaction_container::{StreamScopeRef, TransactionRef, TransactionStreamRef},
    variable_filter::{VariableIOFilterType, VariableNameFilterType},
    variable_name_type::VariableNameType,
    wave_container::{ScopeRef, ScopeRefExt, VariableRef, VariableRefExt},
    wave_data::ScopeType,
    wave_source::{LoadOptions, STATE_FILE_EXTENSION},
};

/// Default snapshot size
const SNAPSHOT_WIDTH: f32 = 1280.0;
const SNAPSHOT_HEIGHT: f32 = 720.0;
const SNAPSHOT_SIZE: Vec2 = Vec2::new(SNAPSHOT_WIDTH, SNAPSHOT_HEIGHT);

fn print_image(img: &DynamicImage) {
    if std::io::stdout().is_terminal() {
        let mut bytes = vec![];
        img.write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        let b64 = general_purpose::STANDARD.encode(&bytes);
        println!(
            "\x1b]1337;File=size={size};width=auto;height=auto;inline=1:{b64}\x1b]\x1b[1E",
            size = bytes.len()
        );
    }
}

/// Compare a rendered image against the stored snapshot, writing a new snapshot
/// and diff if they differ.  Shared by [`render_and_compare_inner`] and tests
/// that need custom rendering (e.g. injecting pointer events to open menus).
fn compare_with_snapshot(filename: &Utf8Path, new: &DynamicImage) {
    let root = get_project_root().expect("Failed to get root");
    let previous_image_file = root.join("snapshots").join(filename).with_extension("png");

    let (write_new_file, diff) = if previous_image_file.exists() {
        let prev = image::open(previous_image_file.clone()).unwrap_or_else(|_| {
            panic!("Failed to load previous image from {previous_image_file:?}")
        });
        let result =
            image_compare::rgb_hybrid_compare(&new.clone().into_rgb8(), &prev.clone().into_rgb8())
                .expect("Comparison failing");
        let (score, map) = (result.score, result.image);
        (score <= 0.99999, Some((score, map)))
    } else {
        (true, None)
    };

    let new_file = root
        .join("snapshots")
        .join(filename)
        .with_extension("new.png");

    if new_file.exists() {
        std::fs::remove_file(&new_file).expect("Failed to remove existing snapshot file");
    }
    if write_new_file {
        std::fs::create_dir_all("snapshots").expect("Failed to create snapshots dir");
        new.write_to(
            &mut File::create(&new_file)
                .unwrap_or_else(|_| panic!("Failed to create {new_file:?}")),
            ImageFormat::Png,
        )
        .unwrap_or_else(|_| panic!("Failed to write new image to {new_file:?}"));
    }

    match (write_new_file, diff) {
        (true, Some((score, map))) => {
            let diff_img = map.to_color_map();
            let diff_file = root
                .join("snapshots")
                .join(filename)
                .with_extension("diff.png");
            diff_img
                .save(diff_file.clone())
                .unwrap_or_else(|_| panic!("Failed to save diff file to {diff_file:?}"));

            let prev = image::open(previous_image_file.clone()).unwrap_or_else(|_| {
                panic!("Failed to load previous image from {previous_image_file:?}")
            });
            println!("Previous: {previous_image_file:?}");
            print_image(&prev);
            println!("New: {new_file:?}");
            print_image(new);
            println!("Diff: {diff_file:?}");
            print_image(&diff_img);
            panic!(
                "Snapshot diff. Score: {score}\n\told: {previous_image_file:?}\n\tnew: {new_file:?}"
            )
        }
        (true, None) => {
            print_image(new);
            panic!("New snapshot image (saved to {new_file:?})")
        }
        (false, _) => {}
    }
}

pub(crate) fn render_and_compare_inner(
    filename: &Utf8Path,
    state: impl Fn() -> SystemState,
    size: Vec2,
    feathering: bool,
    threshold_score: f64,
) {
    info!("test up and running");

    // https://tokio.rs/tokio/topics/bridging
    // We want to run the gui in the main thread, but some long running tasks like
    // loading VCDs should be done asynchronously. We can't just use std::thread to
    // do that due to wasm support, so we'll start a tokio runtime
    let runtime = tokio::runtime::Builder::new_current_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();

    let _enter = runtime.enter();

    std::thread::spawn(move || {
        runtime.block_on(async {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
            }
        });
    });

    let mut state = state();
    state.user.show_statusbar = Some(false);
    // disable the default timeline
    state.user.show_default_timeline = Some(!state.show_default_timeline());

    if state
        .user
        .waves
        .as_ref()
        .and_then(|w| w.inner.as_transactions())
        .is_some_and(|t| t.is_native())
    {
        let context = egui::Context::default();
        for _ in 0..3 {
            let mut output = context.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ui| {
                    setup_custom_font(ui.ctx());
                    for message in state.draw(ui, Some(size)) {
                        state.update(message);
                    }
                },
            );
            output.textures_delta.clear();
            state.handle_async_messages();
            wait_for_waves_fully_loaded(&mut state, 10);
        }
    }
    let size_i = (size.x as i32, size.y as i32);

    let mut surface = create_surface(size_i);
    surface.canvas().clear(egui_skia_renderer::Color::BLACK);

    draw_onto_surface(
        &mut surface,
        |ctx| {
            ctx.memory_mut(|mem| mem.options.tessellation_options.feathering = feathering);
            ctx.set_visuals(state.get_visuals());
            setup_custom_font(ctx);
            let msgs = state.draw(ctx, Some(size));
            // Process only BuildAnalogCache messages as other messages can be fuzzy (command matcher)
            for msg in msgs {
                if matches!(msg, Message::BuildAnalogCache { .. }) {
                    state.update(msg);
                }
            }
            // Match the app's frame pump: screen samples request native payloads
            // after geometry is known, so settle that bounded work before capture.
            if state
                .user
                .waves
                .as_ref()
                .and_then(|w| w.inner.as_transactions())
                .is_some_and(|t| t.is_native())
            {
                state.handle_async_messages();
                wait_for_waves_fully_loaded(&mut state, 10);
            }
            // Wait for analog cache builds to complete
            while !state.analog_caches_ready() {
                std::thread::sleep(std::time::Duration::from_millis(1));
                state.handle_async_messages();
            }
        },
        Some(egui_skia_renderer::RasterizeOptions {
            frames_before_screenshot: 5,
            ..Default::default()
        }),
    );

    let data = surface
        .image_snapshot()
        .encode(None, EncodedImageFormat::PNG, None)
        .expect("Failed to encode image");
    let new = image::load_from_memory(&data).expect("Failed to decode png with image crate");

    let root = get_project_root().expect("Failed to get root");

    let previous_image_file = root.join("snapshots").join(filename).with_extension("png");

    let (write_new_file, diff) = if previous_image_file.exists() {
        let prev = image::open(previous_image_file.clone()).unwrap_or_else(|_| {
            panic!("Failed to load previous image from {previous_image_file:?}")
        });
        let result =
            image_compare::rgb_hybrid_compare(&new.clone().into_rgb8(), &prev.clone().into_rgb8())
                .expect("Comparison failing");
        // comparator.create_image_rgb(&prev_imgref.as_ref(), width, height);

        let (score, map) = (result.score, result.image);
        (score <= threshold_score, Some((score, map)))
    } else {
        (true, None)
    };

    let new_file = root
        .join("snapshots")
        .join(filename)
        .with_extension("new.png");

    if new_file.exists() {
        std::fs::remove_file(&new_file).expect("Failed to remove existing snapshot file");
    }
    if write_new_file {
        std::fs::create_dir_all("snapshots").expect("Failed to create snapshots dir");
        new.write_to(
            &mut File::create(&new_file)
                .unwrap_or_else(|_| panic!("Failed to create {new_file:?}")),
            ImageFormat::Png,
        )
        .unwrap_or_else(|_| panic!("Failed to write new image to {new_file:?}"));
    }

    match (write_new_file, diff) {
        (true, Some((score, map))) => {
            let diff_img = map.to_color_map();
            let diff_file = root
                .join("snapshots")
                .join(filename)
                .with_extension("diff.png");

            diff_img
                .save(diff_file.clone())
                .unwrap_or_else(|_| panic!("Failed to save diff file to {diff_file:?}"));

            let prev = image::open(previous_image_file.clone()).unwrap_or_else(|_| {
                panic!("Failed to load previous image from {previous_image_file:?}")
            });
            println!("Previous: {previous_image_file:?}");
            print_image(&prev);
            println!("New: {new_file:?}");
            print_image(&new);

            println!("Diff: {diff_file:?}");
            print_image(&diff_img);

            panic!(
                "Snapshot diff. Score: {score}\n\told: {previous_image_file:?}\n\tnew: {new_file:?}"
            )
        }
        (true, None) => {
            print_image(&new);
            panic!("New snapshot image (saved to {new_file:?})")
        }
        (false, _) => {}
    }
}

pub(crate) fn render_and_compare(filename: &Utf8Path, state: impl Fn() -> SystemState) {
    render_and_compare_inner(filename, state, SNAPSHOT_SIZE, false, 0.99999);
}

macro_rules! snapshot_ui {
    ($name:ident, $state:expr) => {
        #[test]
        fn $name() {
            render_and_compare(&Utf8PathBuf::from(stringify!($name)), $state);
        }
    };
}

macro_rules! snapshot_empty_state_with_msgs {
    ($name:ident, $msgs:expr) => {
        snapshot_ui! {$name, || {
            let mut state = SystemState::new_default_config().unwrap().with_params(StartupParams::default());
            for msg in $msgs {
                state.update(msg);
            }
            state
        }}
    };
}

macro_rules! snapshot_ui_with_file_and_msgs {
    ($name:ident, $file:expr,state_mods: $initial_state_mods:expr, $msgs:expr) => {
        snapshot_ui_with_file_and_msgs!($name, $file, $initial_state_mods, $msgs);
    };
    ($name:ident, $file:expr, $msgs:expr) => {
        snapshot_ui_with_file_and_msgs!($name, $file, (|_state| {}), $msgs);
    };
    ($name:ident, $file:expr, $initial_state_mod:expr, $msgs:expr) => {
        snapshot_ui_with_file_and_msgs!($name, $file, $initial_state_mod, $msgs, []);
    };
    ($name:ident, $file:expr, $initial_state_mod:expr, $msgs:expr, $late_msgs:expr) => {
        snapshot_ui!($name, || {
            let mut state = SystemState::new_default_config()
                .unwrap()
                .with_params(StartupParams {
                    waves: Some(WaveSource::File(
                        get_project_root().unwrap().join($file).try_into().unwrap(),
                    )),
                    startup_commands: vec![],
                    ..Default::default()
                });

            $initial_state_mod(&mut state);

            let load_start = std::time::Instant::now();

            loop {
                state.handle_async_messages();
                state.handle_batch_commands();
                if state.waves_fully_loaded() {
                    break;
                }

                if load_start.elapsed().as_secs() > 10 {
                    panic!("Timeout")
                }
            }
            state.add_batch_message(Message::SetMenuVisible(false));
            state.add_batch_message(Message::SetSidePanelVisible(false));
            state.add_batch_message(Message::SetToolbarVisible(false));
            state.add_batch_message(Message::SetOverviewVisible(false));
            state.add_batch_message(Message::CloseOpenSiblingStateFileDialog {
                load_state: false,
                do_not_show_again: true,
            });
            state.add_batch_messages($msgs);

            // make sure all the signals added by the proceeding messages are properly loaded
            wait_for_waves_fully_loaded(&mut state, 10);

            let late_msgs: Vec<Message> = $late_msgs.into();
            if !late_msgs.is_empty() {
                // Do a preliminary draw to enable scrolling before processing late messages
                let mut surface = create_surface((SNAPSHOT_WIDTH as i32, SNAPSHOT_HEIGHT as i32));
                draw_onto_surface(
                    &mut surface,
                    |ui| {
                        setup_custom_font(ui);
                        state.draw(ui, Some(SNAPSHOT_SIZE));
                    },
                    None,
                );

                for msg in late_msgs {
                    state.update(msg);
                }
            }

            state
        });
    };
}

/// Run a snapshot test called `$name` loading the theme `$theme`.
macro_rules! snapshot_ui_with_theme {
    ($name:ident, $theme:expr) => {
        snapshot_ui_with_file_and_msgs! {$name, "examples/theme_demo.ghw", [
            Message::AddScope(ScopeRef::from_strs(&["theme_demo"]), false),
            Message::AddTimeLine(None),
            Message::CloseOpenSiblingStateFileDialog {load_state: false, do_not_show_again: true},
            Message::FocusItem(VisibleItemIndex(0)),
            Message::MoveCursorToTransition { next: true, variable: None, skip_zero: true },
            Message::SetFocusHighlight(FocusHighlight::Background),
            Message::SelectTheme(Some($theme.to_string()))
        ]}
    };
}

#[test]
fn render_readme_screenshot() {
    render_and_compare_inner(
        &Utf8PathBuf::from("render_readme_screenshot"),
        || {
            let mut state = SystemState::new_default_config()
                .unwrap()
                .with_params(StartupParams {
                    waves: Some(WaveSource::File(
                        get_project_root()
                            .unwrap()
                            .join("examples")
                            .join("picorv32.vcd")
                            .try_into()
                            .unwrap(),
                    )),
                    ..Default::default()
                });

            let load_start = std::time::Instant::now();

            loop {
                state.handle_async_messages();
                state.handle_batch_commands();

                if state.waves_fully_loaded() {
                    break;
                }

                if load_start.elapsed().as_secs() > 10 {
                    panic!("Timeout")
                }
            }
            let msgs = vec![
                Message::ToDocument(DocumentCommand::SetActiveScope(Some(ScopeType::WaveScope(
                    ScopeRef::from_strs(&["testbench", "top"]),
                )))),
                Message::AddVariables(vec![
                    VariableRef::from_hierarchy_string("testbench.top.clk"),
                    VariableRef::from_hierarchy_string("testbench.top.uut.pcpi_insn"),
                    VariableRef::from_hierarchy_string(
                        "testbench.top.uut.picorv32_core.mem_do_rinst",
                    ),
                ]),
                Message::CloseOpenSiblingStateFileDialog {
                    load_state: false,
                    do_not_show_again: true,
                },
                Message::VariableFormatChange(
                    MessageTarget::Explicit(DisplayedFieldRef {
                        item: DisplayedItemRef(1),
                        field: vec![],
                    }),
                    String::from("Clock"),
                ),
                Message::VariableFormatChange(
                    MessageTarget::Explicit(DisplayedFieldRef {
                        item: DisplayedItemRef(2),
                        field: vec![],
                    }),
                    String::from("RV32"),
                ),
                Message::FocusItem(VisibleItemIndex(2)),
                Message::AddDivider(None, None),
                Message::AddDivider(Some("Top module:".to_string()), None),
                Message::ItemColorChange(
                    MessageTarget::CurrentSelection,
                    Some("green".to_string()),
                ),
                Message::AddScope(ScopeRef::from_strs(&["testbench", "top"]), false),
                Message::ZoomToRange {
                    start: 1612078.to_bigint().unwrap(),
                    end: 2176254.to_bigint().unwrap(),
                    tile_id: crate::tiles::TileId(1),
                },
                Message::SetMarker {
                    id: 0,
                    time: 1764339.to_bigint().unwrap(),
                },
                Message::ItemColorChange(
                    MessageTarget::CurrentSelection,
                    Some("orange".to_string()),
                ),
                Message::SetMarker {
                    id: 1,
                    time: 1912676.to_bigint().unwrap(),
                },
                Message::ItemColorChange(
                    MessageTarget::CurrentSelection,
                    Some("violet".to_string()),
                ),
                Message::ToDocument(DocumentCommand::CursorSet(1820000.to_bigint().unwrap())),
            ];
            state.add_batch_messages(msgs);

            // make sure all the signals added by the proceeding messages are properly loaded
            wait_for_waves_fully_loaded(&mut state, 10);

            state
        },
        Vec2::new(1440., 810.),
        true,
        0.99,
    );
}

fn verilator_example(name: &str, format: &str) -> SystemState {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join(format!("examples/verilator/{name}.{format}"))
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });
    wait_for_waves_fully_loaded(&mut state, 10);
    let signals = match name {
        "pipeline" => vec![
            "clk", "rst_n", "en", "sel", "a", "b", "mux", "first", "q", "u0.q", "u1.q",
        ],
        _ => vec![
            "clk",
            "a",
            "b",
            "index",
            "sum",
            "difference",
            "product",
            "shift_s",
            "selected",
            "enum_value",
            "array_value",
            "replicated",
        ],
    };
    state.update(Message::AddVariables(
        signals
            .into_iter()
            .map(|name| VariableRef::from_hierarchy_string(&format!("TOP.top.{name}")))
            .collect(),
    ));
    wait_for_waves_fully_loaded(&mut state, 10);
    state.update(Message::ToDocument(DocumentCommand::CursorSet(26.into())));
    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetSidePanelVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state
}

#[test]
fn verilator_pipeline_fst_snapshot() {
    render_and_compare(Utf8Path::new("verilator_pipeline"), || {
        verilator_example("pipeline", "fst")
    });
}

#[test]
fn verilator_pipeline_vtr_snapshot() {
    render_and_compare(Utf8Path::new("verilator_pipeline"), || {
        verilator_example("pipeline", "vtr")
    });
}

#[test]
fn verilator_operators_fst_snapshot() {
    render_and_compare(Utf8Path::new("verilator_operators"), || {
        verilator_example("operators", "fst")
    });
}

#[test]
fn verilator_operators_vtr_snapshot() {
    render_and_compare(Utf8Path::new("verilator_operators"), || {
        verilator_example("operators", "vtr")
    });
}

#[test]
fn verilator_document_replacement_keeps_source_attachment_with_its_trace() {
    render_and_compare(Utf8Path::new("verilator_pipeline"), || {
        let mut state = verilator_example("pipeline", "vtr");
        assert!(state.waveform_services().source_index.is_some());
        let root = get_project_root().unwrap().join("examples/verilator");
        state
            .load_from_file(
                root.join("pipeline.fst").try_into().unwrap(),
                LoadOptions::KeepAll,
            )
            .unwrap();
        wait_for_waves_fully_loaded(&mut state, 10);
        assert!(state.waveform_services().source_index.is_none());
        let path: Utf8PathBuf = root.join("pipeline.vtr").try_into().unwrap();
        state
            .load_from_dropped_bytes(Some(path.clone()), std::fs::read(&path).unwrap())
            .unwrap();
        wait_for_waves_fully_loaded(&mut state, 10);
        assert!(state.waveform_services().source_index.is_some());
        // Restore the same displayed variables after a drop replaces the document.
        verilator_example("pipeline", "vtr")
    });
}

snapshot_ui! {verilator_source_navigation, || {
    let mut state = verilator_example("pipeline", "vtr");
    state.user.show_statusbar = Some(false);
    state.user.show_default_timeline = Some(false);
    let ctx = egui::Context::default();
    let frame = |state: &mut SystemState, events| {
    let mut output = ctx.run_ui(RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SNAPSHOT_SIZE)),
        events,
        ..Default::default()
    }, |ui| {
        ui.set_visuals(state.get_visuals());
        setup_custom_font(ui.ctx());
        let messages = state.draw(ui, Some(SNAPSHOT_SIZE));
        for message in messages { state.update(message); }
    });
    output.textures_delta.clear();
    output
    };
    let click = |pos, button| vec![
        Event::PointerMoved(pos),
        Event::PointerButton { pos, button, pressed: true, modifiers: Modifiers::default() },
        Event::PointerButton { pos, button, pressed: false, modifiers: Modifiers::default() },
    ];
    frame(&mut state, vec![]);
    frame(&mut state, vec![]);
    // Right-click the first-stage output row, then find the actual menu label.
    frame(&mut state, click(Pos2::new(45.0, 199.0), PointerButton::Secondary));
    let output = frame(&mut state, vec![]);
    fn label_position(shape: &epaint::Shape) -> Option<Pos2> {
        match shape {
            epaint::Shape::Text(text) if text.galley.job.text == "Go to source" =>
                Some(text.pos + text.galley.rect.center().to_vec2()),
            epaint::Shape::Vec(shapes) => shapes.iter().find_map(label_position),
            _ => None,
        }
    }
    let target = output.shapes.iter().find_map(|shape| label_position(&shape.shape))
        .expect("signal context menu must offer Go to source");
    frame(&mut state, click(target, PointerButton::Primary));
    let source = state.user.workspace.tiles().values().find_map(|tile| {
        if let crate::tiles::kind::TileKind::SourceCode(source) = &tile.kind { Some(source) } else { None }
    }).expect("context-menu click opens source tile");
    assert_eq!(source.line, 2);
    assert_eq!(source.file.as_ref().unwrap().file_name(), Some("pipeline.sv"));
    state
}}

#[cfg(not(target_arch = "wasm32"))]
fn simulation_logs_example(fuzzy: &str, warning_only: bool) -> SystemState {
    simulation_logs_file("examples/simulation_logs.vtr", fuzzy, warning_only)
}
#[cfg(not(target_arch = "wasm32"))]
fn simulation_logs_file(path: &str, fuzzy: &str, warning_only: bool) -> SystemState {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root().unwrap().join(path).try_into().unwrap(),
            )),
            ..Default::default()
        });
    wait_for_waves_fully_loaded(&mut state, 10);
    state.update(Message::Workspace(WorkspaceCommand::OpenTile {
        kind: "simulation_logs".into(),
        placement: crate::tiles::layout::Placement::Edge(TileDirection::Right),
        focus: true,
    }));
    let id = state.user.workspace.layout().focused().unwrap();
    let source = state
        .user
        .waves
        .as_ref()
        .unwrap()
        .inner
        .as_transactions()
        .unwrap()
        .native
        .as_ref()
        .unwrap()
        .logs
        .clone();
    let stream = source
        .streams
        .iter()
        .find(|s| s.1 == "simulation_log")
        .unwrap()
        .0;
    let generator = warning_only.then(|| {
        source
            .generators
            .iter()
            .find(|g| g.stream == stream && g.label.starts_with("warn"))
            .unwrap()
            .id
    });
    state.update(Message::ToTile(
        id,
        crate::tiles::kind::TileMessage::SimulationLogs(
            crate::tile_kinds::simulation_logs::Query {
                stream: Some(stream),
                generator,
                fuzzy: fuzzy.into(),
                ..Default::default()
            },
        ),
    ));
    let others: Vec<_> = state
        .user
        .workspace
        .tiles()
        .keys()
        .copied()
        .filter(|other| *other != id)
        .collect();
    for other in others {
        state.update(Message::Workspace(WorkspaceCommand::CloseTile(other)));
    }
    state.user.show_hierarchy = Some(false);
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetMenuVisible(false));
    state.user.show_statusbar = Some(false);
    let context = egui::Context::default();
    let start = std::time::Instant::now();
    loop {
        schematic_frame(&mut state, &context, vec![]);
        let crate::tiles::kind::TileKind::SimulationLogs(tile) =
            &state.user.workspace.tiles()[&id].kind
        else {
            panic!()
        };
        if tile.ready() {
            break;
        }
        assert!(start.elapsed().as_secs() < 10, "log query did not complete");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    state
}
#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "Generate /tmp/simulation_logs_million.vtr with the VTR simulation_logs example first"]
fn simulation_logs_million_benchmark() {
    render_and_compare(&Utf8PathBuf::from("simulation_logs_all"), || {
        for path in [
            "examples/simulation_logs.vtr",
            "/tmp/simulation_logs_million.vtr",
        ] {
            let start = std::time::Instant::now();
            let mut state = simulation_logs_file(path, "", false);
            let loaded = start.elapsed();
            let context = egui::Context::default();
            schematic_frame(&mut state, &context, vec![]);
            let start = std::time::Instant::now();
            for _ in 0..100 {
                schematic_frame(&mut state, &context, vec![]);
            }
            let frame = start.elapsed() / 100;
            let id = state.user.workspace.layout().focused().unwrap();
            let crate::tiles::kind::TileKind::SimulationLogs(tile) =
                &state.user.workspace.tiles()[&id].kind
            else {
                panic!()
            };
            assert!(tile.rendered_rows() > 0 && tile.rendered_rows() < 40);
            assert_eq!(
                tile.indexed_rows(),
                if path.starts_with("/tmp") {
                    1_010_000
                } else {
                    2020
                }
            );
            println!(
                "{path}: index/open={loaded:?}, frame={frame:?}, rendered_rows={}",
                tile.rendered_rows()
            );
            let mut query = tile.query.clone();
            query.fuzzy = "dmch".into();
            state.update(Message::ToTile(
                id,
                crate::tiles::kind::TileMessage::SimulationLogs(query),
            ));
            let start = std::time::Instant::now();
            let mut max_frame = std::time::Duration::ZERO;
            loop {
                let frame = std::time::Instant::now();
                schematic_frame(&mut state, &context, vec![]);
                max_frame = max_frame.max(frame.elapsed());
                let crate::tiles::kind::TileKind::SimulationLogs(tile) =
                    &state.user.workspace.tiles()[&id].kind
                else {
                    panic!()
                };
                if tile.ready() {
                    break;
                }
                assert!(start.elapsed().as_secs() < 60);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            println!(
                "{path}: fuzzy={:?}, max_frame_during_search={max_frame:?}",
                start.elapsed()
            );
        }
        simulation_logs_example("", false)
    });
}

#[cfg(not(target_arch = "wasm32"))]
snapshot_ui!(simulation_logs_all, || simulation_logs_example("", false));
#[cfg(not(target_arch = "wasm32"))]
snapshot_ui!(simulation_logs_generator, || simulation_logs_example(
    "", true
));
#[cfg(not(target_arch = "wasm32"))]
snapshot_ui!(simulation_logs_fuzzy, || simulation_logs_example(
    "dmch", false
));
#[cfg(not(target_arch = "wasm32"))]
snapshot_ui!(simulation_logs_empty, || simulation_logs_example(
    "unfindable message",
    false
));

#[cfg(not(target_arch = "wasm32"))]
fn simulation_logs_wait(state: &mut SystemState, context: &egui::Context) {
    let start = std::time::Instant::now();
    loop {
        schematic_frame(state, context, vec![]);
        let id = state.user.workspace.layout().focused().unwrap();
        let crate::tiles::kind::TileKind::SimulationLogs(tile) =
            &state.user.workspace.tiles()[&id].kind
        else {
            panic!()
        };
        if tile.ready() {
            break;
        }
        assert!(start.elapsed().as_secs() < 10);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}
#[cfg(not(target_arch = "wasm32"))]
snapshot_ui!(simulation_logs_verilator, || simulation_logs_file(
    "examples/verilator/logs_normal.vtr",
    "",
    false
));
#[cfg(not(target_arch = "wasm32"))]
snapshot_ui!(simulation_logs_time_cursor, || {
    let mut state = simulation_logs_example("", false);
    let context = egui::Context::default();
    let output = schematic_frame(&mut state, &context, vec![]);
    let from = schematic_label(&output, "From");
    schematic_frame(
        &mut state,
        &context,
        schematic_click(from, PointerButton::Primary),
    );
    schematic_frame(&mut state, &context, vec![Event::Text("100".into())]);
    let output = schematic_frame(&mut state, &context, vec![]);
    let to = schematic_label(&output, "To");
    schematic_frame(
        &mut state,
        &context,
        schematic_click(to, PointerButton::Primary),
    );
    schematic_frame(&mut state, &context, vec![Event::Text("200".into())]);
    simulation_logs_wait(&mut state, &context);
    let output = schematic_frame(&mut state, &context, vec![]);
    let timestamp = schematic_label(&output, "130");
    schematic_frame(
        &mut state,
        &context,
        schematic_click(timestamp, PointerButton::Primary),
    );
    assert_eq!(state.user.waves.as_ref().unwrap().cursor, Some(130.into()));
    let id = state.user.workspace.layout().focused().unwrap();
    let crate::tiles::kind::TileKind::SimulationLogs(tile) =
        &state.user.workspace.tiles()[&id].kind
    else {
        panic!()
    };
    assert_eq!((&tile.query.start[..], &tile.query.end[..]), ("100", "200"));
    let clone = tile.clone();
    assert_eq!(clone.query, tile.query);
    assert!(!clone.ready());
    let encoded = state.encode_state().unwrap();
    let restored: crate::state::UserState = crate::tiles::serde::decode(&encoded).unwrap();
    let crate::tiles::kind::TileKind::SimulationLogs(restored) =
        &restored.workspace.tiles()[&id].kind
    else {
        panic!()
    };
    assert_eq!(restored.query, tile.query);
    assert!(!restored.ready());
    state
});
#[cfg(not(target_arch = "wasm32"))]
snapshot_ui!(simulation_logs_generator_menu, || {
    let mut state = simulation_logs_example("", false);
    let context = egui::Context::default();
    let output = schematic_frame(&mut state, &context, vec![]);
    let selector = schematic_label(&output, "All generators");
    schematic_frame(
        &mut state,
        &context,
        schematic_click(selector, PointerButton::Primary),
    );
    let output = schematic_frame(&mut state, &context, vec![]);
    let mut output = output;
    output.shapes.reverse(); // Popup labels are drawn after the underlying table.
    let warning = schematic_label(&output, "warn");
    schematic_frame(
        &mut state,
        &context,
        schematic_click(warning, PointerButton::Primary),
    );
    simulation_logs_wait(&mut state, &context);
    let id = state.user.workspace.layout().focused().unwrap();
    let crate::tiles::kind::TileKind::SimulationLogs(tile) =
        &state.user.workspace.tiles()[&id].kind
    else {
        panic!()
    };
    let source = &state.user.waves.as_ref().unwrap().inner.as_transactions().unwrap().native.as_ref().unwrap().logs;
    assert_eq!(source.generators.iter().find(|g| Some(g.id) == tile.query.generator).unwrap().label, "warn");
    state
});

#[cfg(not(target_arch = "wasm32"))]
snapshot_ui!(simulation_logs_without_recording, || {
    let mut state = simulation_logs_example("", false);
    let path = get_project_root().unwrap().join("examples/verilator/pipeline.fst");
    state.load_from_file(path.try_into().unwrap(), LoadOptions::KeepAll).unwrap();
    wait_for_waves_fully_loaded(&mut state, 10);
    schematic_frame(&mut state, &egui::Context::default(), vec![]);
    let id = state.user.workspace.layout().focused().unwrap();
    let crate::tiles::kind::TileKind::SimulationLogs(tile) = &state.user.workspace.tiles()[&id].kind else { panic!() };
    assert!(!tile.ready(), "old recording data must be cleared");
    state
});

#[cfg(not(target_arch = "wasm32"))]
fn schematic_example(name: &str, instance: &str, highlight: Option<&str>) -> SystemState {
    let mut state = verilator_example(name, "vtr");
    state.update(Message::OpenSchematic(
        instance.into(),
        highlight.map(str::to_owned),
    ));
    let other_tiles: Vec<_> = state
        .user
        .workspace
        .tiles()
        .iter()
        .filter_map(|(id, tile)| (tile.kind.kind_name() != "schematic").then_some(*id))
        .collect();
    for id in other_tiles {
        state.update(Message::Workspace(WorkspaceCommand::CloseTile(id)));
    }
    state.user.show_hierarchy = Some(false);
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetMenuVisible(false));
    state.user.show_statusbar = Some(false);
    schematic_wait(&mut state);
    state
}

#[cfg(not(target_arch = "wasm32"))]
fn schematic_wait(state: &mut SystemState) {
    let ctx = egui::Context::default();
    let deadline = std::time::Instant::now();
    loop {
        schematic_frame(state, &ctx, vec![]);
        if schematic_tile(state).layout_ready() {
            break;
        }
        assert!(
            deadline.elapsed().as_secs() < 10,
            "schematic layout did not complete"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[cfg(not(target_arch = "wasm32"))]
snapshot_ui!(schematic_pipeline, || schematic_example(
    "pipeline", "top", None
));
#[cfg(not(target_arch = "wasm32"))]
snapshot_ui!(schematic_signal_highlight, || schematic_example(
    "pipeline",
    "top",
    Some("top.first")
));
#[cfg(not(target_arch = "wasm32"))]
snapshot_ui!(schematic_child_module, || schematic_example(
    "pipeline",
    "top.u0",
    Some("top.u0.q")
));
#[cfg(not(target_arch = "wasm32"))]
snapshot_ui!(schematic_operators, || schematic_example(
    "operators",
    "top",
    Some("top.sum")
));

#[cfg(not(target_arch = "wasm32"))]
fn schematic_frame(
    state: &mut SystemState,
    context: &egui::Context,
    events: Vec<Event>,
) -> egui::FullOutput {
    let mut output = context.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SNAPSHOT_SIZE)),
            events,
            ..Default::default()
        },
        |ui| {
            ui.set_visuals(state.get_visuals());
            setup_custom_font(ui.ctx());
            for message in state.draw(ui, Some(SNAPSHOT_SIZE)) {
                state.update(message);
            }
        },
    );
    output.textures_delta.clear();
    output
}
#[cfg(not(target_arch = "wasm32"))]
fn schematic_click(position: Pos2, button: PointerButton) -> Vec<Event> {
    vec![
        Event::PointerMoved(position),
        Event::PointerButton {
            pos: position,
            button,
            pressed: true,
            modifiers: Modifiers::default(),
        },
        Event::PointerButton {
            pos: position,
            button,
            pressed: false,
            modifiers: Modifiers::default(),
        },
    ]
}
#[cfg(not(target_arch = "wasm32"))]
fn schematic_label(output: &egui::FullOutput, label: &str) -> Pos2 {
    fn find(shape: &epaint::Shape, label: &str) -> Option<Pos2> {
        match shape {
            epaint::Shape::Text(text) if text.galley.job.text == label => {
                Some(text.pos + text.galley.rect.center().to_vec2())
            }
            epaint::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, label)),
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|shape| find(&shape.shape, label))
        .unwrap_or_else(|| panic!("missing label: {label}"))
}
#[cfg(not(target_arch = "wasm32"))]
fn schematic_tile(state: &SystemState) -> &crate::schematic::SchematicTile {
    state
        .user
        .workspace
        .tiles()
        .values()
        .find_map(|entry| match &entry.kind {
            crate::tiles::kind::TileKind::Schematic(tile) => Some(tile),
            _ => None,
        })
        .unwrap()
}
#[cfg(not(target_arch = "wasm32"))]
snapshot_ui!(schematic_selected_block, || {
    let mut state = schematic_example("pipeline", "top", None);
    let ctx = egui::Context::default();
    let output = schematic_frame(&mut state, &ctx, vec![]);
    let position = schematic_label(&output, "u0 : stage");
    schematic_frame(
        &mut state,
        &ctx,
        schematic_click(position, PointerButton::Primary),
    );
    state
});
#[cfg(not(target_arch = "wasm32"))]
snapshot_ui!(schematic_wire_to_source, || {
    let mut state = schematic_example("pipeline", "top", None);
    let ctx = egui::Context::default();
    schematic_frame(&mut state, &ctx, vec![]);
    let position = schematic_tile(&state).test_wire_point("top.first");
    schematic_frame(
        &mut state,
        &ctx,
        schematic_click(position, PointerButton::Secondary),
    );
    let output = schematic_frame(&mut state, &ctx, vec![]);
    let target = schematic_label(&output, "Go to source");
    schematic_frame(
        &mut state,
        &ctx,
        schematic_click(target, PointerButton::Primary),
    );
    let source = state
        .user
        .workspace
        .tiles()
        .values()
        .find_map(|entry| match &entry.kind {
            crate::tiles::kind::TileKind::SourceCode(source) => Some(source),
            _ => None,
        })
        .expect("wire context menu opens source");
    assert_eq!(
        source.file.as_ref().unwrap().file_name(),
        Some("pipeline.sv")
    );
    assert!(source.line > 0);
    state
});
#[cfg(not(target_arch = "wasm32"))]
snapshot_ui!(schematic_block_to_hierarchy, || {
    let mut state = schematic_example("pipeline", "top", None);
    let ctx = egui::Context::default();
    let output = schematic_frame(&mut state, &ctx, vec![]);
    let position = schematic_label(&output, "u0 : stage");
    schematic_frame(
        &mut state,
        &ctx,
        schematic_click(position, PointerButton::Secondary),
    );
    let output = schematic_frame(&mut state, &ctx, vec![]);
    let target = schematic_label(&output, "Reveal in hierarchy");
    schematic_frame(
        &mut state,
        &ctx,
        schematic_click(target, PointerButton::Primary),
    );
    assert!(state.show_hierarchy());
    assert_eq!(
        state.user.waves.as_ref().unwrap().active_scope,
        Some(ScopeType::WaveScope(ScopeRef::from_strs(&[
            "TOP", "top", "u0"
        ])))
    );
    state
});
#[cfg(not(target_arch = "wasm32"))]
snapshot_ui!(schematic_zoom_and_pan, || {
    let mut state = schematic_example("pipeline", "top", Some("top.first"));
    let ctx = egui::Context::default();
    schematic_frame(&mut state, &ctx, vec![]);
    let before = schematic_tile(&state).test_camera();
    let anchor = Pos2::new(800.0, 400.0);
    schematic_frame(
        &mut state,
        &ctx,
        vec![Event::PointerMoved(anchor), Event::Zoom(1.5)],
    );
    let zoomed = schematic_tile(&state).test_camera();
    assert!(zoomed.0 > before.0);
    let from = Pos2::new(900.0, 580.0);
    let to = Pos2::new(790.0, 540.0);
    schematic_frame(
        &mut state,
        &ctx,
        vec![
            Event::PointerMoved(from),
            Event::PointerButton {
                pos: from,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::default(),
            },
        ],
    );
    schematic_frame(&mut state, &ctx, vec![Event::PointerMoved(to)]);
    schematic_frame(
        &mut state,
        &ctx,
        vec![Event::PointerButton {
            pos: to,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::default(),
        }],
    );
    assert_ne!(schematic_tile(&state).test_camera().1, zoomed.1);
    state
});

#[cfg(not(target_arch = "wasm32"))]
snapshot_ui!(schematic_from_signal_menu, || {
    let mut state = verilator_example("pipeline", "vtr");
    state.user.show_statusbar = Some(false);
    state.user.show_default_timeline = Some(false);
    let ctx = egui::Context::default();
    schematic_frame(&mut state, &ctx, vec![]);
    schematic_frame(&mut state, &ctx, vec![]);
    schematic_frame(
        &mut state,
        &ctx,
        schematic_click(Pos2::new(45.0, 199.0), PointerButton::Secondary),
    );
    let output = schematic_frame(&mut state, &ctx, vec![]);
    let target = schematic_label(&output, "Open schematic");
    schematic_frame(
        &mut state,
        &ctx,
        schematic_click(target, PointerButton::Primary),
    );
    assert_eq!(schematic_tile(&state).instance.as_deref(), Some("top.u0"));
    assert_eq!(
        schematic_tile(&state).highlight.as_deref(),
        Some("top.u0.q")
    );
    schematic_wait(&mut state);
    state
});

#[cfg(not(target_arch = "wasm32"))]
snapshot_ui!(schematic_from_hierarchy_menu, || {
    let mut state = verilator_example("pipeline", "vtr");
    state.update(Message::RevealSchematicHierarchy("TOP.top.u0".into()));
    let ctx = egui::Context::default();
    schematic_frame(&mut state, &ctx, vec![]);
    let output = schematic_frame(&mut state, &ctx, vec![]);
    let target = schematic_label(&output, "u0");
    schematic_frame(
        &mut state,
        &ctx,
        schematic_click(target, PointerButton::Secondary),
    );
    let output = schematic_frame(&mut state, &ctx, vec![]);
    let target = schematic_label(&output, "Open schematic");
    schematic_frame(
        &mut state,
        &ctx,
        schematic_click(target, PointerButton::Primary),
    );
    assert_eq!(schematic_tile(&state).instance.as_deref(), Some("top.u0"));
    assert_eq!(schematic_tile(&state).highlight, None);
    schematic_wait(&mut state);
    state
});

#[cfg(not(target_arch = "wasm32"))]
snapshot_ui!(schematic_drill_and_up, || {
    let mut state = schematic_example("pipeline", "top", None);
    let ctx = egui::Context::default();
    let output = schematic_frame(&mut state, &ctx, vec![]);
    let target = schematic_label(&output, "u0 : stage");
    schematic_frame(
        &mut state,
        &ctx,
        schematic_click(target, PointerButton::Primary),
    );
    schematic_frame(
        &mut state,
        &ctx,
        schematic_click(target, PointerButton::Primary),
    );
    assert_eq!(schematic_tile(&state).instance.as_deref(), Some("top.u0"));
    schematic_wait(&mut state);
    let output = schematic_frame(&mut state, &ctx, vec![]);
    let target = schematic_label(&output, "Up");
    schematic_frame(
        &mut state,
        &ctx,
        schematic_click(target, PointerButton::Primary),
    );
    assert_eq!(schematic_tile(&state).instance.as_deref(), Some("top"));
    schematic_wait(&mut state);
    state
});

#[cfg(not(target_arch = "wasm32"))]
snapshot_ui!(schematic_without_companion, || {
    let mut state = schematic_example("pipeline", "top", Some("top.first"));
    let path = get_project_root()
        .unwrap()
        .join("examples/verilator/pipeline.fst");
    state
        .load_from_file(path.try_into().unwrap(), LoadOptions::KeepAll)
        .unwrap();
    wait_for_waves_fully_loaded(&mut state, 10);
    let ctx = egui::Context::default();
    schematic_frame(&mut state, &ctx, vec![]);
    assert!(
        !schematic_tile(&state).layout_ready(),
        "old design geometry must be cleared"
    );
    state
});

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn schematic_serialization_retains_destination_and_discards_runtime() {
    render_and_compare(Utf8Path::new("schematic_child_module"), || {
    let state = schematic_example("pipeline", "top.u0", Some("top.u0.q"));
    let tile = schematic_tile(&state);
    let saved = serde_json::to_value(tile).unwrap();
    assert_eq!(
        saved,
        serde_json::json!({"instance":"top.u0", "highlight":"top.u0.q"})
    );
    let restored: crate::schematic::SchematicTile = serde_json::from_value(saved).unwrap();
    assert!(!restored.layout_ready());
    assert!(!tile.clone().layout_ready());
    state
    });
}

snapshot_ui! {startup_screen_looks_fine, || {
    SystemState::new_default_config().unwrap().with_params(StartupParams::default())
}}

snapshot_ui! {source_code_tile_renders_systemverilog, || {
    let mut state = SystemState::new_default_config().unwrap();
    state.update(Message::OpenSource(
        get_project_root().unwrap().join("examples/source_tile_demo.sv").try_into().unwrap(),
        6,
        3,
    ));
    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetSidePanelVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state
}}

#[cfg(not(target_arch = "wasm32"))]
snapshot_ui! {vtr_waveform_and_transaction_render_together, || {
    let path = get_project_root().unwrap().join("examples/combined.vtr");


    let mut state = SystemState::new_default_config().unwrap().with_params(StartupParams {
        waves: Some(WaveSource::File(path.try_into().unwrap())),
        ..Default::default()
    });
    let load_start = std::time::Instant::now();
    loop {
        state.handle_async_messages();
        state.handle_batch_commands();
        if state.waves_fully_loaded() {
            break;
        }
        assert!(load_start.elapsed().as_secs() < 10, "VTR snapshot load timed out");
    }
    state.update(Message::AddVariables(vec![VariableRef::from_hierarchy_string("top.count")]));
    state.update(Message::AddStreamOrGenerator(TransactionStreamRef::new_gen(
        StreamId(2),
        GeneratorId(3),
        "issue".into(),
    )));
    wait_for_waves_fully_loaded(&mut state, 10);
    state.update(Message::FocusTransaction(
        Some(TransactionRef { id: TransactionId(1) }),
        TileId(1),
    ));
    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetSidePanelVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state
}}

fn chi_noc_example() -> SystemState {
    let path = get_project_root().unwrap().join("examples/chi_noc.vtr");
    let reader = vtr::Reader::open(&path).unwrap();
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(path.try_into().unwrap())),
            ..Default::default()
        });
    wait_for_waves_fully_loaded(&mut state, 10);
    for controller in ["RNF0", "HNF0"] {
        let stream = reader.find_node(&["chi_noc", controller, "tx"]).unwrap();
        state.update(Message::AddStreamOrGenerator(
            TransactionStreamRef::new_stream(
                StreamId(stream.0 as usize),
                format!("{controller}.tx"),
            ),
        ));
    }
    {
        let waves = state.user.waveform_edit().unwrap();
        waves.document.refresh_time_range(false);
        waves.view.viewport.zoom_to_range(
            &BigInt::from(0),
            &BigInt::from(260),
            waves.document.time_range(),
        );
    }
    state.reconcile_native_transactions();
    wait_for_waves_fully_loaded(&mut state, 10);
    state.invalidate_draw_commands();
    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetSidePanelVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state
}

#[test]
fn vtr_chi_noc_packet_streams() {
    render_and_compare_inner(
        &Utf8PathBuf::from("vtr_chi_noc_packet_streams"),
        || {
            let mut state = chi_noc_example();
            state.user.config.layout.transactions_line_height = 12.0;
            {
                let waves = state.user.waveform_edit().unwrap();
                waves.view.viewport.zoom_to_range(
                    &400.into(),
                    &640.into(),
                    waves.document.time_range(),
                );
            }
            state.invalidate_draw_commands();
            state
        },
        Vec2::new(1280.0, 1600.0),
        false,
        0.99999,
    );
}

#[test]
fn vtr_event_marker_hover_and_click() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        tokio::task::spawn_blocking(|| {
            let mut state = chi_noc_example();
            let context = egui::Context::default();
            context.style_mut_of(egui::Theme::Dark, |style| {
                style.interaction.tooltip_delay = 0.0;
                style.interaction.tooltip_grace_time = 0.0;
            });
            schematic_frame(&mut state, &context, vec![]);
            state.handle_async_messages();
            wait_for_waves_fully_loaded(&mut state, 10);
            let output = schematic_frame(&mut state, &context, vec![]);
            // Find an actual painted dot, avoiding assumptions about canvas offsets.
            let marker = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    epaint::Shape::Circle(circle)
                        if circle.radius == 3.0 && circle.center.x > 300.0 =>
                    {
                        Some(circle.center)
                    }
                    _ => None,
                })
                .expect("event marker is painted");
            schematic_frame(&mut state, &context, vec![Event::PointerMoved(marker)]);
            let mut output = schematic_frame(&mut state, &context, vec![]);
            for _ in 0..3 {
                output = schematic_frame(&mut state, &context, vec![]);
            }
            schematic_label(&output, "router_1");
            schematic_label(&output, "50 ns");
            schematic_label(&output, "hop: 1");
            schematic_frame(
                &mut state,
                &context,
                schematic_click(marker, PointerButton::Primary),
            );
            assert_eq!(
                state
                    .user
                    .waveform_read()
                    .unwrap()
                    .view
                    .focused_transaction
                    .as_ref()
                    .unwrap()
                    .id,
                TransactionId(1)
            );
        })
        .await
        .unwrap();
    });
}

snapshot_ui!(menu_can_be_hidden, || {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams::default());
    let msgs = [Message::SetMenuVisible(false)];
    for message in msgs {
        state.update(message);
    }
    state
});

snapshot_ui!(side_panel_can_be_hidden, || {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams::default());
    let msgs = [Message::SetSidePanelVisible(false)];
    for message in msgs {
        state.update(message);
    }
    state
});

snapshot_ui!(toolbar_can_be_hidden, || {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams::default());
    let msgs = [Message::SetToolbarVisible(false)];
    for message in msgs {
        state.update(message);
    }
    state
});

snapshot_ui!(overview_can_be_hidden, || {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples")
                    .join("counter.vcd")
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });

    loop {
        state.handle_async_messages();
        state.handle_batch_commands();
        if state.waves_fully_loaded() {
            break;
        }
    }
    state.update(Message::CloseOpenSiblingStateFileDialog {
        load_state: false,
        do_not_show_again: true,
    });
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.dut.counter"),
    ]));
    state.update(Message::ToDocument(DocumentCommand::CursorSet(
        BigInt::from(10),
    )));
    state.update(Message::SetOverviewVisible(false));
    // make sure all the signals added by the proceeding messages are properly loaded
    wait_for_waves_fully_loaded(&mut state, 10);
    state
});

snapshot_ui!(statusbar_can_be_hidden, || {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples")
                    .join("counter.vcd")
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });

    loop {
        state.handle_async_messages();
        state.handle_batch_commands();
        if state.waves_fully_loaded() {
            break;
        }
    }
    state.update(Message::CloseOpenSiblingStateFileDialog {
        load_state: false,
        do_not_show_again: true,
    });
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.dut.counter"),
    ]));
    state.update(Message::ToDocument(DocumentCommand::CursorSet(
        BigInt::from(10),
    )));
    state.update(Message::SetStatusbarVisible(false));
    // make sure all the signals added by the proceeding messages are properly loaded
    wait_for_waves_fully_loaded(&mut state, 10);
    state
});

snapshot_ui! {example_vcd_renders, || {
    let mut state = SystemState::new_default_config().unwrap().with_params(StartupParams {
        waves: Some(WaveSource::File(get_project_root().unwrap().join("examples").join("counter.vcd").try_into().unwrap())),
        ..Default::default()
    });

    loop {
        state.handle_async_messages();
        state.handle_batch_commands();
        if state.waves_fully_loaded() {
            break;
        }
    }

    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetSidePanelVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state.update(Message::CloseOpenSiblingStateFileDialog {load_state: false, do_not_show_again: true});
    state.update(Message::AddScope(ScopeRef::from_strs(&["tb"]), false));
    state.update(Message::AddScope(ScopeRef::from_strs(&["tb", "dut"]), false));
    // make sure all the signals added by the proceeding messages are properly loaded
    wait_for_waves_fully_loaded(&mut state, 10);
    state
}}

snapshot_empty_state_with_msgs! {
    dialogs_work,
    [
        Message::SetMenuVisible(false),
        Message::SetSidePanelVisible(false),
        Message::SetToolbarVisible(false),
        Message::SetOverviewVisible(false),
        Message::CloseOpenSiblingStateFileDialog {load_state: false, do_not_show_again: true},
        Message::SetUrlEntryVisible(true, None),  // No need to provide callback
        Message::SetKeyHelpVisible(true),
        Message::SetGestureHelpVisible(true),
        Message::SetLicenseVisible(true),
    ]
}
snapshot_empty_state_with_msgs! {
    quick_start_works,
    [
        Message::SetQuickStartVisible(true),
    ]
}

snapshot_ui_with_file_and_msgs! {top_level_signals_have_no_aliasing, "examples/picorv32.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["testbench"]), false)
]}

snapshot_ui_with_file_and_msgs! {expand_scope_works, "examples/counter.vcd", [
    Message::SetSidePanelVisible(true),
    Message::AddScope(ScopeRef::from_strs(&["tb"]), true),
    Message::ExpandScope(ScopeExpandType::ExpandSpecific(ScopeRef::from_strs(&["tb", "dut"]))),
]}

snapshot_ui_with_file_and_msgs! {expand_all_scopes_works, "examples/counter.vcd", [
    Message::SetSidePanelVisible(true),
    Message::AddScope(ScopeRef::from_strs(&["tb"]), true),
    Message::ExpandScope(ScopeExpandType::ExpandAll),
]}

snapshot_ui! {resizing_the_canvas_redraws, || {
    let mut state = SystemState::new_default_config().unwrap().with_params(StartupParams {
        waves: Some(WaveSource::File(get_project_root().unwrap().join("examples").join("counter.vcd").try_into().unwrap())),
        ..Default::default()
    });

    loop {
        state.handle_async_messages();
        state.handle_batch_commands();
        if state.waves_fully_loaded() {
            break;
        }
    }

    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state.update(Message::AddScope(ScopeRef::from_strs(&["tb"]), false));
    state.update(Message::CloseOpenSiblingStateFileDialog {load_state: false, do_not_show_again: true});
    state.update(Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(100))));
    // make sure all the signals added by the proceeding messages are properly loaded
    wait_for_waves_fully_loaded(&mut state, 10);

    // Render the UI once with the sidebar shown
    let size_i = (SNAPSHOT_WIDTH as i32, SNAPSHOT_HEIGHT as i32);
    let mut surface = create_surface(size_i);
    surface.canvas().clear(egui_skia_renderer::Color::BLACK);

    draw_onto_surface(
        &mut surface,
        |ctx| {
            ctx.memory_mut(|mem| mem.options.tessellation_options.feathering = false);
            ctx.set_visuals(state.get_visuals());

            state.draw(ctx, Some(SNAPSHOT_SIZE));
        },
        None,
    );

    state.update(Message::SetSidePanelVisible(false));

    state
}}

snapshot_ui_with_file_and_msgs! {set_variable_name_type, "examples/counter.vcd", [
    Message::AddVariables(vec![VariableRef::from_hierarchy_string("tb.dut.clk"),
                               VariableRef::from_hierarchy_string("tb.dut.clk"),
                               VariableRef::from_hierarchy_string("tb.dut.clk"),
                               VariableRef::from_hierarchy_string("tb.clk")]),
    Message::ChangeVariableNameType(MessageTarget::Explicit(VisibleItemIndex(0)), VariableNameType::Global),
    Message::FocusItem(VisibleItemIndex(1)),
    Message::ChangeVariableNameType(MessageTarget::CurrentSelection, VariableNameType::Unique),
    Message::ChangeVariableNameType(MessageTarget::Explicit(VisibleItemIndex(2)), VariableNameType::Local),
    Message::ChangeVariableNameType(MessageTarget::Explicit(VisibleItemIndex(3)), VariableNameType::Unique),
]}

snapshot_ui_with_file_and_msgs! {clock_pulses_render_line, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::VariableFormatChange(MessageTarget::Explicit(DisplayedFieldRef{item: DisplayedItemRef(2), field: vec![]}), String::from("Clock")),
    Message::SetClockHighlightType(ClockHighlightType::Line),
]}

snapshot_ui_with_file_and_msgs! {clock_pulses_render_cycle, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::VariableFormatChange(MessageTarget::Explicit(DisplayedFieldRef{item: DisplayedItemRef(2), field: vec![]}), String::from("Clock")),
    Message::SetClockHighlightType(ClockHighlightType::Cycle),
]}

snapshot_ui_with_file_and_msgs! {clock_pulses_render_none, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::VariableFormatChange(MessageTarget::Explicit(DisplayedFieldRef{item: DisplayedItemRef(2), field: vec![]}), String::from("Clock")),
    Message::SetClockHighlightType(ClockHighlightType::None),
]}

snapshot_ui_with_file_and_msgs! {multiple_clock_pulses_render_line, "examples/three_clocks.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["logic"]), false),
    Message::VariableFormatChange(MessageTarget::Explicit(DisplayedFieldRef{item: DisplayedItemRef(2), field: vec![]}), String::from("Clock")),
    Message::VariableFormatChange(MessageTarget::Explicit(DisplayedFieldRef{item: DisplayedItemRef(3), field: vec![]}), String::from("Clock")),
    Message::VariableFormatChange(MessageTarget::Explicit(DisplayedFieldRef{item: DisplayedItemRef(1), field: vec![]}), String::from("Clock")),
    Message::SetClockHighlightType(ClockHighlightType::Line),
]}

snapshot_ui_with_file_and_msgs! {multiple_clock_pulses_render_cycle, "examples/three_clocks.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["logic"]), false),
    Message::VariableFormatChange(MessageTarget::Explicit(DisplayedFieldRef{item: DisplayedItemRef(2), field: vec![]}), String::from("Clock")),
    Message::VariableFormatChange(MessageTarget::Explicit(DisplayedFieldRef{item: DisplayedItemRef(3), field: vec![]}), String::from("Clock")),
    Message::VariableFormatChange(MessageTarget::Explicit(DisplayedFieldRef{item: DisplayedItemRef(1), field: vec![]}), String::from("Clock")),
    Message::SetClockHighlightType(ClockHighlightType::Cycle),
]}

snapshot_ui_with_file_and_msgs! {multiple_clock_pulses_render_none, "examples/three_clocks.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["logic"]), false),
    Message::VariableFormatChange(MessageTarget::Explicit(DisplayedFieldRef{item: DisplayedItemRef(3), field: vec![]}), String::from("Clock")),
    Message::VariableFormatChange(MessageTarget::Explicit(DisplayedFieldRef{item: DisplayedItemRef(1), field: vec![]}), String::from("Clock")),
    Message::VariableFormatChange(MessageTarget::Explicit(DisplayedFieldRef{item: DisplayedItemRef(2), field: vec![]}), String::from("Clock")),
    Message::SetClockHighlightType(ClockHighlightType::None),
]}

snapshot_ui_with_file_and_msgs! {recursive_add_scope, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), true),
]}

snapshot_ui_with_file_and_msgs! {recursive_add_scope_with_types, "examples/manytypes2.fst", [
    Message::AddScope(ScopeRef::from_strs(&["comprehensive2_tb"]), true),
]}

snapshot_ui_with_file_and_msgs! {add_scope_as_group, "examples/picorv32.vcd", [
    Message::AddScopeAsGroup(ScopeRef::from_strs(&["testbench"]), false),
]}

snapshot_ui_with_file_and_msgs! {add_scope_as_group_recursively, "examples/picorv32.vcd", [
    Message::AddScopeAsGroup(ScopeRef::from_strs(&["testbench"]), true),
]}

snapshot_ui_with_file_and_msgs! {vertical_scrolling_works, "examples/picorv32.vcd",
    (|_state| {}),
    [Message::AddScope(ScopeRef::from_strs(&["testbench", "top", "mem"]), false)],
    [
        Message::ToTile(crate::tiles::TileId(1), crate::tiles::kind::TileMessage::Waveform(crate::tile_kinds::waveform::WaveformMessage::ScrollRows { down: true, count: 3 })),
        Message::ToTile(crate::tiles::TileId(1), crate::tiles::kind::TileMessage::Waveform(crate::tile_kinds::waveform::WaveformMessage::ScrollRows { down: false, count: 1 })),
    ]
}

snapshot_ui_with_file_and_msgs! {vcd_with_empty_scope_loads, "examples/verilator_empty_scope.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["top_test"]), false),
]}

snapshot_ui_with_file_and_msgs! {fst_with_sv_data_types_loads, "examples/many_sv_datatypes.fst", [
    Message::AddScope(ScopeRef::from_strs(&["TOP", "SVDataTypeWrapper", "bb"]), false),
]}

snapshot_ui_with_file_and_msgs! {fst_from_vhdl_loads, "examples/vhdl3.fst", [
    Message::AddScope(ScopeRef::from_strs(&["test", "rr"]), false),
]}

snapshot_ui_with_file_and_msgs! {vcd_from_vhdl_loads, "examples/vhdl3.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["test", "rr"]), false),
]}

// This VCD file contains signals that are not initialized at zero and only obtain their first value at a later point.
snapshot_ui_with_file_and_msgs! {vcd_with_non_zero_start_displays_correctly, "examples/gameroy_trace_with_non_zero_start.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["gameroy", "cpu"]), false),
]}

// This VCD file used to cause Issue 145 (https://gitlab.com/surfer-project/surfer/-/issues/145).
// It contains "false" changes, where a change to the same value is reported.
snapshot_ui_with_file_and_msgs! {vcd_with_false_changes_correctly, "examples/issue_145.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["logic"]), false),
]}

// This GHW file was generated with GHDL from a simple VHDL test provided by oscar.
snapshot_ui_with_file_and_msgs! {simple_ghw_loads, "examples/oscar_test.ghw", [
    Message::AddScope(ScopeRef::from_strs(&["test"]), false),
    Message::AddScope(ScopeRef::from_strs(&["test", "rr"]), false),
]}

// This GHW file comes from the GHDL regression suite.
snapshot_ui_with_file_and_msgs! {ghw_from_ghdl_suite_loads, "examples/tb_recv.ghw", [
    Message::AddScope(ScopeRef::from_strs(&["tb_recv"]), false),
    Message::AddScope(ScopeRef::from_strs(&["tb_recv", "dut"]), false),
]}

// This GHW file was generated with GHDL using VHDL fixed_pkg.
snapshot_ui_with_file_and_msgs! {fixedpoint_translator_selected, "examples/vhdlfixed.ghw", [
    Message::AddScope(ScopeRef::from_strs(&["constantadder_fixed_tb", "dut"]), false),
]}

// This FST file was generated with NVC using VHDL numeric_std.
snapshot_ui_with_file_and_msgs! {signed_translator_selected, "examples/vhdlsigned.fst", [
    Message::AddScope(ScopeRef::from_strs(&["constantadder_tb", "dut"]), false),
]}

snapshot_ui_with_file_and_msgs! {divider_works, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::AddDivider(Some("Divider".to_string()), None),
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::ItemBackgroundColorChange(MessageTarget::Explicit(VisibleItemIndex(4)), Some("Blue".to_string())),
    Message::ItemColorChange(MessageTarget::Explicit(VisibleItemIndex(4)), Some("Green".to_string()))
]}

snapshot_ui_with_file_and_msgs! {markers_work, "examples/counter.vcd", [
    Message::SetOverviewVisible(true),
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(600))),
    Message::MoveMarkerToCursor(2),
    Message::ItemColorChange(MessageTarget::Explicit(VisibleItemIndex(4)), Some("Blue".to_string())),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(200))),
    Message::MoveMarkerToCursor(1),
    Message::ItemColorChange(MessageTarget::Explicit(VisibleItemIndex(5)), Some("Green".to_string())),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(500))),
]}

snapshot_ui_with_file_and_msgs! {markers_dialog_work, "examples/counter.vcd", [
    Message::SetOverviewVisible(true),
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(600))),
    Message::MoveMarkerToCursor(2),
    Message::ItemColorChange(MessageTarget::Explicit(VisibleItemIndex(4)), Some("Blue".to_string())),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(200))),
    Message::MoveMarkerToCursor(1),
    Message::ItemColorChange(MessageTarget::Explicit(VisibleItemIndex(5)), Some("Green".to_string())),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(100))),
    Message::MoveMarkerToCursor(3),
    Message::ItemColorChange(MessageTarget::Explicit(VisibleItemIndex(6)), Some("Orange".to_string())),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(350))),
    Message::MoveMarkerToCursor(4),
    Message::ItemColorChange(MessageTarget::Explicit(VisibleItemIndex(7)), Some("Yellow".to_string())),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(500))),
    Message::Workspace(crate::tiles::commands::WorkspaceCommand::OpenTile {
                    kind: "markers".into(), placement: crate::tiles::layout::Placement::Edge(crate::tiles::layout::Direction::Right), focus: true,
                })
]}

snapshot_ui_with_file_and_msgs! {transition_value_next, "examples/counter.vcd", [
    Message::SetOverviewVisible(true),
    Message::SetTransitionValue(TransitionValue::Next),
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::AddScope(ScopeRef::from_strs(&["tb", "dut"]), false),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(390))),
]}

snapshot_ui_with_file_and_msgs! {transition_value_previous, "examples/counter.vcd", [
    Message::SetOverviewVisible(true),
    Message::SetTransitionValue(TransitionValue::Previous),
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::AddScope(ScopeRef::from_strs(&["tb", "dut"]), false),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(390))),
]}

snapshot_ui_with_file_and_msgs! {transition_value_both, "examples/counter.vcd", [
    Message::SetOverviewVisible(true),
    Message::SetTransitionValue(TransitionValue::Both),
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::AddScope(ScopeRef::from_strs(&["tb", "dut"]), false),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(390))),
]}

snapshot_ui_with_file_and_msgs! {transition_value_both_zero_works, "examples/counter.vcd", [
    Message::SetOverviewVisible(true),
    Message::SetTransitionValue(TransitionValue::Both),
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(0))),
]}

snapshot_ui_with_file_and_msgs! {add_move_delete_marker, "examples/counter.vcd", [
    Message::AddVariables(vec![VariableRef::from_hierarchy_string("tb.dut.counter")]),
    // Add marker with name
    Message::AddMarker{time: 200.into(), name:Some("Test".to_string()), move_focus: true},
    // Add marker w/o name and move it
    Message::AddMarker{time: 200.into(), name: None, move_focus: true},
    Message::SetMarker{id: 1, time: 250.into()},
    // Add marker by moving it to cursor, must create one bec. of ID
    Message::ToDocument(DocumentCommand::CursorSet(300.into())),
    Message::MoveMarkerToCursor(10),
    // Setting non-existing marker must create one (10)
    Message::SetMarker{id: 11, time: 400.into()},
    // Add and remove a marker again
    Message::AddMarker{time: 350.into(), name: None, move_focus: true},
    Message::RemoveMarker(2),
    // Removing a non-existing marker must not crash
    Message::RemoveMarker(100),
]}

snapshot_ui_with_file_and_msgs! {goto_markers, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(600))),
    Message::MoveMarkerToCursor(2),
    Message::GoToMarkerPosition(2, crate::tiles::TileId(1))
]}

snapshot_ui_with_file_and_msgs! {delete_marker_row_preserves_shared_time, "examples/counter.vcd", [
    Message::AddVariables(vec![VariableRef::from_hierarchy_string("tb.dut.counter")]),
    Message::AddMarker{time: 200.into(), name: None, move_focus: true},
    Message::RemoveVisibleItems(MessageTarget::Explicit(VisibleItemIndex(1))),
    Message::RemoveVisibleItems(MessageTarget::Explicit(VisibleItemIndex(1))),
]}

snapshot_ui_with_file_and_msgs! {add_annotation, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::RectangleAdded { time_at_start: BigInt::from(300), time_at_end: BigInt::from(500), wave_from: Some(GraphicsY {
        item: DisplayedItemRef(2),
        anchor: Anchor::Top,
    }), wave_to: Some(GraphicsY {
        item: DisplayedItemRef(3),
        anchor: Anchor::Bottom,
    }), rect: Rect::ZERO },
    Message::ArrowAdded { wave_point_from: WavePoint{
        time: BigInt::from(100),
        attached_item: Some(DisplayedItemRef(3)),
        screen_pos: Pos2::new(0., 0.),
    }, wave_point_to: WavePoint{
        time: BigInt::from(200),
        attached_item: Some(DisplayedItemRef(3)),
        screen_pos: Pos2::new(0., 0.)}, head_mode: crate::arrow::ArrowHeadMode::End },
    Message::ArrowAdded { wave_point_from: WavePoint{
        time: BigInt::from(100),
        attached_item: Some(DisplayedItemRef(1)),
        screen_pos: Pos2::new(0., 0.),
    }, wave_point_to: WavePoint{
        time: BigInt::from(200),
        attached_item: Some(DisplayedItemRef(1)),
        screen_pos: Pos2::new(0., 0.)}, head_mode: crate::arrow::ArrowHeadMode::Double },
]}

snapshot_ui_with_file_and_msgs! {annotation_list_works, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::RectangleAdded { time_at_start: BigInt::from(300), time_at_end: BigInt::from(500), wave_from: Some(GraphicsY {
        item: DisplayedItemRef(2),
        anchor: Anchor::Top,
    }), wave_to: Some(GraphicsY {
        item: DisplayedItemRef(3),
        anchor: Anchor::Bottom,
    }), rect: Rect::ZERO },
    Message::Workspace(crate::tiles::commands::WorkspaceCommand::OpenTile {
                    kind: "annotation_list".into(), placement: crate::tiles::layout::Placement::Edge(crate::tiles::layout::Direction::Right), focus: true,
                }),
    Message::CreateAnnotationGroup("test".to_string()),
    Message::CreateAnnotationGroup("test2".to_string()),
    Message::DeleteAnnotationGroup("test2".to_string()),
]}

snapshot_ui_with_file_and_msgs! {
    startup_commands_work,
    "examples/counter.vcd",
    state_mods: (|state: &mut SystemState| {
        state.add_batch_commands(vec!["scope_add tb".to_string()]);
    }),
    []
}

// NOTE: The `divider_add .` command currently fails because of a bug in the CLI
// parsing library. If we fix that, it this test should be updated. For now, it is
// enough to make sure that this one broken command doesn't bring the rest of the
// test down
snapshot_ui_with_file_and_msgs! {
    yosys_blogpost_startup_commands_work,
    "examples/picorv32.vcd",
    state_mods: (|state: &mut SystemState| {
        state.add_batch_commands(vec!["startup_commands=module_add testbench;divider_add .;divider_add top;module_add testbench.top;show_quick_start".to_string()]);
    }),
    []
}

snapshot_ui_with_file_and_msgs! {signals_are_added_at_focus, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::FocusItem(VisibleItemIndex(1)),
    Message::AddVariables(vec![VariableRef::from_hierarchy_string("tb.dut.counter")])
]}

snapshot_ui_with_file_and_msgs! {dividers_are_added_at_focus, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::FocusItem(VisibleItemIndex(1)),
    Message::AddDivider(Some(String::from("Test")), None)
]}

snapshot_ui_with_file_and_msgs! {dividers_are_appended_without_focus, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::AddDivider(Some(String::from("Test")), None)
]}

snapshot_ui_with_file_and_msgs! {timeline_render, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::AddTimeLine(None)
]}

snapshot_ui_with_file_and_msgs! {toggle_tick_lines, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::SetTickLines(false)
]}

snapshot_ui_with_file_and_msgs! {command_prompt, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::ShowCommandPrompt(String::new(), None)
]}

snapshot_ui_with_file_and_msgs! {command_prompt_with_init_text, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::ShowCommandPrompt("test ".to_string(), None)
]}

snapshot_ui_with_file_and_msgs! {command_prompt_next_command, "examples/counter.vcd", [
    Message::ShowCommandPrompt(String::new(), None),
    Message::CommandPromptUpdate { suggestions: vec![("test".to_string(), vec![true, true, false, false]); 10] },
    Message::SelectNextCommand,
    Message::SelectNextCommand,
    Message::SelectNextCommand,
    Message::SelectNextCommand,
    Message::SelectNextCommand,
]}

snapshot_ui_with_file_and_msgs! {command_prompt_prev_command, "examples/counter.vcd", [
    Message::ShowCommandPrompt(String::new(), None),
    Message::CommandPromptUpdate { suggestions: vec![("test".to_string(), vec![true, true, false, false]); 10] },
    Message::SelectNextCommand,
    Message::SelectNextCommand,
    Message::SelectNextCommand,
    Message::SelectNextCommand,
    Message::SelectPrevCommand
]}

// FIXME: The test is broken, but scrolling still works
// snapshot_ui_with_file_and_msgs! {command_prompt_scrolls, "examples/counter.vcd", [
//     Message::ShowCommandPrompt(true),
//     Message::CommandPromptUpdate { suggestions: vec![("test".to_string(), vec![true, true, false, false]); 50] },
//     Message::SelectNextCommand,
//     Message::SelectNextCommand,
//     Message::SelectNextCommand,
//     Message::SelectNextCommand,
//     Message::SelectNextCommand,
//     Message::SelectNextCommand,
//     Message::SelectNextCommand,
//     Message::SelectNextCommand,
//     Message::SelectNextCommand,
//     Message::SelectNextCommand,
//     Message::SelectNextCommand,
//     Message::SelectNextCommand,
//     Message::SelectNextCommand,
//     Message::SelectNextCommand,
//     Message::SelectNextCommand,
//     Message::SelectNextCommand,
//     Message::SelectNextCommand,
//     Message::SelectNextCommand,
//     Message::SelectNextCommand,
//     Message::SelectNextCommand,
//     Message::SelectNextCommand,
//     Message::SelectNextCommand,
//     Message::SelectNextCommand
// ]}

snapshot_ui_with_file_and_msgs! {command_prompt_focus, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::ItemHeightScalingFactorChange(MessageTarget::Explicit(VisibleItemIndex(2)), 4.0),
    Message::ShowCommandPrompt("item_focus ".to_string(), None)
]}

snapshot_ui_with_file_and_msgs! {command_prompt_focus_item_selected, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::ShowCommandPrompt("item_focus b".to_string(), None)
]}

snapshot_ui_with_file_and_msgs!(
    command_prompt_scroll_bounds_prev,
    "examples/counter.vcd",
    [
        Message::ShowCommandPrompt(String::new(), None),
        Message::CommandPromptUpdate {
            suggestions: vec![("test".to_string(), vec![true, true, false, false]); 5]
        },
        Message::SelectPrevCommand,
    ]
);

snapshot_ui_with_file_and_msgs!(
    command_prompt_scroll_bounds_next,
    "examples/counter.vcd",
    [
        Message::ShowCommandPrompt(String::new(), None),
        Message::CommandPromptUpdate {
            suggestions: vec![("test".to_string(), vec![true, true, false, false]); 5]
        },
        // 5 items, 6 "select next command"
        Message::SelectNextCommand,
        Message::SelectNextCommand,
        Message::SelectNextCommand,
        Message::SelectNextCommand,
        Message::SelectNextCommand,
        Message::SelectNextCommand,
    ]
);

snapshot_ui_with_file_and_msgs! {zoom_in_exceedingly, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::CanvasZoom {mouse_ptr: None, delta:0.000000001, tile_id: crate::tiles::TileId(1)},
]}

snapshot_ui_with_file_and_msgs! {negative_cursorlocation, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::GoToTime(Some(BigInt::from(-50)), crate::tiles::TileId(1)),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(-100))),
]}

snapshot_ui_with_file_and_msgs! {goto_start, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::CanvasZoom {mouse_ptr: None, delta:0.2, tile_id: crate::tiles::TileId(1)},
    Message::GoToStart{tile_id: crate::tiles::TileId(1)}
]}

snapshot_ui_with_file_and_msgs! {goto_end, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::CanvasZoom {mouse_ptr: None, delta:0.2, tile_id: crate::tiles::TileId(1)},
    Message::GoToEnd{tile_id: crate::tiles::TileId(1)}
]}

snapshot_ui_with_file_and_msgs! {zoom_to_fit, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::CanvasZoom {mouse_ptr: None, delta:0.2, tile_id: crate::tiles::TileId(1)},
    Message::GoToEnd{tile_id: crate::tiles::TileId(1)},
    Message::ZoomToFit{tile_id: crate::tiles::TileId(1)}
]}

snapshot_ui_with_file_and_msgs! {zoom_to_range, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::ZoomToRange { start: BigInt::from(100), end: BigInt::from(250) , tile_id: crate::tiles::TileId(1)}
]}

snapshot_ui_with_file_and_msgs! {height_scaling, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::ItemHeightScalingFactorChange(MessageTarget::Explicit(VisibleItemIndex(2)), 4.0)
]}

snapshot_ui_with_file_and_msgs! {remove_item, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::RemoveVisibleItems(MessageTarget::Explicit(VisibleItemIndex(1))),
]}

snapshot_ui_with_file_and_msgs! {remove_item_with_focus, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::FocusItem(VisibleItemIndex(1)),
    Message::RemoveVisibleItems(MessageTarget::Explicit(VisibleItemIndex(1))),
]}

snapshot_ui_with_file_and_msgs! {remove_item_before_focus, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::FocusItem(VisibleItemIndex(3)),
    Message::RemoveVisibleItems(MessageTarget::Explicit(VisibleItemIndex(1))),
]}

snapshot_ui_with_file_and_msgs! {remove_item_after_focus, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::FocusItem(VisibleItemIndex(1)),
    Message::RemoveVisibleItems(MessageTarget::Explicit(VisibleItemIndex(2))),
]}

snapshot_ui_with_file_and_msgs! {canvas_scroll, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::CanvasScroll { delta: Vec2 { x: 0., y: 100.}, tile_id: crate::tiles::TileId(1) }
]}

snapshot_ui_with_file_and_msgs! {move_focused_item_up, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::FocusItem(VisibleItemIndex(2)),
    Message::MoveFocusedItem(MoveDir::Up, 1),
]}

snapshot_ui_with_file_and_msgs! {move_focused_item_to_top, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::FocusItem(VisibleItemIndex(2)),
    Message::MoveFocusedItem(MoveDir::Up, 4),
]}

snapshot_ui_with_file_and_msgs! {move_focused_item_down, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::FocusItem(VisibleItemIndex(0)),
    Message::MoveFocusedItem(MoveDir::Down, 2),
]}

snapshot_ui_with_file_and_msgs! {move_focused_item_to_bottom, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::FocusItem(VisibleItemIndex(0)),
    Message::MoveFocusedItem(MoveDir::Down, 10),
]}

snapshot_ui_with_file_and_msgs! {move_focus_up, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::FocusItem(VisibleItemIndex(2)),
    Message::MoveFocus(MoveDir::Up, 1, false),
]}

snapshot_ui_with_file_and_msgs! {move_focus_to_top, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::FocusItem(VisibleItemIndex(2)),
    Message::MoveFocus(MoveDir::Up, 4, false),
]}

snapshot_ui_with_file_and_msgs! {move_focus_down, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::FocusItem(VisibleItemIndex(0)),
    Message::MoveFocus(MoveDir::Down, 2, false),
]}

snapshot_ui_with_file_and_msgs! {move_focus_to_bottom, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::FocusItem(VisibleItemIndex(0)),
    Message::MoveFocus(MoveDir::Down, 10, false),
]}

snapshot_ui_with_file_and_msgs! {selection_extend_up, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::FocusItem(VisibleItemIndex(2)),
    Message::MoveFocus(MoveDir::Up, 1, true),
]}

snapshot_ui_with_file_and_msgs! {selection_extend_down, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::FocusItem(VisibleItemIndex(2)),
    Message::MoveFocus(MoveDir::Down, 1, true),
]}

snapshot_ui_with_file_and_msgs! {selection_extend_change_color, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::FocusItem(VisibleItemIndex(2)),
    Message::MoveFocus(MoveDir::Up, 1, true),
    Message::ItemColorChange(MessageTarget::CurrentSelection, Some("Blue".to_string())),
]}

snapshot_ui_with_file_and_msgs! {framebuffer_no_cursor, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::SetFrameBufferVisibleVariable(Some(VisibleItemIndex(1)))
]}

snapshot_ui_with_file_and_msgs! {framebuffer_cursor, "examples/picorv32.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["testbench"]), false),
    Message::SetFrameBufferVisibleVariable(Some(VisibleItemIndex(2))),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(4700000)))
]}

snapshot_ui_with_file_and_msgs! {framebuffer_array, "examples/manytypes2.fst", [
    Message::AddScopeAsGroup(ScopeRef::from_hierarchy_string("comprehensive2_tb.array_signal"), false),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(470000000))),
    Message::SetFrameBufferArray(ScopeRef::from_hierarchy_string("comprehensive2_tb.array_signal"))
]}

snapshot_ui_with_file_and_msgs! {framebuffer_array_no_need_to_display, "examples/manytypes2.fst", [
    Message::AddVariables(vec![VariableRef::from_hierarchy_string("comprehensive2_tb.bit_signal")]),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(470000000))),
    Message::SetFrameBufferArray(ScopeRef::from_hierarchy_string("comprehensive2_tb.array_signal"))
]}

snapshot_ui_with_file_and_msgs! {framebuffer_multidimensional_array, "examples/arrays_nvc.fst", [
    Message::AddVariables(vec![VariableRef::from_hierarchy_string("arrays_testbench.arr_1d.[1]")]),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(25000000))),
    Message::SetFrameBufferArray(ScopeRef::from_hierarchy_string("arrays_testbench.arr_1d_2d_as_3d"))
]}

snapshot_ui_with_file_and_msgs! {framebuffer_rgb, "examples/smallsurfer.vcd", [
    Message::AddVariables(vec![VariableRef::from_hierarchy_string("image_memory.height")]),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(0))),
    Message::SetFrameBufferArray(ScopeRef::from_hierarchy_string("image_memory.mem")),
    Message::SetFrameBufferMode(crate::frame_buffer::FrameBufferColorMode::Rgb, 8, 8, 8),
    Message::SetFrameBufferWidth(48),
]}

snapshot_ui_with_file_and_msgs! {framebuffer_ycbcr, "examples/smallsurfer.vcd", [
    Message::AddVariables(vec![VariableRef::from_hierarchy_string("image_memory.height")]),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(0))),
    Message::SetFrameBufferArray(ScopeRef::from_hierarchy_string("image_memory.mem")),
    Message::SetFrameBufferMode(crate::frame_buffer::FrameBufferColorMode::YCbCr, 8, 8, 8),
    Message::SetFrameBufferWidth(48),
]}

snapshot_ui_with_file_and_msgs! {memory_viewer_scope_open, "examples/smallsurfer.vcd", [
    Message::AddVariables(vec![VariableRef::from_hierarchy_string("image_memory.height")]),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(0))),
    Message::OpenMemoryViewer {
        scope: ScopeRef::from_hierarchy_string("image_memory.mem"),
        name: Some("image_memory.mem".to_string()),
        placement: None,
    },
]}
snapshot_ui!(regex_error_indication, || {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples")
                    .join("counter.vcd")
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });
    loop {
        state.handle_async_messages();
        state.handle_batch_commands();
        if state.waves_fully_loaded() {
            break;
        }
    }

    let msgs = [
        Message::SetMenuVisible(false),
        Message::SetToolbarVisible(false),
        Message::SetOverviewVisible(false),
        Message::CloseOpenSiblingStateFileDialog {
            load_state: false,
            do_not_show_again: true,
        },
        Message::ToDocument(DocumentCommand::SetActiveScope(Some(ScopeType::WaveScope(
            ScopeRef::from_strs(&["tb"]),
        )))),
        Message::AddVariables(vec![VariableRef::from_hierarchy_string("tb.clk")]),
        Message::SetVariableNameFilterType(VariableNameFilterType::Regex),
    ];
    for message in msgs {
        state.update(message);
    }
    state.user.variable_filter.name_filter_str.push_str("a(");
    // make sure all the signals added by the proceeding messages are properly loaded
    wait_for_waves_fully_loaded(&mut state, 10);
    state
});

snapshot_ui_with_file_and_msgs! {signal_list_works, "examples/counter.vcd", [
    Message::SetSidePanelVisible(true),
    Message::SetShowVariableDirection(false),
    Message::ToDocument(DocumentCommand::SetActiveScope(Some(ScopeType::WaveScope(ScopeRef::from_strs(&["tb"]))))),
    Message::AddVariables(vec![VariableRef::from_hierarchy_string("tb.clk")]),
]}

snapshot_ui_with_file_and_msgs! {draw_from_first_value, "examples/offset_100us.vcd", [
    Message::AddVariables(vec![VariableRef::from_hierarchy_string("testbench.clock"),
                               VariableRef::from_hierarchy_string("testbench.counter"),
                               VariableRef::from_hierarchy_string("testbench.data"),
                               VariableRef::from_hierarchy_string("testbench.state")]),
    Message::SetDefaultTimeline(false),
    Message::SetTimeOffsetEnabled(true)
]}

snapshot_ui_with_file_and_msgs! {draw_from_start_even_without_values, "examples/offset_100us.vcd", [
    Message::AddVariables(vec![VariableRef::from_hierarchy_string("testbench.clock"),
                               VariableRef::from_hierarchy_string("testbench.counter"),
                               VariableRef::from_hierarchy_string("testbench.data"),
                               VariableRef::from_hierarchy_string("testbench.state")]),
    Message::SetDefaultTimeline(false),
    Message::SetTimeOffsetEnabled(false)
]}

snapshot_ui!(fuzzy_signal_filter_works, || {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples")
                    .join("picorv32.vcd")
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });
    loop {
        state.handle_async_messages();
        state.handle_batch_commands();
        if state.waves_fully_loaded() {
            break;
        }
    }

    let msgs = [
        Message::SetMenuVisible(false),
        Message::SetToolbarVisible(false),
        Message::SetOverviewVisible(false),
        Message::SetShowVariableDirection(false),
        Message::CloseOpenSiblingStateFileDialog {
            load_state: false,
            do_not_show_again: true,
        },
        Message::ToDocument(DocumentCommand::SetActiveScope(Some(ScopeType::WaveScope(
            ScopeRef::from_strs(&["testbench", "top", "mem"]),
        )))),
        Message::AddVariables(vec![VariableRef::from_hierarchy_string("testbench.clk")]),
        Message::SetVariableNameFilterType(VariableNameFilterType::Fuzzy),
    ];
    for message in msgs {
        state.update(message);
    }
    state.user.variable_filter.name_filter_str.push_str("at");
    // make sure all the signals added by the proceeding messages are properly loaded
    wait_for_waves_fully_loaded(&mut state, 10);
    state
});

snapshot_ui!(contain_signal_filter_works, || {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples")
                    .join("picorv32.vcd")
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });
    loop {
        state.handle_async_messages();
        state.handle_batch_commands();
        if state.waves_fully_loaded() {
            break;
        }
    }

    let msgs = [
        Message::SetMenuVisible(false),
        Message::SetToolbarVisible(false),
        Message::SetOverviewVisible(false),
        Message::SetShowVariableDirection(false),
        Message::CloseOpenSiblingStateFileDialog {
            load_state: false,
            do_not_show_again: true,
        },
        Message::ToDocument(DocumentCommand::SetActiveScope(Some(ScopeType::WaveScope(
            ScopeRef::from_strs(&["testbench", "top", "mem"]),
        )))),
        Message::AddVariables(vec![VariableRef::from_hierarchy_string("testbench.clk")]),
        Message::SetVariableNameFilterType(VariableNameFilterType::Contain),
    ];
    for message in msgs {
        state.update(message);
    }
    state.user.variable_filter.name_filter_str.push_str("at");
    // make sure all the signals added by the proceeding messages are properly loaded
    wait_for_waves_fully_loaded(&mut state, 10);
    state
});

snapshot_ui!(regex_signal_filter_works, || {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples")
                    .join("picorv32.vcd")
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });
    loop {
        state.handle_async_messages();
        state.handle_batch_commands();
        if state.waves_fully_loaded() {
            break;
        }
    }

    let msgs = [
        Message::SetMenuVisible(false),
        Message::SetToolbarVisible(false),
        Message::SetOverviewVisible(false),
        Message::SetShowVariableDirection(false),
        Message::CloseOpenSiblingStateFileDialog {
            load_state: false,
            do_not_show_again: true,
        },
        Message::ToDocument(DocumentCommand::SetActiveScope(Some(ScopeType::WaveScope(
            ScopeRef::from_strs(&["testbench", "top", "mem"]),
        )))),
        Message::AddVariables(vec![VariableRef::from_hierarchy_string("testbench.clk")]),
        Message::SetVariableNameFilterType(VariableNameFilterType::Regex),
    ];
    for message in msgs {
        state.update(message);
    }
    state.user.variable_filter.name_filter_str.push_str("a[dx]");
    // make sure all the signals added by the proceeding messages are properly loaded
    wait_for_waves_fully_loaded(&mut state, 10);
    state
});

snapshot_ui!(start_signal_filter_works, || {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples")
                    .join("picorv32.vcd")
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });
    loop {
        state.handle_async_messages();
        state.handle_batch_commands();
        if state.waves_fully_loaded() {
            break;
        }
    }

    let msgs = [
        Message::SetMenuVisible(false),
        Message::SetToolbarVisible(false),
        Message::SetOverviewVisible(false),
        Message::SetShowVariableDirection(false),
        Message::CloseOpenSiblingStateFileDialog {
            load_state: false,
            do_not_show_again: true,
        },
        Message::ToDocument(DocumentCommand::SetActiveScope(Some(ScopeType::WaveScope(
            ScopeRef::from_strs(&["testbench", "top", "mem"]),
        )))),
        Message::AddVariables(vec![VariableRef::from_hierarchy_string("testbench.clk")]),
        Message::SetVariableNameFilterType(VariableNameFilterType::Start),
    ];
    for message in msgs {
        state.update(message);
    }
    state.user.variable_filter.name_filter_str.push('a');
    // make sure all the signals added by the proceeding messages are properly loaded
    wait_for_waves_fully_loaded(&mut state, 10);
    state
});

snapshot_ui!(case_sensitive_signal_filter_works, || {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples")
                    .join("picorv32.vcd")
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });
    loop {
        state.handle_async_messages();
        state.handle_batch_commands();
        if state.waves_fully_loaded() {
            break;
        }
    }

    let msgs = [
        Message::SetMenuVisible(false),
        Message::SetToolbarVisible(false),
        Message::SetOverviewVisible(false),
        Message::SetShowVariableDirection(false),
        Message::CloseOpenSiblingStateFileDialog {
            load_state: false,
            do_not_show_again: true,
        },
        Message::ToDocument(DocumentCommand::SetActiveScope(Some(ScopeType::WaveScope(
            ScopeRef::from_strs(&["testbench", "top", "mem"]),
        )))),
        Message::AddVariables(vec![VariableRef::from_hierarchy_string("testbench.clk")]),
        Message::SetVariableNameFilterType(VariableNameFilterType::Start),
        Message::SetVariableNameFilterCaseInsensitive(false),
    ];
    for message in msgs {
        state.update(message);
    }
    state.user.variable_filter.name_filter_str.push('a');
    // make sure all the signals added by the proceeding messages are properly loaded
    wait_for_waves_fully_loaded(&mut state, 10);
    state
});

snapshot_ui!(signal_type_filter_works_1, || {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples")
                    .join("many_sv_datatypes.fst")
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });
    loop {
        state.handle_async_messages();
        state.handle_batch_commands();
        if state.waves_fully_loaded() {
            break;
        }
    }

    let msgs = [
        Message::SetMenuVisible(false),
        Message::SetToolbarVisible(false),
        Message::SetOverviewVisible(false),
        Message::SetShowVariableDirection(false),
        Message::CloseOpenSiblingStateFileDialog {
            load_state: false,
            do_not_show_again: true,
        },
        Message::ToDocument(DocumentCommand::SetActiveScope(Some(ScopeType::WaveScope(
            ScopeRef::from_strs(&["TOP", "SVDataTypeWrapper", "bb"]),
        )))),
        Message::SetVariableIOFilter(VariableIOFilterType::Other, false),
    ];
    for message in msgs {
        state.update(message);
    }
    // make sure all the signals added by the proceeding messages are properly loaded
    wait_for_waves_fully_loaded(&mut state, 10);
    state
});

snapshot_ui!(signal_type_filter_works_2, || {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples")
                    .join("many_sv_datatypes.fst")
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });
    loop {
        state.handle_async_messages();
        state.handle_batch_commands();
        if state.waves_fully_loaded() {
            break;
        }
    }

    let msgs = [
        Message::SetMenuVisible(false),
        Message::SetToolbarVisible(false),
        Message::SetOverviewVisible(false),
        Message::SetShowVariableDirection(false),
        Message::CloseOpenSiblingStateFileDialog {
            load_state: false,
            do_not_show_again: true,
        },
        Message::ToDocument(DocumentCommand::SetActiveScope(Some(ScopeType::WaveScope(
            ScopeRef::from_strs(&["TOP", "SVDataTypeWrapper", "bb"]),
        )))),
        Message::SetVariableIOFilter(VariableIOFilterType::Other, false),
        Message::SetVariableIOFilter(VariableIOFilterType::Output, false),
    ];
    for message in msgs {
        state.update(message);
    }
    // make sure all the signals added by the proceeding messages are properly loaded
    wait_for_waves_fully_loaded(&mut state, 10);
    state
});

snapshot_ui!(signal_type_group_works, || {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples")
                    .join("many_sv_datatypes.fst")
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });
    loop {
        state.handle_async_messages();
        state.handle_batch_commands();
        if state.waves_fully_loaded() {
            break;
        }
    }

    let msgs = [
        Message::SetMenuVisible(false),
        Message::SetToolbarVisible(false),
        Message::SetOverviewVisible(false),
        Message::SetShowVariableDirection(false),
        Message::CloseOpenSiblingStateFileDialog {
            load_state: false,
            do_not_show_again: true,
        },
        Message::ToDocument(DocumentCommand::SetActiveScope(Some(ScopeType::WaveScope(
            ScopeRef::from_strs(&["TOP", "SVDataTypeWrapper", "bb"]),
        )))),
        Message::SetVariableGroupByDirection(true),
    ];
    for message in msgs {
        state.update(message);
    }
    // make sure all the signals added by the proceeding messages are properly loaded
    wait_for_waves_fully_loaded(&mut state, 10);
    state
});

snapshot_ui!(load_keep_all_works, || {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples")
                    .join("xx_1.vcd")
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });
    wait_for_waves_fully_loaded(&mut state, 10);

    let msgs = [
        Message::SetMenuVisible(false),
        Message::SetToolbarVisible(false),
        Message::SetOverviewVisible(false),
        Message::SetSidePanelVisible(false),
        Message::CloseOpenSiblingStateFileDialog {
            load_state: false,
            do_not_show_again: true,
        },
        Message::AddScope(ScopeRef::from_strs(&["TOP"]), false),
        Message::AddScope(ScopeRef::from_strs(&["TOP", "Foobar"]), false),
        Message::LoadFile(
            get_project_root()
                .unwrap()
                .join("examples")
                .join("xx_2.vcd")
                .try_into()
                .unwrap(),
            LoadOptions::KeepAll,
        ),
    ];
    for message in msgs {
        state.update(message);
    }
    loop {
        state.handle_async_messages();
        state.handle_batch_commands();
        if let Some(waves) = &state.user.waves
            && waves.source
                == WaveSource::File(
                    get_project_root()
                        .unwrap()
                        .join("examples")
                        .join("xx_2.vcd")
                        .try_into()
                        .unwrap(),
                )
        {
            break;
        }
    }
    wait_for_waves_fully_loaded(&mut state, 10);
    state
});

snapshot_ui!(load_keep_signal_remove_unavailable_works, || {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples")
                    .join("xx_1.vcd")
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });
    wait_for_waves_fully_loaded(&mut state, 10);

    let msgs = [
        Message::SetMenuVisible(false),
        Message::SetToolbarVisible(false),
        Message::SetOverviewVisible(false),
        Message::SetSidePanelVisible(false),
        Message::CloseOpenSiblingStateFileDialog {
            load_state: false,
            do_not_show_again: true,
        },
        Message::AddScope(ScopeRef::from_strs(&["TOP"]), false),
        Message::AddScope(ScopeRef::from_strs(&["TOP", "Foobar"]), false),
        Message::LoadFile(
            get_project_root()
                .unwrap()
                .join("examples")
                .join("xx_2.vcd")
                .try_into()
                .unwrap(),
            LoadOptions::KeepAvailable,
        ),
    ];
    for message in msgs {
        state.update(message);
    }
    loop {
        state.handle_async_messages();
        state.handle_batch_commands();
        if let Some(waves) = &state.user.waves
            && waves.source
                == WaveSource::File(
                    get_project_root()
                        .unwrap()
                        .join("examples")
                        .join("xx_2.vcd")
                        .try_into()
                        .unwrap(),
                )
        {
            break;
        }
    }
    wait_for_waves_fully_loaded(&mut state, 10);
    state
});

snapshot_ui_with_file_and_msgs! {alignment_right_works, "examples/counter.vcd", [
Message::SetOverviewVisible(true),
Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
Message::SetNameAlignRight(true)
]}

snapshot_ui_with_file_and_msgs! {add_viewport_works, "examples/counter.vcd", [
    Message::Workspace(WorkspaceCommand::SplitTile { tile: TileId(1), dir: TileDirection::Right, mode: SplitMode::Linked }),
    Message::Workspace(WorkspaceCommand::SplitTile { tile: TileId(2), dir: TileDirection::Right, mode: SplitMode::Linked }),
    Message::ToDocument(DocumentCommand::SetActiveScope(Some(ScopeType::WaveScope(ScopeRef::from_strs(&["tb"]))))),
    Message::AddVariables(vec![VariableRef::from_hierarchy_string("tb.clk")]),
    Message::AddTimeLine(None),
]}

/// Build two independent lists in one tab group through the public commands.
fn multi_tab_state() -> (SystemState, crate::tiles::TileId, crate::tiles::TileId) {
    use crate::tiles::{commands::WorkspaceCommand, layout::Placement};
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples/counter.vcd")
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });
    wait_for_waves_fully_loaded(&mut state, 10);
    for message in [
        Message::SetMenuVisible(false),
        Message::SetSidePanelVisible(false),
        Message::SetToolbarVisible(false),
        Message::SetOverviewVisible(false),
    ] {
        state.update(message);
    }
    let first = state.user.workspace.layout().focused().unwrap();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
    ]));
    state.update(Message::Workspace(WorkspaceCommand::RenameTile {
        tile: first,
        title: Some("Clock".into()),
    }));
    state.update(Message::Workspace(WorkspaceCommand::CreateTile {
        kind: "waveform".into(),
        placement: Placement::TabAfter(first),
        focus: true,
    }));
    let second = state.user.workspace.layout().focused().unwrap();
    assert_ne!(first, second);
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.dut.counter"),
    ]));
    state.update(Message::Workspace(WorkspaceCommand::RenameTile {
        tile: second,
        title: Some("Counter".into()),
    }));
    wait_for_waves_fully_loaded(&mut state, 10);
    assert_eq!(state.user.workspace.item_lists().len(), 2);
    (state, first, second)
}

snapshot_ui!(multi_tab_second_active, || multi_tab_state().0);

snapshot_ui!(multi_tab_switch_back, || {
    let (mut state, first, _) = multi_tab_state();
    state.update(Message::Workspace(
        crate::tiles::commands::WorkspaceCommand::FocusTile(first),
    ));
    state
});

snapshot_ui!(multi_tab_close_undo, || {
    let (mut state, _, second) = multi_tab_state();
    state.update(Message::Workspace(
        crate::tiles::commands::WorkspaceCommand::CloseTile(second),
    ));
    assert!(!state.user.workspace.tiles().contains_key(&second));
    state.update(Message::Undo(1));
    assert!(state.user.workspace.tiles().contains_key(&second));
    state.update(Message::Workspace(
        crate::tiles::commands::WorkspaceCommand::FocusTile(second),
    ));
    state
});

/// counter.vcd with the chrome hidden, before any workspace edits.
fn counter_state() -> SystemState {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples/counter.vcd")
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });
    wait_for_waves_fully_loaded(&mut state, 10);
    for message in [
        Message::SetMenuVisible(false),
        Message::SetSidePanelVisible(false),
        Message::SetToolbarVisible(false),
        Message::SetOverviewVisible(false),
    ] {
        state.update(message);
    }
    state
}

fn multi_tab_linked_state() -> SystemState {
    let (mut state, first, second) = multi_tab_state();
    state.update(Message::Workspace(WorkspaceCommand::SplitTile {
        tile: second,
        dir: TileDirection::Right,
        mode: SplitMode::Linked,
    }));
    let linked = state.user.workspace.layout().focused().unwrap();
    assert_ne!(linked, second);
    assert_eq!(state.user.workspace.item_lists().len(), 2);
    state.update(Message::Workspace(WorkspaceCommand::RenameTile {
        tile: linked,
        title: Some("Counter detail".into()),
    }));
    state.update(Message::ToTile(
        linked,
        crate::tiles::kind::TileMessage::Waveform(
            crate::tile_kinds::waveform::WaveformMessage::Navigate(
                crate::tile_kinds::waveform::WaveformNavigation::ZoomToRange {
                    start: 100.into(),
                    end: 200.into(),
                },
            ),
        ),
    ));
    state.update(Message::Workspace(WorkspaceCommand::FocusTile(first)));
    state
}

snapshot_ui!(multi_tab_linked_split, multi_tab_linked_state);

/// A saved workspace installs atomically into a fresh session with the same
/// document and renders exactly like the session that saved it.
#[test]
fn saved_workspace_round_trip_renders_identically() {
    render_and_compare(Utf8Path::new("multi_tab_linked_split"), || {
        let saved = multi_tab_linked_state();
        let encoded = saved.encode_state().unwrap();
        let restored: UserState = crate::tiles::serde::decode(&encoded).unwrap();
        let mut fresh = counter_state();
        fresh.update(Message::Workspace(WorkspaceCommand::RenameTile {
            tile: fresh.user.workspace.layout().focused().unwrap(),
            title: Some("Replaced by the saved workspace".into()),
        }));
        fresh.update(Message::LoadState(Box::new(restored), None));
        // Reattachment resolves variable identities, so compare the workspace
        // contract rather than raw text: layout, tile envelopes and row names.
        let saved_file = saved.user.workspace.to_file().unwrap();
        let loaded_file = fresh.user.workspace.to_file().unwrap();
        assert_eq!(saved_file.layout, loaded_file.layout);
        assert_eq!(
            ron::to_string(&saved_file.tiles).unwrap(),
            ron::to_string(&loaded_file.tiles).unwrap()
        );
        let rows = |workspace: &crate::tiles::workspace::Workspace| {
            workspace
                .item_lists()
                .iter()
                .map(|(id, list)| {
                    (
                        *id,
                        list.items_tree
                            .iter()
                            .map(|node| list.displayed_items[&node.item_ref].name())
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(rows(&saved.user.workspace), rows(&fresh.user.workspace));
        wait_for_waves_fully_loaded(&mut fresh, 10);
        fresh
    });
}

/// The overview strip draws one window per visible waveform tile; clicking a
/// window focuses that tile, preferring the narrowest window under the pointer.
#[test]
fn overview_click_focuses_the_waveform_tile_under_the_pointer() {
    use crate::tiles::layout::Placement;
    // Drive async file loading the same way the snapshot harness does.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let _enter = runtime.enter();
    std::thread::spawn(move || {
        runtime.block_on(async {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
            }
        });
    });
    let (mut state, first, second) = multi_tab_state();
    state.update(Message::Workspace(WorkspaceCommand::MoveTile {
        tile: second,
        to: Placement::Beside(first, TileDirection::Right),
    }));
    state.update(Message::SetOverviewVisible(true));
    state.update(Message::ToTile(
        second,
        crate::tiles::kind::TileMessage::Waveform(
            crate::tile_kinds::waveform::WaveformMessage::Navigate(
                crate::tile_kinds::waveform::WaveformNavigation::ZoomToRange {
                    start: 0.into(),
                    end: 20.into(),
                },
            ),
        ),
    ));
    state.update(Message::Workspace(WorkspaceCommand::FocusTile(second)));
    let ctx = egui::Context::default();
    let size = Vec2::new(800.0, 600.0);
    let frame = |events: Vec<Event>, state: &SystemState| {
        let mut msgs = Vec::new();
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ui| {
                let waves = state.user.waveform_read().unwrap();
                state.add_overview_panel(ui, &waves, &mut msgs);
            },
        );
        output.textures_delta.clear();
        msgs
    };
    let click = |pos: Pos2, pressed: bool| {
        vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            },
        ]
    };
    frame(vec![], &state);
    // The far right lies only inside the full-range window of the first tile.
    let right = Pos2::new(size.x - 3.0, size.y - 3.0);
    frame(click(right, true), &state);
    let msgs = frame(click(right, false), &state);
    assert!(
        matches!(msgs.as_slice(), [Message::Workspace(WorkspaceCommand::FocusTile(id))] if *id == first),
        "expected a focus command, got {msgs:?}"
    );
    // The far left lies inside both windows; the narrower (focused) one wins.
    let left = Pos2::new(3.0, size.y - 3.0);
    frame(click(left, true), &state);
    let msgs = frame(click(left, false), &state);
    assert!(
        !msgs
            .iter()
            .any(|msg| matches!(msg, Message::Workspace(WorkspaceCommand::FocusTile(_)))),
        "unexpected {msgs:?}"
    );
    state.update(Message::Workspace(WorkspaceCommand::FocusTile(first)));
    assert_eq!(state.user.workspace.layout().focused(), Some(first));
}

snapshot_ui!(multi_tab_independent_split, || {
    let (mut state, _, second) = multi_tab_state();
    state.update(Message::Workspace(WorkspaceCommand::SplitTile {
        tile: second,
        dir: TileDirection::Down,
        mode: SplitMode::Independent,
    }));
    let copy = state.user.workspace.layout().focused().unwrap();
    assert_eq!(state.user.workspace.item_lists().len(), 3);
    // The copy owns its list: replacing its rows leaves the original untouched.
    let list = state.user.workspace.tiles()[&copy]
        .kind
        .waveform_list()
        .unwrap();
    let rows = state.user.workspace.item_lists()[&list]
        .displayed_items
        .keys()
        .copied()
        .collect::<Vec<_>>();
    state.update(Message::ToTile(
        copy,
        crate::tiles::kind::TileMessage::Waveform(
            crate::tile_kinds::waveform::WaveformMessage::RemoveItems(rows),
        ),
    ));
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.reset"),
        VariableRef::from_hierarchy_string("tb.overflow"),
    ]));
    state.update(Message::Workspace(WorkspaceCommand::RenameTile {
        tile: copy,
        title: Some("Counter copy".into()),
    }));
    wait_for_waves_fully_loaded(&mut state, 10);
    state
});

snapshot_ui!(multi_tile_every_kind, || {
    use crate::tiles::layout::Placement;
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples/smallsurfer.vcd")
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });
    wait_for_waves_fully_loaded(&mut state, 10);
    for message in [
        Message::SetMenuVisible(false),
        Message::SetSidePanelVisible(false),
        Message::SetToolbarVisible(false),
        Message::SetOverviewVisible(false),
    ] {
        state.update(message);
    }
    let waveform = state.user.workspace.layout().focused().unwrap();
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("image_memory.height"),
        VariableRef::from_hierarchy_string("image_memory.width"),
    ]));
    state.update(Message::ToDocument(DocumentCommand::CursorSet(
        BigInt::from(0),
    )));
    state.update(Message::OpenMemoryViewer {
        scope: ScopeRef::from_hierarchy_string("image_memory.mem"),
        name: Some("image_memory.mem".to_string()),
        placement: None,
    });
    let memory = state.user.workspace.layout().focused().unwrap();
    // An array that this document does not contain renders an unavailable state.
    state.update(Message::OpenMemoryViewer {
        scope: ScopeRef::from_hierarchy_string("image_memory.missing_array"),
        name: Some("missing array".to_string()),
        placement: None,
    });
    let missing = state.user.workspace.layout().focused().unwrap();
    state.update(Message::Workspace(WorkspaceCommand::MoveTile {
        tile: missing,
        to: Placement::Beside(memory, TileDirection::Down),
    }));
    state.update(Message::Workspace(WorkspaceCommand::OpenTile {
        kind: "markers".into(),
        placement: Placement::Beside(waveform, TileDirection::Down),
        focus: false,
    }));
    state.update(Message::SetMarker {
        id: 0,
        time: 5.into(),
    });
    state.update(Message::SetMarker {
        id: 1,
        time: 20.into(),
    });
    state.update(Message::Workspace(WorkspaceCommand::OpenTile {
        kind: "annotation_list".into(),
        placement: Placement::Edge(TileDirection::Right),
        focus: false,
    }));
    let annotations = *state
        .user
        .workspace
        .tiles()
        .iter()
        .find(|(_, entry)| entry.kind.kind_name() == "annotation_list")
        .unwrap()
        .0;
    state.update(Message::Workspace(WorkspaceCommand::OpenTile {
        kind: "frame_buffer".into(),
        placement: Placement::Beside(annotations, TileDirection::Down),
        focus: false,
    }));
    state.update(Message::Workspace(WorkspaceCommand::OpenTile {
        kind: "logs".into(),
        placement: Placement::Edge(TileDirection::Down),
        focus: false,
    }));
    let logs = *state
        .user
        .workspace
        .tiles()
        .iter()
        .find(|(_, entry)| entry.kind.kind_name() == "logs")
        .unwrap()
        .0;
    // Log text contains timings; hide the records so the tile renders deterministically.
    state.update(Message::ToTile(
        logs,
        crate::tiles::kind::TileMessage::Logs(crate::tile_kinds::logs::LogsMessage::SetFilter(
            crate::tile_kinds::logs::LevelFilter::Off,
        )),
    ));
    state.update(Message::Workspace(WorkspaceCommand::FocusTile(memory)));
    wait_for_waves_fully_loaded(&mut state, 10);
    assert_eq!(state.user.workspace.tiles().len(), 7);
    state
});

snapshot_ui_with_file_and_msgs! {hide_single_tab_bar, "examples/counter.vcd", state_mods: (|state: &mut SystemState| {
    assert!(state.user.config.layout.hide_single_tab_bar);
}), [
    Message::AddVariables(vec![VariableRef::from_hierarchy_string("tb.clk")]),
]}

snapshot_ui!(unknown_tile_kind_preserved, || {
    use crate::tiles::{kind::TileEntry, layout::Placement};
    let (mut state, first, _) = multi_tab_state();
    let entry = TileEntry::from_file(
        crate::tiles::serde::decode(include_str!("../tiles/fixtures/future-tile.ron")).unwrap(),
    )
    .unwrap();
    state
        .user
        .workspace
        .insert_prepared(
            &mut state.workspace_runtime,
            entry,
            Default::default(),
            Placement::Beside(first, TileDirection::Right),
            false,
        )
        .unwrap();
    state.update(Message::Workspace(WorkspaceCommand::FocusTile(first)));
    // The envelope survives a save unchanged.
    let saved = state.encode_state().unwrap();
    assert!(saved.contains("future.pipeline"));
    state
});

snapshot_ui!(workspace_reset_keeps_target_waveform, || {
    use crate::tiles::layout::Placement;
    let (mut state, _, second) = multi_tab_state();
    for kind in ["logs", "markers"] {
        state.update(Message::Workspace(WorkspaceCommand::OpenTile {
            kind: kind.into(),
            placement: Placement::Edge(TileDirection::Down),
            focus: false,
        }));
    }
    state.update(Message::Workspace(WorkspaceCommand::FocusTile(second)));
    let reset = state.user.workspace.reset_command();
    state.update(Message::Workspace(reset));
    assert_eq!(
        state
            .user
            .workspace
            .tiles()
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        [second]
    );
    state
});

// A version-zero state file migrates into linked tiles with the legacy
// shared-column look, plus tiles for its open windows; unavailable inspectors
// render their empty states.
snapshot_ui!(legacy_state_file_migrates_into_linked_tiles, || {
    let mut state = counter_state();
    let restored: UserState =
        crate::tiles::serde::decode(include_str!("../tiles/fixtures/legacy-state-v0.ron")).unwrap();
    state.update(Message::LoadState(Box::new(restored), None));
    for message in [
        Message::SetMenuVisible(false),
        Message::SetSidePanelVisible(false),
        Message::SetToolbarVisible(false),
        Message::SetOverviewVisible(false),
    ] {
        state.update(message);
    }
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
        VariableRef::from_hierarchy_string("tb.dut.counter"),
    ]));
    assert_eq!(state.user.workspace.item_lists().len(), 1);
    assert_eq!(state.user.workspace.tiles().len(), 5);
    wait_for_waves_fully_loaded(&mut state, 10);
    state
});

snapshot_ui_with_file_and_msgs! {remove_viewport_works, "examples/counter.vcd", [
    Message::Workspace(WorkspaceCommand::SplitTile { tile: TileId(1), dir: TileDirection::Right, mode: SplitMode::Linked }),
    Message::Workspace(WorkspaceCommand::SplitTile { tile: TileId(2), dir: TileDirection::Right, mode: SplitMode::Linked }),
    Message::ToDocument(DocumentCommand::SetActiveScope(Some(ScopeType::WaveScope(ScopeRef::from_strs(&["tb"]))))),
    Message::AddVariables(vec![VariableRef::from_hierarchy_string("tb.clk")]),
    Message::AddTimeLine(None),
    Message::Workspace(WorkspaceCommand::CloseTile(TileId(3))),
]}

snapshot_ui_with_file_and_msgs! {hierarchy_tree, "examples/counter.vcd", [
    Message::SetSidePanelVisible(true),
    Message::SetHierarchyStyle(HierarchyStyle::Tree),
]}

snapshot_ui_with_file_and_msgs! {hierarchy_variables, "examples/counter.vcd", [
    Message::SetSidePanelVisible(true),
    Message::SetHierarchyStyle(HierarchyStyle::Variables),
]}

snapshot_ui_with_file_and_msgs! {transaction_hierarchy_separate, "examples/my_db.ftr", [
    Message::ToDocument(DocumentCommand::SetActiveScope(Some(ScopeType::StreamScope(StreamScopeRef::Root)))),
    Message::SetSidePanelVisible(true),
    Message::SetHierarchyStyle(HierarchyStyle::Separate),
]}

snapshot_ui_with_file_and_msgs! {transaction_hierarchy_tree, "examples/my_db.ftr", [
    Message::SetSidePanelVisible(true),
    Message::SetHierarchyStyle(HierarchyStyle::Tree),
]}

snapshot_ui_with_file_and_msgs! {transaction_hierarchy_variables, "examples/my_db.ftr", [
    Message::SetSidePanelVisible(true),
    Message::SetHierarchyStyle(HierarchyStyle::Variables),
]}

// makes sure that variables that are not part of a scope are properly displayed in the hierarchy tree view
snapshot_ui_with_file_and_msgs! {hierarchy_tree_with_root_vars, "examples/atxmega256a3u-bmda-jtag_short.vcd", [
    Message::SetSidePanelVisible(true),
    Message::SetShowVariableDirection(false),
    Message::SetHierarchyStyle(HierarchyStyle::Tree),
    Message::AddVariables(vec![
        VariableRef::from_strs(&["tck"]),
        VariableRef::from_strs(&["tms"]),
        VariableRef::from_strs(&["tdi"]),
        VariableRef::from_strs(&["tdo"]),
        VariableRef::from_strs(&["srst"])])
]}

// makes sure that variables that are not part of a scope are properly displayed in the separate hierarchy view
snapshot_ui_with_file_and_msgs! {hierarchy_separate_with_root_vars, "examples/atxmega256a3u-bmda-jtag_short.vcd", [
    Message::SetSidePanelVisible(true),
    Message::SetShowVariableDirection(false),
    Message::AddVariables(vec![
        VariableRef::from_strs(&["tck"]),
        VariableRef::from_strs(&["tms"]),
        VariableRef::from_strs(&["tdi"]),
        VariableRef::from_strs(&["tdo"]),
        VariableRef::from_strs(&["srst"])])
]}

snapshot_ui_with_file_and_msgs! {hierarchy_separate, "examples/counter.vcd", [
    Message::SetSidePanelVisible(true),
    Message::SetHierarchyStyle(HierarchyStyle::Separate),
]}

snapshot_ui_with_file_and_msgs! {fst_scope_and_variable_icons, "examples/fst_types.fst", [
    Message::SetSidePanelVisible(true),
    Message::SetShowHierarchyIcons(true),
    Message::SetHierarchyStyle(HierarchyStyle::Separate),
    Message::ExpandScope(ScopeExpandType::ExpandSpecific(ScopeRef::from_strs(&["rtl"]))),
    Message::ToDocument(DocumentCommand::SetActiveScope(Some(ScopeType::WaveScope(ScopeRef::from_strs(&["top"]))))),
    Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("top.clk"),
        VariableRef::from_hierarchy_string("top.data_reg"),
        VariableRef::from_hierarchy_string("top.trigger_event"),
    ]),
    Message::ExpandScope(ScopeExpandType::ExpandSpecific(ScopeRef::from_strs(&["top"]))),
]}

snapshot_ui_with_file_and_msgs! {vcd_scope_and_variable_icons, "examples/vcd_extensions.vcd", [
    Message::SelectTheme(Some("light+".to_string())),
    Message::SetSidePanelVisible(true),
    Message::SetShowHierarchyIcons(true),
    Message::SetHierarchyStyle(HierarchyStyle::Separate),
    Message::ExpandScope(ScopeExpandType::ExpandSpecific(ScopeRef::from_strs(&["main"]))),
    Message::ToDocument(DocumentCommand::SetActiveScope(Some(ScopeType::WaveScope(ScopeRef::from_strs(&["main"]))))),
    Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("main.EVENT_IN"),
        VariableRef::from_hierarchy_string("main.INT32_OUT"),
        VariableRef::from_hierarchy_string("main.REAL_BUF"),
        VariableRef::from_hierarchy_string("main.WIRE_var"),
    ]),
]}

snapshot_ui_with_file_and_msgs! {aliasing_works_on_random_3_16, "examples/random_3_16_true.vcd", [
    Message::AddVariables(vec![VariableRef::from_hierarchy_string("TOP.LEB128Compressor_3_16.adaptedCounterFlagBits")]),
]}

snapshot_ui_with_file_and_msgs! {next_transition, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(500))),
    Message::FocusItem(VisibleItemIndex(0)),
    Message::MoveCursorToTransition { next: true, variable: None, skip_zero: false }
]}

snapshot_ui_with_file_and_msgs! {next_transition_numbered, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(500))),
    Message::MoveCursorToTransition { next: true, variable: Some(VisibleItemIndex(0)), skip_zero: false }
]}

snapshot_ui_with_file_and_msgs! {next_transition_do_not_get_stuck, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(500))),
    Message::FocusItem(VisibleItemIndex(0)),
    Message::MoveCursorToTransition { next: true, variable: None, skip_zero: false },
    Message::MoveCursorToTransition { next: true, variable: None, skip_zero: false }
]}

snapshot_ui_with_file_and_msgs! {previous_transition, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(500))),
    Message::FocusItem(VisibleItemIndex(0)),
    Message::MoveCursorToTransition { next: false, variable: None, skip_zero: false}
]}

snapshot_ui_with_file_and_msgs! {previous_transition_numbered, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(500))),
    Message::MoveCursorToTransition { next: false, variable: Some(VisibleItemIndex(0)), skip_zero: false }
]}

snapshot_ui_with_file_and_msgs! {previous_transition_do_not_get_stuck, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(500))),
    Message::FocusItem(VisibleItemIndex(0)),
    Message::MoveCursorToTransition { next: false, variable: None, skip_zero: false },
    Message::MoveCursorToTransition { next: false, variable: None, skip_zero: false }
]}

snapshot_ui_with_file_and_msgs! {next_transition_no_cursor, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::FocusItem(VisibleItemIndex(0)),
    Message::MoveCursorToTransition { next: true, variable: None, skip_zero: false },
]}

snapshot_ui_with_file_and_msgs! {previous_transition_no_cursor, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::FocusItem(VisibleItemIndex(0)),
    Message::MoveCursorToTransition { next: false, variable: None, skip_zero: false },
]}

snapshot_ui_with_file_and_msgs! {next_transition_skip_zero, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::FocusItem(VisibleItemIndex(1)),
    Message::MoveCursorToTransition { next: true, variable: None, skip_zero: true },
    Message::MoveCursorToTransition { next: true, variable: None, skip_zero: true }
]}

snapshot_ui_with_file_and_msgs! {previous_transition_skip_zero, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::FocusItem(VisibleItemIndex(1)),
    Message::MoveCursorToTransition { next: false, variable: None, skip_zero: true },
    Message::MoveCursorToTransition { next: false, variable: None, skip_zero: true }
]}

snapshot_ui_with_file_and_msgs! {toggle_variable_indices, "examples/counter.vcd", [
    Message::AddVariables(vec![VariableRef::from_hierarchy_string("tb.dut.counter")]),
    Message::SetShowIndices(false),
]}

snapshot_ui_with_file_and_msgs! {toggle_high_value_fill, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb", "dut"]), false),
    Message::SetFillHighValues(false),
]}

snapshot_ui_with_file_and_msgs! {dinotrace_works, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb", "dut"]), false),
    Message::SetTraceStyle(TraceStyle::Dinotrace),
    Message::ZoomToRange { start: BigInt::from(375), end: BigInt::from(435), tile_id: crate::tiles::TileId(1) }
]}

snapshot_ui_with_file_and_msgs! {zero_trace_works, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb", "dut"]), false),
    Message::SetTraceStyle(TraceStyle::Zero),
    Message::ZoomToRange { start: BigInt::from(375), end: BigInt::from(435), tile_id: crate::tiles::TileId(1) }
]}

snapshot_ui_with_file_and_msgs! {draw_events, "examples/events.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["logic"]), false),
]}

snapshot_ui_with_file_and_msgs! {direction_works, "examples/tb_recv.ghw", [
    Message::SetSidePanelVisible(true),
    Message::ToDocument(DocumentCommand::SetActiveScope(Some(ScopeType::WaveScope(ScopeRef::from_strs(&["tb_recv", "dut"]))))),
    Message::AddVariables(vec![VariableRef::from_hierarchy_string("tb_recv.dut.en")]),
]}

snapshot_ui!(signals_can_be_added_after_file_switch, || {
    let project_root: camino::Utf8PathBuf = get_project_root().unwrap().try_into().unwrap();
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                project_root.join("examples").join("counter.vcd"),
            )),
            ..Default::default()
        });

    wait_for_waves_fully_loaded(&mut state, 10);
    state.update(Message::CloseOpenSiblingStateFileDialog {
        load_state: false,
        do_not_show_again: true,
    });
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetSidePanelVisible(false));
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.dut.counter"),
    ]));
    state.update(Message::LoadFile(
        project_root.join("examples").join("counter2.vcd"),
        LoadOptions::KeepAvailable,
    ));

    loop {
        state.handle_async_messages();
        state.handle_batch_commands();
        if state.user.waves.as_ref().is_some_and(|w| {
            w.source
                .as_file()
                .unwrap()
                .ends_with("examples/counter2.vcd")
        }) {
            break;
        }
    }

    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
    ]));
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.reset"),
    ]));

    wait_for_waves_fully_loaded(&mut state, 10);

    state
});

/// wait for GUI to converge
#[inline]
pub fn wait_for_waves_fully_loaded(state: &mut SystemState, timeout_s: u64) {
    let load_start = std::time::Instant::now();
    while !(state.waves_fully_loaded() && state.batch_commands_completed()) {
        state.handle_async_messages();
        state.handle_batch_commands();
        if load_start.elapsed().as_secs() > timeout_s {
            panic!("Timeout after {timeout_s}s!");
        }
    }
}

snapshot_ui_with_theme!(theme_dark_high_contrast, "dark-high-contrast");
snapshot_ui_with_theme!(theme_dark_plus, "dark+");
snapshot_ui_with_theme!(theme_default, "default");
snapshot_ui_with_theme!(theme_ibm, "IBM");
snapshot_ui_with_theme!(theme_light_high_contrast, "light-high-contrast");
snapshot_ui_with_theme!(theme_light_plus, "light+");
snapshot_ui_with_theme!(theme_petroff_dark, "Petroff Dark");
snapshot_ui_with_theme!(theme_petroff_light, "Petroff Light");
snapshot_ui_with_theme!(theme_solarized, "Solarized");
snapshot_ui_with_theme!(theme_rose_pine, "Rosé Pine");
snapshot_ui_with_theme!(theme_rose_pine_dawn, "Rosé Pine Dawn");
snapshot_ui_with_theme!(theme_rose_pine_moon, "Rosé Pine Moon");

snapshot_ui_with_file_and_msgs! {undo_redo_works, "examples/counter.vcd", [
    Message::AddVariables(vec![]),
    Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.dut.counter"),
        VariableRef::from_hierarchy_string("tb.dut.clk")]),
    Message::Undo(1),
    Message::Redo(1),
    Message::Undo(1),
    Message::AddVariables(vec![VariableRef::from_hierarchy_string("tb.dut.reset")]),
    Message::AddVariables(vec![VariableRef::from_hierarchy_string("tb.dut.reset")]),
    Message::AddVariables(vec![VariableRef::from_hierarchy_string("tb.dut.reset")]),
    Message::Undo(2),
    Message::AddVariables(vec![VariableRef::from_hierarchy_string("tb.dut.reset")]),
    // the redo stack is cleared when something is added to the view
    Message::Redo(1)
]}

snapshot_ui!(rising_clock_markers, || {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples/counter.vcd")
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });
    loop {
        state.handle_async_messages();
        state.handle_batch_commands();
        if state.waves_fully_loaded() {
            break;
        }
    }
    state.user.config.theme.clock_rising_marker = true;
    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetSidePanelVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state.update(Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("tb.clk"),
    ]));
    state.update(Message::VariableFormatChange(
        MessageTarget::Explicit(DisplayedFieldRef {
            item: DisplayedItemRef(1),
            field: vec![],
        }),
        String::from("Clock"),
    ));
    state.update(Message::CanvasZoom {
        mouse_ptr: None,
        delta: 0.5,
        tile_id: crate::tiles::TileId(1),
    });
    wait_for_waves_fully_loaded(&mut state, 10);
    state
});

fn handle_messages_until(
    state: &mut SystemState,
    matcher: impl Fn(&Message) -> bool,
    timeout_s: u64,
) {
    let load_start = std::time::Instant::now();
    loop {
        if load_start.elapsed().as_secs() > timeout_s {
            panic!("Timeout waiting for message after {timeout_s}s!");
        }
        let msg = match state.channels.msg_receiver.try_recv() {
            Ok(msg) => msg,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                std::thread::sleep(std::time::Duration::from_millis(10));
                continue;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                panic!("message sender disconnected")
            }
        };

        let end = match &msg {
            Message::DocumentLoadResult(request, message) => {
                *request == state.document_load_request && matcher(message)
            }
            _ => matcher(&msg),
        };

        state.update(msg);

        if end {
            return;
        }
    }
}

snapshot_ui!(save_and_start_with_state, || {
    // FIXME refactor startup code so that we can test the actual code,
    // not with a separate load command like here
    let save_file = Utf8PathBuf::from_path_buf(
        env::temp_dir().join(format!("save_and_start_with_state.{STATE_FILE_EXTENSION}")),
    )
    .unwrap();
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples")
                    .join("with_8_bit.vcd")
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });
    wait_for_waves_fully_loaded(&mut state, 10);

    state.update(Message::AddVariables(
        [
            VariableRef::from_hierarchy_string("logic.data"),
            VariableRef::from_hierarchy_string("logic.data_valid"),
            VariableRef::from_hierarchy_string("logic.not_always"),
        ]
        .into(),
    ));
    state.update(Message::VariableFormatChange(
        MessageTarget::Explicit(DisplayedFieldRef {
            item: DisplayedItemRef(1),
            field: vec![],
        }),
        String::from("Binary"),
    ));
    state.update(Message::ZoomToFit {
        tile_id: crate::tiles::TileId(1),
    });

    state.handle_async_messages();

    state.update(Message::SaveStateFile(Some(save_file.clone())));

    handle_messages_until(
        &mut state,
        |msg| matches!(&msg, Message::AsyncDone(AsyncJob::SaveState)),
        10,
    );

    state.update(Message::VariableFormatChange(
        MessageTarget::Explicit(DisplayedFieldRef {
            item: DisplayedItemRef(2),
            field: vec![],
        }),
        String::from("RV32"),
    ));

    state.handle_async_messages();

    state.update(Message::SaveStateFile(state.user.state_file.clone()));

    handle_messages_until(
        &mut state,
        |msg| matches!(&msg, Message::AsyncDone(AsyncJob::SaveState)),
        1,
    );

    let mut state = SystemState::new_default_config().unwrap();

    // set shared state from file
    state.user = std::fs::read_to_string(&save_file)
        .map(|content| ron::from_str::<UserState>(&content).unwrap())
        .unwrap();

    // overwrite with startup params
    let mut state = state.with_params(StartupParams {
        waves: Some(WaveSource::File(
            get_project_root()
                .unwrap()
                .join("examples")
                .join("with_8_bit.vcd")
                .try_into()
                .unwrap(),
        )),
        ..Default::default()
    });

    // overwrite with default config
    state.user.config = SurferConfig::new(true).unwrap();

    wait_for_waves_fully_loaded(&mut state, 10);

    std::fs::remove_file(save_file).unwrap();

    state
});

snapshot_ui!(switch, || {
    // check that variables are kept, not available ones as well
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples")
                    .join("with_8_bit.vcd")
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });

    wait_for_waves_fully_loaded(&mut state, 10);

    state.update(Message::AddVariables(
        [
            VariableRef::from_hierarchy_string("logic.data"),
            VariableRef::from_hierarchy_string("logic.data_valid"),
            VariableRef::from_hierarchy_string("logic.not_always"),
        ]
        .into(),
    ));

    handle_messages_until(
        &mut state,
        |msg| matches!(&msg, Message::SignalsLoaded(..)),
        10,
    );

    state.update(Message::VariableFormatChange(
        MessageTarget::Explicit(DisplayedFieldRef {
            item: DisplayedItemRef(1),
            field: vec![],
        }),
        String::from("Binary"),
    ));
    state.update(Message::VariableFormatChange(
        MessageTarget::Explicit(DisplayedFieldRef {
            item: DisplayedItemRef(3),
            field: vec![],
        }),
        String::from("Hexadecimal"),
    ));
    state.update(Message::ZoomToFit {
        tile_id: crate::tiles::TileId(1),
    });
    state.update(Message::LoadFile(
        get_project_root()
            .unwrap()
            .join("examples")
            .join("with_1_bit.vcd")
            .try_into()
            .unwrap(),
        LoadOptions::KeepAll,
    ));

    handle_messages_until(
        &mut state,
        |msg| matches!(&msg, Message::WaveBodyLoaded(..)),
        10,
    );
    handle_messages_until(
        &mut state,
        |msg| matches!(&msg, Message::SignalsLoaded(..)),
        10,
    );

    state
});

snapshot_ui!(switch_and_switch_back, || {
    // verify that decoder settings are remembered even if not recommended / not available
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples")
                    .join("with_8_bit.vcd")
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });

    wait_for_waves_fully_loaded(&mut state, 10);

    state.update(Message::AddVariables(
        [
            VariableRef::from_hierarchy_string("logic.data"),
            VariableRef::from_hierarchy_string("logic.not_always"),
        ]
        .into(),
    ));
    state.update(Message::VariableFormatChange(
        MessageTarget::Explicit(DisplayedFieldRef {
            item: DisplayedItemRef(1),
            field: vec![],
        }),
        String::from("RV32"),
    ));
    state.update(Message::VariableFormatChange(
        MessageTarget::Explicit(DisplayedFieldRef {
            item: DisplayedItemRef(2),
            field: vec![],
        }),
        String::from("Hexadecimal"),
    ));
    state.update(Message::ZoomToFit {
        tile_id: crate::tiles::TileId(1),
    });

    handle_messages_until(
        &mut state,
        |msg| matches!(&msg, Message::SignalsLoaded(..)),
        10,
    );

    state.update(Message::LoadFile(
        get_project_root()
            .unwrap()
            .join("examples")
            .join("with_1_bit.vcd")
            .try_into()
            .unwrap(),
        LoadOptions::KeepAll,
    ));

    handle_messages_until(
        &mut state,
        |msg| matches!(&msg, Message::SignalsLoaded(..)),
        10,
    );

    state.update(Message::LoadFile(
        get_project_root()
            .unwrap()
            .join("examples")
            .join("with_8_bit.vcd")
            .try_into()
            .unwrap(),
        LoadOptions::KeepAll,
    ));
    handle_messages_until(
        &mut state,
        |msg| matches!(&msg, Message::SignalsLoaded(..)),
        10,
    );

    state
});

snapshot_ui!(save_and_load, || {
    let save_file = Utf8PathBuf::from_path_buf(
        env::temp_dir().join(format!("save_and_load.{STATE_FILE_EXTENSION}")),
    )
    .unwrap();
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples")
                    .join("with_8_bit.vcd")
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });

    wait_for_waves_fully_loaded(&mut state, 10);

    state.update(Message::AddVariables(
        [
            VariableRef::from_hierarchy_string("logic.data"),
            VariableRef::from_hierarchy_string("logic.data_valid"),
            VariableRef::from_hierarchy_string("logic.not_always"),
        ]
        .into(),
    ));
    state.update(Message::VariableFormatChange(
        MessageTarget::Explicit(DisplayedFieldRef {
            item: DisplayedItemRef(1),
            field: vec![],
        }),
        String::from("Binary"),
    ));

    state.update(Message::ZoomToFit {
        tile_id: crate::tiles::TileId(1),
    });

    handle_messages_until(
        &mut state,
        |msg| matches!(&msg, Message::SignalsLoaded(..)),
        10,
    );

    state.update(Message::SaveStateFile(Some(save_file.clone())));

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples")
                    .join("with_8_bit.vcd")
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });
    wait_for_waves_fully_loaded(&mut state, 10);

    state.update(Message::LoadStateFile(Some(save_file.clone())));

    handle_messages_until(
        &mut state,
        |msg| matches!(&msg, Message::SignalsLoaded(..)),
        10,
    );

    std::fs::remove_file(save_file).unwrap();

    state
});

snapshot_ui!(export_to_fst_and_reload, || {
    let export_file =
        Utf8PathBuf::from_path_buf(env::temp_dir().join("export_to_fst_and_reload.fst")).unwrap();

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples")
                    .join("picorv32.vcd")
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });

    wait_for_waves_fully_loaded(&mut state, 10);

    // Sparsely select variables from five different branches/depths of the hierarchy to
    // check that only the displayed variables (and only the hierarchy needed to reach
    // them) end up in the exported file.
    state.update(Message::AddVariables(
        [
            VariableRef::from_hierarchy_string("testbench.clk"),
            VariableRef::from_hierarchy_string("testbench.trace_data"),
            VariableRef::from_hierarchy_string("testbench.top.clk"),
            VariableRef::from_hierarchy_string("testbench.top.uut.pcpi_insn"),
            VariableRef::from_hierarchy_string("testbench.top.uut.picorv32_core.mem_do_rinst"),
        ]
        .into(),
    ));

    handle_messages_until(
        &mut state,
        |msg| matches!(&msg, Message::SignalsLoaded(..)),
        10,
    );

    state.update(Message::ExportSignalsToFst(Some(export_file.clone())));

    handle_messages_until(
        &mut state,
        |msg| matches!(&msg, Message::AsyncDone(AsyncJob::ExportFst)),
        10,
    );

    // Load the freshly exported FST file into a new state and show everything in it.
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(export_file.clone())),
            ..Default::default()
        });

    wait_for_waves_fully_loaded(&mut state, 10);

    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetSidePanelVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state.update(Message::CloseOpenSiblingStateFileDialog {
        load_state: false,
        do_not_show_again: true,
    });
    state.update(Message::AddScope(ScopeRef::from_strs(&["testbench"]), true));

    wait_for_waves_fully_loaded(&mut state, 10);

    std::fs::remove_file(export_file).unwrap();

    state
});

#[cfg(feature = "python")]
snapshot_ui_with_file_and_msgs!(
    python_example_translator,
    "examples/with_8_bit.vcd",
    [
        Message::AddScope(ScopeRef::from_strs(&["logic"]), false),
        Message::LoadPythonTranslator(
            get_project_root()
                .unwrap()
                .join("examples")
                .join("hexadecimal.py")
                .try_into()
                .unwrap()
        ),
        Message::VariableFormatChange(
            MessageTarget::Explicit(DisplayedFieldRef {
                item: DisplayedItemRef(1),
                field: vec![],
            }),
            String::from("Hexadecimal (Python)"),
        ),
    ]
);

snapshot_ui_with_file_and_msgs! {simple_ftr_loads, "examples/my_db.ftr", [
    Message::AddStreamOrGenerator(TransactionStreamRef::new_stream(StreamId(1), "pipelined_stream".to_string())),
    Message::AddStreamOrGenerator(TransactionStreamRef::new_stream(StreamId(2), "addr_stream".to_string())),
    Message::AddStreamOrGenerator(TransactionStreamRef::new_stream(StreamId(3), "data_stream".to_string())),
    Message::AddDivider(Some("Divider".to_string()), None),
    Message::AddStreamOrGenerator(TransactionStreamRef::new_gen(StreamId(1), GeneratorId(4), "pipelined_stream.read".to_string())),
    Message::AddStreamOrGenerator(TransactionStreamRef::new_gen(StreamId(1), GeneratorId(5), "pipelined_stream.write".to_string())),
    Message::AddStreamOrGenerator(TransactionStreamRef::new_gen(StreamId(2), GeneratorId(6), "addr_stream.addr".to_string())),
    Message::AddStreamOrGenerator(TransactionStreamRef::new_gen(StreamId(3), GeneratorId(7), "data_stream.rdata".to_string())),
    Message::AddStreamOrGenerator(TransactionStreamRef::new_gen(StreamId(3), GeneratorId(8), "data_stream.wdata".to_string())),

]}

snapshot_ui_with_file_and_msgs! {add_stream_from_name, "examples/my_db.ftr", [
    Message::AddStreamOrGeneratorFromName(None, "read".to_string()),
    Message::AddDivider(Some("Add all for root".to_string()), None),
    Message::AddAllFromStreamScope("tr".to_string()),
    Message::AddDivider(Some("Add all for pipelined_stream".to_string()), None),
    Message::AddAllFromStreamScope("tr.pipelined_stream".to_string()),
    Message::MoveTransaction { next: true },
]}

snapshot_ui_with_file_and_msgs! {focus_transaction, "examples/my_db.ftr", [
    Message::AddStreamOrGenerator(TransactionStreamRef::new_stream(StreamId(1), "pipelined_stream".to_string())),
    Message::AddStreamOrGenerator(TransactionStreamRef::new_gen(StreamId(1), GeneratorId(4), "pipelined_stream.read".to_string())),
    Message::AddStreamOrGenerator(TransactionStreamRef::new_gen(StreamId(1), GeneratorId(5), "pipelined_stream.write".to_string())),
    Message::AddStreamOrGenerator(TransactionStreamRef::new_gen(StreamId(2), GeneratorId(6), "addr_stream.addr".to_string())),
    Message::FocusTransaction(
        Some(TransactionRef { id: TransactionId(4) }),
        crate::tiles::TileId(1),
    ),
]}

snapshot_ui_with_file_and_msgs! {tx_stream_multiple_viewport_works, "examples/my_db.ftr", [
    Message::AddStreamOrGenerator(TransactionStreamRef::new_stream(StreamId(1), "pipelined_stream".to_string())),
    Message::AddStreamOrGenerator(TransactionStreamRef::new_stream(StreamId(2), "addr_stream".to_string())),
    Message::AddStreamOrGenerator(TransactionStreamRef::new_stream(StreamId(3), "data_stream".to_string())),
    Message::Workspace(WorkspaceCommand::SplitTile { tile: TileId(1), dir: TileDirection::Right, mode: SplitMode::Linked }),
    Message::CanvasScroll {delta: Vec2::new(-300., 0.),tile_id: crate::tiles::TileId(2)},
    Message::FocusTransaction(Some(TransactionRef { id: TransactionId(34) }), crate::tiles::TileId(1)),
    Message::FocusTransaction(Some(TransactionRef { id: TransactionId(34) }), crate::tiles::TileId(2)),
]}

snapshot_ui_with_file_and_msgs! {parameter_in_scopes, "examples/picorv32.vcd", [
    Message::SetSidePanelVisible(true),
    Message::ExpandParameterSection,
    Message::ToDocument(DocumentCommand::SetActiveScope(Some(ScopeType::WaveScope(ScopeRef::from_strs(&[
        "testbench",
        "top",
    ]))))),
    Message::AddVariables(
        [
            VariableRef::from_hierarchy_string("testbench.top.clk"),
        ]
        .into(),
    )
]}

snapshot_ui_with_file_and_msgs! {parameter_in_variables, "examples/picorv32.vcd", [
    Message::SetSidePanelVisible(true),
    Message::SetParameterDisplayLocation(ParameterDisplayLocation::Variables),
    Message::ToDocument(DocumentCommand::SetActiveScope(Some(ScopeType::WaveScope(ScopeRef::from_strs(&[
        "testbench",
        "top",
    ]))))),
    Message::AddVariables(
        [
            VariableRef::from_hierarchy_string("testbench.top.clk"),
        ]
        .into(),
    )
]}

snapshot_ui_with_file_and_msgs! {parameter_in_variables_expanded, "examples/picorv32.vcd", [
    Message::SetSidePanelVisible(true),
    Message::SetParameterDisplayLocation(ParameterDisplayLocation::Variables),
    Message::ExpandParameterSection,
    Message::ToDocument(DocumentCommand::SetActiveScope(Some(ScopeType::WaveScope(ScopeRef::from_strs(&[
        "testbench",
        "top",
    ]))))),
    Message::AddVariables(
        [
            VariableRef::from_hierarchy_string("testbench.top.clk"),
        ]
        .into(),
    )
]}

snapshot_ui!(arrow_drawing, || {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples")
                    .join("counter.vcd")
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });
    loop {
        state.handle_async_messages();
        state.handle_batch_commands();
        if state.waves_fully_loaded() {
            break;
        }
    }
    state.user.config.theme.clock_rising_marker = true;
    wait_for_waves_fully_loaded(&mut state, 10);
    state.update(Message::AddScope(ScopeRef::from_strs(&["tb"]), false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetSidePanelVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state.update(Message::ZoomToRange {
        start: 0u32.to_bigint().unwrap(),
        end: 100u32.to_bigint().unwrap(),
        tile_id: crate::tiles::TileId(1),
    });

    let mut idxes = state
        .user
        .waveform_read()
        .unwrap()
        .items
        .displayed_items
        .keys()
        .cloned()
        .collect::<Vec<_>>();

    idxes.sort_by_key(|r| r.0);

    state.update(Message::AddGraphic(
        GraphicId(0),
        Graphic::TextArrow {
            from: (
                GrPoint {
                    x: 5u32.to_bigint().unwrap(),
                    y: crate::graphics::GraphicsY {
                        item: idxes[0],
                        anchor: crate::graphics::Anchor::Top,
                    },
                },
                Direction::West,
            ),
            to: (
                GrPoint {
                    x: 10u32.to_bigint().unwrap(),
                    y: crate::graphics::GraphicsY {
                        item: idxes[1],
                        anchor: crate::graphics::Anchor::Top,
                    },
                },
                Direction::East,
            ),
            text: "A".to_string(),
        },
    ));
    state.update(Message::AddGraphic(
        GraphicId(1),
        Graphic::TextArrow {
            from: (
                GrPoint {
                    x: 15u32.to_bigint().unwrap(),
                    y: crate::graphics::GraphicsY {
                        item: idxes[0],
                        anchor: crate::graphics::Anchor::Top,
                    },
                },
                Direction::East,
            ),
            to: (
                GrPoint {
                    x: 20u32.to_bigint().unwrap(),
                    y: crate::graphics::GraphicsY {
                        item: idxes[1],
                        anchor: crate::graphics::Anchor::Bottom,
                    },
                },
                Direction::West,
            ),
            text: "B".to_string(),
        },
    ));
    state.update(Message::AddGraphic(
        GraphicId(2),
        Graphic::TextArrow {
            from: (
                GrPoint {
                    x: 30u32.to_bigint().unwrap(),
                    y: crate::graphics::GraphicsY {
                        item: idxes[0],
                        anchor: crate::graphics::Anchor::Top,
                    },
                },
                Direction::East,
            ),
            to: (
                GrPoint {
                    x: 25u32.to_bigint().unwrap(),
                    y: crate::graphics::GraphicsY {
                        item: idxes[1],
                        anchor: crate::graphics::Anchor::Center,
                    },
                },
                Direction::West,
            ),
            text: "C".to_string(),
        },
    ));
    state.update(Message::AddGraphic(
        GraphicId(3),
        Graphic::TextArrow {
            from: (
                GrPoint {
                    x: 40u32.to_bigint().unwrap(),
                    y: crate::graphics::GraphicsY {
                        item: idxes[1],
                        anchor: crate::graphics::Anchor::Center,
                    },
                },
                Direction::South,
            ),
            to: (
                GrPoint {
                    x: 35u32.to_bigint().unwrap(),
                    y: crate::graphics::GraphicsY {
                        item: idxes[3],
                        anchor: crate::graphics::Anchor::Center,
                    },
                },
                Direction::North,
            ),
            text: "D".to_string(),
        },
    ));
    state.update(Message::AddGraphic(
        GraphicId(4),
        Graphic::TextArrow {
            from: (
                GrPoint {
                    x: 45u32.to_bigint().unwrap(),
                    y: crate::graphics::GraphicsY {
                        item: idxes[3],
                        anchor: crate::graphics::Anchor::Top,
                    },
                },
                Direction::North,
            ),
            to: (
                GrPoint {
                    x: 50u32.to_bigint().unwrap(),
                    y: crate::graphics::GraphicsY {
                        item: idxes[1],
                        anchor: crate::graphics::Anchor::Center,
                    },
                },
                Direction::South,
            ),
            text: "E".to_string(),
        },
    ));
    wait_for_waves_fully_loaded(&mut state, 10);
    state
});

snapshot_ui_with_file_and_msgs! {default_timeline_works, "examples/counter.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["tb"]), false),
    Message::SetDefaultTimeline(false),
]}

snapshot_ui!(command_file_loading_works, || {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams::default());
    state.update(Message::LoadCommandFile(
        get_project_root()
            .unwrap()
            .join("examples")
            .join("counter.sucl")
            .try_into()
            .unwrap(),
    ));
    wait_for_waves_fully_loaded(&mut state, 10);
    state
});

// Failing as the run_command_file command inserts commands after the ones in the current command file
snapshot_ui!(command_file_in_command_file_works, || {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams::default());
    state.update(Message::LoadCommandFile(
        get_project_root()
            .unwrap()
            .join("examples")
            .join("script_running_script.sucl")
            .try_into()
            .unwrap(),
    ));
    wait_for_waves_fully_loaded(&mut state, 10);
    state
});

snapshot_ui!(marker_set_then_remove_by_name, || {
    let wave_path = get_project_root()
        .unwrap()
        .join("examples")
        .join("counter.vcd")
        .try_into()
        .unwrap();
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(wave_path)),
            wcp_initiate: None,
            startup_commands: vec![],
        });

    wait_for_waves_fully_loaded(&mut state, 10);

    state.add_batch_commands(vec![
        "marker_set m0 2000".to_string(),
        "marker_remove m0".to_string(),
    ]);
    wait_for_waves_fully_loaded(&mut state, 10);
    state.update(Message::Workspace(
        crate::tiles::commands::WorkspaceCommand::OpenTile {
            kind: "markers".into(),
            placement: crate::tiles::layout::Placement::Edge(
                crate::tiles::layout::Direction::Right,
            ),
            focus: true,
        },
    ));

    state
});

// FIXME Known to produce unexpected results (does not remove marker)
// as characters after a # are stripped as comments from batch commands
// before reaching marker parser.
snapshot_ui!(marker_set_then_remove_by_number, || {
    let wave_path = get_project_root()
        .unwrap()
        .join("examples/counter.vcd")
        .try_into()
        .unwrap();
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(wave_path)),
            wcp_initiate: None,
            startup_commands: vec![],
        });

    wait_for_waves_fully_loaded(&mut state, 10);

    state.add_batch_commands(vec![
        "marker_set m0 2000".to_string(),
        "marker_set m1 4000".to_string(),
        "marker_remove #0".to_string(),
    ]);
    wait_for_waves_fully_loaded(&mut state, 10);
    state.update(Message::Workspace(
        crate::tiles::commands::WorkspaceCommand::OpenTile {
            kind: "markers".into(),
            placement: crate::tiles::layout::Placement::Edge(
                crate::tiles::layout::Direction::Right,
            ),
            focus: true,
        },
    ));

    state
});

snapshot_ui!(
    marker_remove_then_readd_does_not_create_default_marker,
    || {
        let wave_path = get_project_root()
            .unwrap()
            .join("examples/counter.vcd")
            .try_into()
            .unwrap();
        let mut state = SystemState::new_default_config()
            .unwrap()
            .with_params(StartupParams {
                waves: Some(WaveSource::File(wave_path)),
                wcp_initiate: None,
                startup_commands: vec![],
            });

        wait_for_waves_fully_loaded(&mut state, 10);

        state.add_batch_commands(vec!["marker_set m0 2000".to_string()]);
        wait_for_waves_fully_loaded(&mut state, 10);

        state.add_batch_commands(vec![
            "marker_remove m0".to_string(),
            "marker_set m0 3000".to_string(),
            "marker_remove m0".to_string(),
            "marker_set m0 4000".to_string(),
        ]);
        wait_for_waves_fully_loaded(&mut state, 10);
        state.update(Message::Workspace(
            crate::tiles::commands::WorkspaceCommand::OpenTile {
                kind: "markers".into(),
                placement: crate::tiles::layout::Placement::Edge(
                    crate::tiles::layout::Direction::Right,
                ),
                focus: true,
            },
        ));

        state
    }
);

#[cfg(feature = "wasm_plugins")]
snapshot_ui_with_file_and_msgs! {wasm_translator_works, "examples/picorv32.vcd", [
    Message::SetSidePanelVisible(true),
    Message::LoadWasmTranslator(
        get_project_root()
            .unwrap()
            .join("examples").join("wasm_example_translator.wasm")
            .try_into()
            .unwrap()
    ),
    Message::AddVariables(vec![VariableRef::from_hierarchy_string("testbench.top.uut.pcpi_insn")]),
    Message::AddScope(ScopeRef::from_hierarchy_string("testbench"), false),
    Message::ToDocument(DocumentCommand::SetActiveScope(Some(ScopeType::WaveScope(ScopeRef::from_hierarchy_string("testbench"))))),
    Message::VariableFormatChange(
        MessageTarget::Explicit(DisplayedFieldRef {
            item: DisplayedItemRef(1),
            field: vec![],
        }),
        String::from("Wasm Example Translator"),
    ),
    Message::VariableFormatChange(
        MessageTarget::Explicit(DisplayedFieldRef {
            item: DisplayedItemRef(1),
            field: vec!["[6]".to_string()],
        }),
        String::from("Binary"),
    ),
    Message::ExpandDrawnItem { item: DisplayedItemRef(1), levels: 1 },
    Message::ToDocument(DocumentCommand::CursorSet(BigInt::from(5000000)))
]}

snapshot_ui_with_file_and_msgs! {analog_waveform_with_4state, "examples/analog.vcd", [
    Message::SelectTheme(Some("light+".to_string())),
    Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("top.clk_cnt"),
        VariableRef::from_hierarchy_string("top.sine_4state"),
        VariableRef::from_hierarchy_string("top.sine_4state"),
        VariableRef::from_hierarchy_string("top.sine_real"),
    ]),

    Message::VariableFormatChange(
        MessageTarget::Explicit(DisplayedFieldRef {
            item: DisplayedItemRef(2),
            field: vec![],
        }),
        String::from("Unsigned"),
    ),

    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Interpolated, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Viewport }),
    ),
    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(1)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Step, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Viewport }),
    ),
    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(3)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Step, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Viewport }),
    ),
    Message::ItemHeightScalingFactorChange(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        8.0,
    ),
    Message::ItemHeightScalingFactorChange(
        MessageTarget::Explicit(VisibleItemIndex(1)),
        4.0,
    ),
    Message::ItemHeightScalingFactorChange(
        MessageTarget::Explicit(VisibleItemIndex(3)),
        2.0,
    ),
]}

// Test interpolation with valid values on both edges (no NaN)
snapshot_ui_with_file_and_msgs! {analog_waveform_interpolate_full, "examples/analog.vcd", [
    Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("top.sine_4state"),
    ]),

    // Configure analog interpolated mode
    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Interpolated, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Viewport }),
    ),

    Message::ItemHeightScalingFactorChange(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        8.0,
    ),

    Message::ZoomToRange {
        start: BigInt::from(7500),
        end: BigInt::from(8500),
        tile_id: crate::tiles::TileId(1)
    },
]}

snapshot_ui_with_file_and_msgs! {analog_waveform_interpolate_nan, "examples/analog.vcd", [
    Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("top.sine_4state"),
    ]),

    // Configure analog interpolated mode
    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Interpolated, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Viewport }),
    ),

    Message::ItemHeightScalingFactorChange(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        8.0,
    ),

    Message::ZoomToRange {
        start: BigInt::from(16300),
        end: BigInt::from(36700),
        tile_id: crate::tiles::TileId(1)
    },
]}

snapshot_ui_with_file_and_msgs! {analog_waveform_interpolate_nan_at_start, "examples/analog.vcd", [
    Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("top.sine_4state"),
    ]),

    // Configure analog interpolated mode
    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Interpolated, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Viewport }),
    ),

    Message::ItemHeightScalingFactorChange(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        8.0,
    ),

    // Zoom to viewport where X region (at 68000) is near the start
    Message::ZoomToRange {
        start: BigInt::from(77000),
        end: BigInt::from(87000),
        tile_id: crate::tiles::TileId(1)
    },
]}

snapshot_ui_with_file_and_msgs! {analog_waveform_interpolate_at_start_range, "examples/analog.vcd", [
    Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("top.sine_4state"),
        VariableRef::from_hierarchy_string("top.sine_4state"),
    ]),

    // Configure analog interpolated mode
    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Interpolated, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Viewport }),
    ),

    Message::ItemHeightScalingFactorChange(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        8.0,
    ),

    Message::ZoomToRange {
        start: BigInt::from(0),
        end: BigInt::from(1400),
        tile_id: crate::tiles::TileId(1)
    },
]}

snapshot_ui_with_file_and_msgs! {analog_waveform_scroll_negative, "examples/analog.vcd", [
    Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("top.sine_4state"),
        VariableRef::from_hierarchy_string("top.sine_4state"),
        VariableRef::from_hierarchy_string("top.sine_4state"),
        VariableRef::from_hierarchy_string("top.sine_4state"),
    ]),

    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Step, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Global }),
    ),
    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(1)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Step, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Viewport }),
    ),
    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(2)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Interpolated, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Global }),
    ),
    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(3)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Interpolated, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Viewport }),
    ),
    Message::AddTimeLine(None),

    Message::CanvasScroll { delta: Vec2 { x: 500., y: 0.}, tile_id: crate::tiles::TileId(1) }

]}

snapshot_ui_with_file_and_msgs! {analog_pulses_no_aliasing1, "examples/analog_pulses.vcd", [
    Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("top.pulse_int"),
        VariableRef::from_hierarchy_string("top.pulse_int"),
    ]),
    Message::AddTimeLine(None),

    Message::VariableFormatChange(
        MessageTarget::Explicit(DisplayedFieldRef {
            item: DisplayedItemRef(0),
            field: vec![],
        }),
        String::from("Unsigned"),
    ),

    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Step, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Viewport }),
    ),

    // Make it a bit taller so the analog shape is clear in the snapshot
    Message::ItemHeightScalingFactorChange(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        16.0,
    ),
]}

snapshot_ui_with_file_and_msgs! {analog_pulses_interpolate_to_range, "examples/analog_pulses.vcd", [
    Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("top.pulse_int"),
        VariableRef::from_hierarchy_string("top.pulse_int"),
    ]),
    Message::AddTimeLine(None),
    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Interpolated, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Global }),
    ),

    Message::ItemHeightScalingFactorChange(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        16.0,
    ),

    Message::ZoomToRange {
        start: BigInt::from(1700000),
        end: BigInt::from(2100000),
        tile_id: crate::tiles::TileId(1)
    },

]}

snapshot_ui_with_file_and_msgs! {analog_pulses_no_aliasing2, "examples/analog_pulses.vcd", [
    Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("top.pulse_int"),
        VariableRef::from_hierarchy_string("top.pulse_int"),
    ]),

    Message::VariableFormatChange(
        MessageTarget::Explicit(DisplayedFieldRef {
            item: DisplayedItemRef(0),
            field: vec![],
        }),
        String::from("Unsigned"),
    ),

    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Step, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Viewport }),
    ),

    Message::ItemHeightScalingFactorChange(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        16.0,
    ),

    Message::ZoomToRange {
        start: BigInt::from(800000),
        end: BigInt::from(1500000),
        tile_id: crate::tiles::TileId(1)
    },
]}

snapshot_ui_with_file_and_msgs! {analog_pulses_4state, "examples/analog_pulses.vcd", [
    Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("top.pulse_reg8"),
        VariableRef::from_hierarchy_string("top.pulse_reg8"),
    ]),

    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Step, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Viewport }),
    ),

    Message::ItemHeightScalingFactorChange(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        8.0,
    ),
]}

snapshot_ui_with_file_and_msgs! {analog_pulses_4state_zoom, "examples/analog_pulses.vcd", [
    Message::AddTimeLine(None),
    Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("top.pulse_reg8"),
        VariableRef::from_hierarchy_string("top.pulse_reg8"),
        VariableRef::from_hierarchy_string("top.pulse_reg8"),
    ]),

    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(1)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Step, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Viewport }),
    ),
    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(2)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Interpolated, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Viewport }),
    ),

    Message::SetItemSelected(VisibleItemIndex(1), true),
    Message::SetItemSelected(VisibleItemIndex(2), true),
    Message::ItemHeightScalingFactorChange(
        MessageTarget::CurrentSelection,
        8.0,
    ),

    Message::ZoomToRange {
        start: BigInt::from(1300000),
        end: BigInt::from(1300010),
        tile_id: crate::tiles::TileId(1)
    },
]}

snapshot_ui_with_file_and_msgs! {analog_pulses_4state_zoom2, "examples/analog_pulses.vcd", [
    Message::AddTimeLine(None),
    Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("top.pulse_reg8"),
        VariableRef::from_hierarchy_string("top.pulse_reg8"),
        VariableRef::from_hierarchy_string("top.pulse_reg8"),
        VariableRef::from_hierarchy_string("top.pulse_reg8"),
    ]),

    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(1)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Step, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Viewport }),
    ),
    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(2)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Step, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Global }),
    ),
    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(3)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Interpolated, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Global }),
    ),

    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(4)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Interpolated, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Viewport }),
    ),

    Message::ItemHeightScalingFactorChange(
        MessageTarget::Explicit(VisibleItemIndex(1)),
        4.0,
    ),
    Message::ItemHeightScalingFactorChange(
        MessageTarget::Explicit(VisibleItemIndex(2)),
        4.0,
    ),
    Message::ItemHeightScalingFactorChange(
        MessageTarget::Explicit(VisibleItemIndex(3)),
        4.0,
    ),
    Message::ItemHeightScalingFactorChange(
        MessageTarget::Explicit(VisibleItemIndex(4)),
        4.0,
    ),

    Message::ZoomToRange {
        start: BigInt::from(1100000),
        end: BigInt::from(2300010),
        tile_id: crate::tiles::TileId(1)
    },
]}

snapshot_ui_with_file_and_msgs! {analog_pulses_4state_scroll_subscale, "examples/analog_pulses.vcd", [
    Message::AddTimeLine(None),
    Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("top.pulse_reg8"),
        VariableRef::from_hierarchy_string("top.pulse_reg8"),
    ]),

    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(1)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Interpolated, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Global }),
    ),

    Message::ItemHeightScalingFactorChange(
        MessageTarget::Explicit(VisibleItemIndex(1)),
        4.0,
    ),

    Message::ZoomToRange {
        start: BigInt::from(2000002),
        end: BigInt::from(2000004),
        tile_id: crate::tiles::TileId(1)
    },

    Message::CanvasScroll { delta: Vec2 { x: -50., y: 0.}, tile_id: crate::tiles::TileId(1) }
]}

snapshot_ui_with_file_and_msgs! {analog_waveform_negive_amplitude, "examples/analog_negative.vcd", [
    Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("top.sine_int_neg"),
        VariableRef::from_hierarchy_string("top.sine_int_neg")
    ]),

    // Configure analog modes
    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Step, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Viewport }),
    ),
    Message::ItemHeightScalingFactorChange(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        4.0,
    )
]}

snapshot_ui_with_file_and_msgs! {analog_waveform_ieee_inf_nan, "examples/analog_ieee_inf_nan.vcd", [
    Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("top.reg32"),
        VariableRef::from_hierarchy_string("top.reg32"),
        VariableRef::from_hierarchy_string("top.reg64"),
        VariableRef::from_hierarchy_string("top.reg64")
    ]),
    Message::AddTimeLine(None),
    // Set IEEE 754 floating point translators
    Message::VariableFormatChange(
        MessageTarget::Explicit(DisplayedFieldRef {
            item: DisplayedItemRef(1),
            field: vec![],
        }),
        String::from("FP: 32-bit IEEE 754"),
    ),
    Message::VariableFormatChange(
        MessageTarget::Explicit(DisplayedFieldRef {
            item: DisplayedItemRef(2),
            field: vec![],
        }),
        String::from("FP: 32-bit IEEE 754"),
    ),
    Message::VariableFormatChange(
        MessageTarget::Explicit(DisplayedFieldRef {
            item: DisplayedItemRef(3),
            field: vec![],
        }),
        String::from("FP: 64-bit IEEE 754"),
    ),
    Message::VariableFormatChange(
        MessageTarget::Explicit(DisplayedFieldRef {
            item: DisplayedItemRef(4),
            field: vec![],
        }),
        String::from("FP: 64-bit IEEE 754"),
    ),
    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Step, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Global }),
    ),
    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(2)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Interpolated, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Viewport }),
    ),
    Message::ItemHeightScalingFactorChange(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        4.0,
    ),
    Message::ItemHeightScalingFactorChange(
        MessageTarget::Explicit(VisibleItemIndex(2)),
        4.0,
    )
]}

snapshot_ui_with_file_and_msgs! {analog_waveform_reg1024, "examples/analog.vcd", [
    Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("top.reg1024"),
        VariableRef::from_hierarchy_string("top.reg1024"),
    ]),
    Message::AddTimeLine(None),

    Message::VariableFormatChange(
        MessageTarget::Explicit(DisplayedFieldRef {
            item: DisplayedItemRef(1),
            field: vec![],
        }),
        String::from("Unsigned"),
    ),

    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Interpolated, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Global }),
    ),

    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(1)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Step, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Global }),
    ),

    Message::ItemHeightScalingFactorChange(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        8.0,
    ),

    Message::ItemHeightScalingFactorChange(
        MessageTarget::Explicit(VisibleItemIndex(1)),
        8.0,
    ),

    Message::ZoomToRange {
        start: BigInt::from(0),
        end: BigInt::from(11000),
        tile_id: crate::tiles::TileId(1)
    },
]}

snapshot_ui_with_file_and_msgs! {analog_waveform_type_limits, "examples/analog.vcd", [
    Message::AddVariables(vec![
        VariableRef::from_hierarchy_string("top.counter8"),
        VariableRef::from_hierarchy_string("top.counter8"),
        VariableRef::from_hierarchy_string("top.counter8"),
    ]),

    // First copy: Unsigned translator + step type limits
    Message::VariableFormatChange(
        MessageTarget::Explicit(DisplayedFieldRef {
            item: DisplayedItemRef(1),
            field: vec![],
        }),
        String::from("Unsigned"),
    ),

    // Second copy: Signed translator + interpolated type limits
    Message::VariableFormatChange(
        MessageTarget::Explicit(DisplayedFieldRef {
            item: DisplayedItemRef(2),
            field: vec![],
        }),
        String::from("Signed"),
    ),

    // Third copy: Unsigned translator + step global (for comparison)
    Message::VariableFormatChange(
        MessageTarget::Explicit(DisplayedFieldRef {
            item: DisplayedItemRef(3),
            field: vec![],
        }),
        String::from("Unsigned"),
    ),

    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Step, y_axis_scale: crate::displayed_item::AnalogYAxisScale::TypeLimits }),
    ),

    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(1)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Interpolated, y_axis_scale: crate::displayed_item::AnalogYAxisScale::TypeLimits }),
    ),

    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(2)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Step, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Global }),
    ),

    Message::ItemHeightScalingFactorChange(
        MessageTarget::Explicit(VisibleItemIndex(0)),
        5.0,
    ),
    Message::ItemHeightScalingFactorChange(
        MessageTarget::Explicit(VisibleItemIndex(1)),
        5.0,
    ),
    Message::ItemHeightScalingFactorChange(
        MessageTarget::Explicit(VisibleItemIndex(2)),
        5.0,
    ),
]}

snapshot_ui_with_file_and_msgs! {divider_text_works, "examples/picorv32.vcd", [
    Message::SetSidePanelVisible(true),
    Message::ExpandParameterSection,
    Message::ToDocument(DocumentCommand::SetActiveScope(Some(ScopeType::WaveScope(ScopeRef::from_strs(&[
        "testbench",
        "top",
    ]))))),
    Message::AddDivider(Some("clk".to_string()), None),
    Message::ShowDividerText(true),
    Message::AddVariables(
        [
            VariableRef::from_hierarchy_string("testbench.top.clk"),
        ]
        .into(),
    ),
]}

snapshot_ui_with_file_and_msgs! {draw_vector_as_line, "examples/picorv32.vcd", [
    Message::AddVariables(
        [
            VariableRef::from_hierarchy_string("testbench.trace_data"),
        ]
        .into(),
    ),
    Message::SetDrawVectorUnknownsAsLine(true)
]}

snapshot_ui_with_file_and_msgs! {focus_highlight_line_width_and_brightness_shift, "examples/picorv32.vcd", [
    Message::AddVariables(
        [
            VariableRef::from_hierarchy_string("testbench.clk"),
            VariableRef::from_hierarchy_string("testbench.resetn"),
            VariableRef::from_hierarchy_string("testbench.trace_data"),
        ]
        .into(),
    ),
    Message::ZoomToRange {
        start: 952593.to_bigint().unwrap(),
        end: 2000047.to_bigint().unwrap(),
        tile_id: crate::tiles::TileId(1),
    },
    Message::FocusItem(VisibleItemIndex(1)),
    Message::SetFocusHighlight(FocusHighlight::LineWidthAndBrightnessShift)
]}

snapshot_ui_with_file_and_msgs! {focus_highlight_brightness_shift, "examples/picorv32.vcd", [
    Message::AddVariables(
        [
            VariableRef::from_hierarchy_string("testbench.clk"),
            VariableRef::from_hierarchy_string("testbench.resetn"),
            VariableRef::from_hierarchy_string("testbench.trace_data"),
        ]
        .into(),
    ),
    Message::ZoomToRange {
        start: 952593.to_bigint().unwrap(),
        end: 2000047.to_bigint().unwrap(),
        tile_id: crate::tiles::TileId(1),
    },
    Message::FocusItem(VisibleItemIndex(2)),
    Message::SetFocusHighlight(FocusHighlight::BrightnessShift)
]}

snapshot_ui_with_file_and_msgs! {focus_highlight_brightness_shift_dark_plus_theme, "examples/picorv32.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["testbench", "top"]), false),
    Message::ZoomToRange {
        start: 4965000.to_bigint().unwrap(),
        end: 5842841.to_bigint().unwrap(),
        tile_id: crate::tiles::TileId(1),
    },
    Message::SelectTheme(Some("dark+".to_string())),
    Message::FocusItem(VisibleItemIndex(8)),
    Message::SetFocusHighlight(FocusHighlight::BrightnessShift)
]}

snapshot_ui_with_file_and_msgs! {focus_highlight_brightness_shift_light_plus_theme, "examples/picorv32.vcd", [
    Message::AddScope(ScopeRef::from_strs(&["testbench", "top"]), false),
    Message::ZoomToRange {
        start: 4965000.to_bigint().unwrap(),
        end: 5842841.to_bigint().unwrap(),
        tile_id: crate::tiles::TileId(1),
    },
    Message::SelectTheme(Some("light+".to_string())),
    Message::FocusItem(VisibleItemIndex(16)),
    Message::SetFocusHighlight(FocusHighlight::LineWidthAndBrightnessShift)
]}

snapshot_ui_with_file_and_msgs! {focus_highlight_background, "examples/analog.vcd", [
    Message::AddVariables(
        [
            VariableRef::from_hierarchy_string("top.clk_cnt"),
            VariableRef::from_hierarchy_string("top.sine_real"),
        ]
        .into(),
    ),
    Message::SetAnalogSettings(
        MessageTarget::Explicit(VisibleItemIndex(1)),
        Some(crate::displayed_item::AnalogSettings { render_style: crate::displayed_item::AnalogRenderStyle::Interpolated, y_axis_scale: crate::displayed_item::AnalogYAxisScale::Viewport }),
    ),
    Message::FocusItem(VisibleItemIndex(1)),
    Message::SetFocusHighlight(FocusHighlight::Background)
]}

/// Snapshot test showing the Theme submenu open with radio buttons.
///
/// The standard `snapshot_ui!` / `draw_onto_surface` helpers can't open menus
/// because they don't inject pointer events.  This test uses `EguiSkia`
/// directly so we can click "View" then hover "Theme" to open the submenu,
/// revealing the radio button on the selected theme.
#[test]
fn theme_menu_radio_button() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let _enter = runtime.enter();
    std::thread::spawn(move || {
        runtime.block_on(async {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
            }
        });
    });

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples")
                    .join("counter.vcd")
                    .try_into()
                    .unwrap(),
            )),
            ..Default::default()
        });
    wait_for_waves_fully_loaded(&mut state, 10);

    // Select a non-default theme so the radio button is visible on it.
    state.update(Message::SelectTheme(Some("light+".to_string())));
    // Local theme files must not change the menu captured by this fixture.
    state.user.config.theme = crate::config::SurferTheme::builtin(Some("light+".into())).unwrap();
    state.user.show_menu = Some(true);
    state.user.show_statusbar = Some(false);
    state.user.show_default_timeline = Some(false);

    let screen_rect = Rect::from_min_size(Pos2::ZERO, SNAPSHOT_SIZE);
    let mut surface = create_surface((SNAPSHOT_WIDTH as i32, SNAPSHOT_HEIGHT as i32));
    surface.canvas().clear(egui_skia_renderer::Color::BLACK);

    let mut backend = EguiSkia::new(1.0);
    let mut frame = 0u32;

    backend.run(
        RawInput {
            screen_rect: Some(screen_rect),
            ..Default::default()
        },
        |ui| {
            ui.set_visuals(state.get_visuals());
            setup_custom_font(ui.ctx());
            let msgs = state.draw(ui, Some(SNAPSHOT_SIZE));
            for msg in msgs {
                if matches!(msg, Message::BuildAnalogCache { .. }) {
                    state.update(msg);
                }
            }
            frame += 1;
        },
    );

    // Frame 2: click "View" to open the dropdown.
    let view_pos = Pos2::new(50.0, 10.0);
    backend.run(
        RawInput {
            screen_rect: Some(screen_rect),
            events: vec![
                Event::PointerMoved(view_pos),
                Event::PointerButton {
                    pos: view_pos,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::default(),
                },
                Event::PointerButton {
                    pos: view_pos,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::default(),
                },
            ],
            ..Default::default()
        },
        |ui| {
            ui.set_visuals(state.get_visuals());
            setup_custom_font(ui.ctx());
            let msgs = state.draw(ui, Some(SNAPSHOT_SIZE));
            for msg in msgs {
                if matches!(msg, Message::BuildAnalogCache { .. }) {
                    state.update(msg);
                }
            }
        },
    );

    // Frames 3-6: hover over "Theme" to open the submenu (no click, so the
    // View dropdown stays open).  "Theme" is near the bottom of the View
    // dropdown at approximately y=385.
    let theme_pos = Pos2::new(50.0, 385.0);
    for _ in 0..4 {
        backend.run(
            RawInput {
                screen_rect: Some(screen_rect),
                events: vec![Event::PointerMoved(theme_pos)],
                ..Default::default()
            },
            |ui| {
                ui.set_visuals(state.get_visuals());
                setup_custom_font(ui.ctx());
                let msgs = state.draw(ui, Some(SNAPSHOT_SIZE));
                for msg in msgs {
                    if matches!(msg, Message::BuildAnalogCache { .. }) {
                        state.update(msg);
                    }
                }
            },
        );
    }

    backend.paint(surface.canvas());

    let data = surface
        .image_snapshot()
        .encode(None, EncodedImageFormat::PNG, None)
        .expect("Failed to encode image");
    let new = image::load_from_memory(&data).expect("Failed to decode png");

    compare_with_snapshot(&Utf8PathBuf::from("theme_menu_radio_button"), &new);
}
