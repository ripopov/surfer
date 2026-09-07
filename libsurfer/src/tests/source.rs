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

/// Loads a Verilator example with its VDB companion.
pub(crate) fn load_example(name: &str) -> SystemState {
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(source_file(name).with_extension("vtr"))),
            ..Default::default()
        });
    wait_for_waves_fully_loaded(&mut state, 10);
    state
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

fn source_tile(state: &SystemState) -> &crate::source_code::SourceCodeTile {
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

/// Screen position of the character at `char_index` (gutter included) on the line whose
/// text ends with `line_text`, from the shapes of the last frame.
pub(crate) fn source_token_position(
    output: &egui::FullOutput,
    line_text: &str,
    char_index: usize,
) -> Pos2 {
    fn find(shape: &epaint::Shape, line_text: &str, char_index: usize) -> Option<Pos2> {
        match shape {
            epaint::Shape::Text(text) if text.galley.job.text.ends_with(line_text) => {
                let rect = text
                    .galley
                    .pos_from_cursor(egui::text::CCursor::new(char_index));
                Some(text.pos + rect.center().to_vec2() + emath::vec2(3.0, 0.0))
            }
            epaint::Shape::Vec(shapes) => {
                shapes.iter().find_map(|s| find(s, line_text, char_index))
            }
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|shape| find(&shape.shape, line_text, char_index))
        .unwrap_or_else(|| panic!("line {line_text:?} is not drawn"))
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
    let line_text = text.lines().nth(line as usize).unwrap().to_owned();
    let column = token_column(&line_text, token, occurrence);
    let instance = current_instance(state);
    open(state, &file, line + 1, instance.as_deref());
    frame(state, context, vec![], Modifiers::NONE);
    frame(state, context, vec![], Modifiers::NONE);
    let output = frame(state, context, vec![], Modifiers::NONE);
    let pos = source_token_position(&output, &line_text, GUTTER + column);
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
