//! Source tile scenarios on the checked-in Verilator examples.
//!
//! Everything the tile shows comes from the VDB companion beside each recording, so
//! these helpers open files in an instance context and deliver modified clicks as
//! egui pointer events, exactly as a user would. No process is started.

use camino::{Utf8Path, Utf8PathBuf};
use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect};
use project_root::get_project_root;

use super::snapshot::{SNAPSHOT_SIZE, wait_for_waves_fully_loaded};
use crate::StartupParams;
use crate::message::Message;
use crate::setup_custom_font;
use crate::source_code::GUTTER;
use crate::source_code::values::ValueState;
use crate::system_state::SystemState;
use crate::wave_source::WaveSource;

pub(crate) fn examples_dir() -> Utf8PathBuf {
    Utf8PathBuf::from_path_buf(get_project_root().unwrap())
        .unwrap()
        .join("examples/verilator")
}

pub(crate) fn source_file(name: &str) -> Utf8PathBuf {
    examples_dir().join(format!("{name}.sv"))
}

/// Loads a recording with its VDB companion.
pub(crate) fn load_trace(trace: Utf8PathBuf) -> SystemState {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(trace)),
            ..Default::default()
        });
    wait_for_waves_fully_loaded(&mut state, 10);
    state
}

/// Loads a Verilator example with its VDB companion.
pub(crate) fn load_example(name: &str) -> SystemState {
    load_trace(source_file(name).with_extension("vtr"))
}

/// A four-state twin of the features recording in which `count` of the wrapping
/// lane goes undefined at time 5, written beside copies of the example's VDB and
/// sources. Verilator only records two-state values, so this is how the tile's
/// unknown-value rendering is exercised on the features design.
pub(crate) fn four_state_features() -> Utf8PathBuf {
    use vtr::{Direction, ScopeType, SignalKind, VarType};
    let examples = examples_dir();
    let root = Utf8PathBuf::from_path_buf(get_project_root().unwrap())
        .unwrap()
        .join("target/source_tests/four_state");
    std::fs::create_dir_all(root.join("include")).unwrap();
    for file in ["features.sv", "features.vdb", "include/features_defs.svh"] {
        std::fs::copy(examples.join(file), root.join(file)).unwrap();
    }
    let vdb: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(examples.join("features.vdb")).unwrap())
            .unwrap();
    let design_id = vdb["design_id"].as_str().unwrap().to_owned();
    let trace = root.join("features.vtr");
    let mut writer = vtr::Writer::create(trace.as_std_path()).unwrap();
    writer.set_timescale(-12).unwrap();
    let id = writer.intern(&design_id);
    writer
        .set_file_attr("design.vdb_id", vtr::Value::Str(id))
        .unwrap();
    let logic = |width| SignalKind::Bits { width, states: 4 };
    let var = |writer: &mut vtr::Writer, name: &str, width: u32, direction: Direction| {
        writer
            .add_var(name, VarType::Wire, direction, logic(width))
            .1
    };
    writer.begin_scope("TOP", ScopeType::Module, "");
    let top_ports = |writer: &mut vtr::Writer| {
        [
            var(writer, "clk", 1, Direction::Input),
            var(writer, "rst_n", 1, Direction::Input),
            var(writer, "en", 1, Direction::Input),
            var(writer, "mode", 2, Direction::Input),
            var(writer, "out", 8, Direction::Output),
        ]
    };
    let mut ports = top_ports(&mut writer).to_vec();
    writer.begin_scope("top", ScopeType::Module, "top");
    ports.extend(top_ports(&mut writer));
    let state = var(&mut writer, "state", 2, Direction::Implicit);
    let pkt = var(&mut writer, "pkt", 5, Direction::Implicit);
    let lanes = [
        var(&mut writer, "lane_count[0]", 8, Direction::Implicit),
        var(&mut writer, "lane_count[1]", 8, Direction::Implicit),
    ];
    let seen = var(&mut writer, "seen", 8, Direction::Implicit);
    let mut counters = Vec::new();
    for lane in 0..2 {
        writer.begin_scope(&format!("g_lane[{lane}]"), ScopeType::Generate, "");
        writer.begin_scope("u", ScopeType::Module, "counter");
        ports.push(var(&mut writer, "clk", 1, Direction::Input));
        ports.push(var(&mut writer, "rst_n", 1, Direction::Input));
        ports.push(var(&mut writer, "en", 1, Direction::Input));
        counters.push(var(&mut writer, "count", 8, Direction::Output));
        writer.end_scope().unwrap();
        writer.end_scope().unwrap();
    }
    writer.end_scope().unwrap();
    writer.end_scope().unwrap();
    let emit = |writer: &mut vtr::Writer, time, values: &[(vtr::SignalId, &str)]| {
        writer.set_time(time).unwrap();
        for (signal, value) in values {
            writer.emit_logic_str(*signal, value.as_bytes()).unwrap();
        }
    };
    let mut initial: Vec<(vtr::SignalId, &str)> = ports
        .iter()
        .map(|signal| (*signal, "0"))
        .chain([(state, "00"), (pkt, "00001"), (seen, "00000000")])
        .chain(lanes.iter().map(|signal| (*signal, "00000000")))
        .collect();
    initial.push((counters[0], "00000000"));
    initial.push((counters[1], "00000000"));
    emit(&mut writer, 0, &initial);
    emit(&mut writer, 5, &[(counters[0], "xxxxxxxx")]);
    emit(&mut writer, 10, &[(counters[0], "00000010"), (state, "01")]);
    writer.close().unwrap();
    trace
}

/// Zero-based number of the first line containing `needle`.
pub(crate) fn line_of(text: &str, needle: &str) -> u32 {
    text.lines()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} not found")) as u32
}

/// Character index of the `occurrence`-th whole-word `token` in `line_text`.
pub(crate) fn token_column(line_text: &str, token: &str, occurrence: usize) -> usize {
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let byte = line_text
        .match_indices(token)
        .filter(|(start, _)| {
            let before = line_text[..*start].chars().next_back();
            let after = line_text[start + token.len()..].chars().next();
            !before.is_some_and(is_word) && !after.is_some_and(is_word)
        })
        .nth(occurrence)
        .map(|(start, _)| start)
        .unwrap_or_else(|| panic!("token {token:?} #{occurrence} not in {line_text:?}"));
    line_text[..byte].chars().count()
}

pub(crate) fn source_tile(state: &SystemState) -> &crate::source_code::SourceCodeTile {
    state
        .user
        .workspace
        .tiles()
        .values()
        .find_map(|entry| match &entry.kind {
            crate::tiles::kind::TileKind::SourceCode(tile) => Some(tile),
            _ => None,
        })
        .expect("source tile open")
}

pub(crate) fn source_tile_id(state: &SystemState) -> crate::tiles::TileId {
    state
        .user
        .workspace
        .tiles()
        .iter()
        .find_map(|(id, entry)| match &entry.kind {
            crate::tiles::kind::TileKind::SourceCode(_) => Some(*id),
            _ => None,
        })
        .expect("source tile open")
}

pub(crate) fn current_file(state: &SystemState) -> Utf8PathBuf {
    source_tile(state)
        .file
        .clone()
        .expect("source tile has a file")
}

pub(crate) fn current_instance(state: &SystemState) -> Option<String> {
    source_tile(state).instance.clone()
}

pub(crate) fn current_line(state: &SystemState) -> u32 {
    source_tile(state).line
}

/// Values the tile shows on the first line containing `needle`, as name, text and
/// state, in order of appearance.
pub(crate) fn values_on(state: &SystemState, needle: &str) -> Vec<(String, String, ValueState)> {
    let text = std::fs::read_to_string(current_file(state)).unwrap();
    source_tile(state).values_on_line(state, line_of(&text, needle))
}

pub(crate) fn open(state: &mut SystemState, file: &Utf8Path, line: u32, instance: Option<&str>) {
    state.update(Message::OpenSource {
        file: file.to_owned(),
        line,
        column: 1,
        instance: instance.map(str::to_owned),
    });
}

/// Runs one frame with the given events and held modifiers, applying the messages
/// the UI produced.
pub(crate) fn frame(
    state: &mut SystemState,
    context: &egui::Context,
    events: Vec<Event>,
    modifiers: Modifiers,
) -> egui::FullOutput {
    let mut events = events;
    events.insert(0, Event::ModifiersChanged(modifiers));
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

/// Screen position of the `occurrence`-th whole-word `token` on the zero-based source
/// `line`, from the shapes of the last frame. Lines are recognized by their gutter,
/// so the values drawn after or between tokens do not matter.
pub(crate) fn source_token_position(
    output: &egui::FullOutput,
    line: u32,
    token: &str,
    occurrence: usize,
) -> Pos2 {
    let gutter = format!("{:>width$}  ", line + 1, width = GUTTER - 2);
    fn find(shape: &epaint::Shape, gutter: &str, token: &str, occurrence: usize) -> Option<Pos2> {
        match shape {
            epaint::Shape::Text(text) if text.galley.job.text.starts_with(gutter) => {
                let column = token_column(&text.galley.job.text, token, occurrence);
                let rect = text
                    .galley
                    .pos_from_cursor(egui::text::CCursor::new(column));
                Some(text.pos + rect.center().to_vec2() + emath::vec2(3.0, 0.0))
            }
            epaint::Shape::Vec(shapes) => shapes
                .iter()
                .find_map(|s| find(s, gutter, token, occurrence)),
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|shape| find(&shape.shape, &gutter, token, occurrence))
        .unwrap_or_else(|| panic!("line {} is not drawn", line + 1))
}

/// Screen position of the center of the widget whose text is exactly `label`.
pub(crate) fn label_position(output: &egui::FullOutput, label: &str) -> Pos2 {
    fn find(shape: &epaint::Shape, label: &str) -> Option<Pos2> {
        match shape {
            epaint::Shape::Text(text) if text.galley.job.text == label => {
                Some(text.pos + text.galley.rect.center().to_vec2())
            }
            epaint::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, label)),
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|shape| find(&shape.shape, label))
        .unwrap_or_else(|| panic!("label {label:?} is not drawn"))
}

/// Presses and releases the primary button at `pos` with `modifiers` held.
pub(crate) fn click_at(
    state: &mut SystemState,
    context: &egui::Context,
    pos: Pos2,
    modifiers: Modifiers,
) {
    frame(state, context, vec![Event::PointerMoved(pos)], modifiers);
    frame(
        state,
        context,
        vec![
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers,
            },
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers,
            },
        ],
        modifiers,
    );
    frame(state, context, vec![], Modifiers::NONE);
}

/// Draws frames until every signal the source tile references is loaded, then
/// re-arms the scroll to the target for a later render in a fresh context.
pub(crate) fn settle(state: &mut SystemState, context: &egui::Context) {
    for _ in 0..3 {
        frame(state, context, vec![], Modifiers::NONE);
        wait_for_waves_fully_loaded(state, 10);
    }
    source_tile(state).rearm_scroll();
}

/// Scrolls the open file to the line containing `needle` and clicks the
/// `occurrence`-th `token` on it with `modifiers` held.
pub(crate) fn click_token(
    state: &mut SystemState,
    context: &egui::Context,
    needle: &str,
    token: &str,
    occurrence: usize,
    modifiers: Modifiers,
) {
    let file = current_file(state);
    let text = std::fs::read_to_string(&file).unwrap();
    let line = line_of(&text, needle);
    let instance = current_instance(state);
    open(state, &file, line + 1, instance.as_deref());
    frame(state, context, vec![], Modifiers::NONE);
    frame(state, context, vec![], Modifiers::NONE);
    let output = frame(state, context, vec![], Modifiers::NONE);
    let pos = source_token_position(&output, line, token, occurrence);
    click_at(state, context, pos, modifiers);
    // Snapshot renderers use a fresh context, so re-arm the scroll to the target.
    let (file, line, instance) = (
        current_file(state),
        current_line(state),
        current_instance(state),
    );
    open(state, &file, line, instance.as_deref());
}

/// Enters a Tokio runtime for tests that pump the async message channel outside
/// `snapshot_ui!`.
pub(crate) fn enter_runtime() -> tokio::runtime::EnterGuard<'static> {
    let runtime: &'static tokio::runtime::Runtime = Box::leak(Box::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap(),
    ));
    let guard = runtime.enter();
    std::thread::spawn(move || {
        runtime.block_on(async {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
            }
        });
    });
    guard
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_word_token_columns() {
        assert_eq!(token_column("count <= count + 1'b1;", "count", 1), 9);
        assert_eq!(token_column("lane_count[i] + count", "count", 0), 16);
        assert_eq!(token_column("é count", "count", 0), 2);
    }
}
