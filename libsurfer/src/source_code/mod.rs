//! Source-code tile.
//!
//! The tile shows one file in the context of one elaborated design instance. Token
//! classes, declarations and the generate blocks an instance leaves uninstantiated
//! come from the static source index of the VDB companion ([`crate::source_index`]);
//! which recorded signal a token denotes, and its value at the waveform cursor, come
//! from the same attachment and the loaded recording. Values are always visible,
//! either trailing the code of each line or as chips after each identifier; hover a
//! symbol for its type and every path it denotes, ctrl-click to navigate, alt-click to
//! add it to the waveform.

pub(crate) mod values;

use camino::Utf8PathBuf;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, RichText, Sense, Stroke, TextWrapMode, Ui};
use serde::{Deserialize, Serialize};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    sync::Arc,
};

use crate::config::{SourceColors, ValuesLayout};
use crate::message::Message;
use crate::source_index::{FileTokens, Modifiers, SourceIndex, SourceLocation, Span, TokenClass};
use crate::system_state::SystemState;
use crate::tiles::kind::TileMessage;
use crate::wave_container::{VariableRef, VariableRefExt};
use values::{Index, LineValues, Shown, ValueState};

const FONT_SIZE: f32 = 13.0;
const VALUE_FONT_SIZE: f32 = 11.0;
/// Characters of line number and padding before the source text of each line.
pub(crate) const GUTTER: usize = 7;
/// Blanks between the end of the code and its trailing values.
const TRAILING_GAP: usize = 4;

/// Settings of the tile that commands and the header change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceCodeMessage {
    /// Where cursor values are drawn.
    ValuesLayout(ValuesLayout),
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceCodeTile {
    pub file: Option<Utf8PathBuf>,
    pub line: u32,
    pub column: u32,
    /// Elaborated design instance the file is viewed in, when known.
    #[serde(default)]
    pub instance: Option<String>,
    /// Value layout chosen in this tile; the config decides while unset.
    #[serde(default)]
    pub values_layout: Option<ValuesLayout>,
    #[serde(skip)]
    document: RefCell<Option<SourceDocument>>,
    #[serde(skip)]
    last_target: Cell<Option<(u32, u32)>>,
    #[serde(skip)]
    notices: RefCell<Vec<String>>,
    #[serde(skip)]
    values: RefCell<ValuesCache>,
    #[serde(skip)]
    demand: RefCell<Option<Demand>>,
}

#[derive(Clone)]
struct SourceDocument {
    file: Utf8PathBuf,
    text: Result<Arc<str>, String>,
}

/// Values of the lines of one file, for one instance, cursor, design and set of
/// waveform formats. Lines still waiting for a signal to load are not kept.
#[derive(Clone, Default)]
struct ValuesCache {
    key: Option<ValuesKey>,
    lines: HashMap<u32, Arc<LineValues>>,
}

#[derive(Clone, PartialEq, Eq)]
struct ValuesKey {
    file: Utf8PathBuf,
    instance: Option<String>,
    cursor: Option<num::BigInt>,
    design: usize,
    formats: u64,
}

/// Recorded signals the file references in the viewed instance, for loading.
#[derive(Clone)]
struct Demand {
    file: Utf8PathBuf,
    instance: Option<String>,
    design: usize,
    signals: Vec<VariableRef>,
}

/// What a modified click on a symbol token asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Intent {
    /// Ctrl-click: open the declaration, or the module of an instance.
    Navigate,
    /// Alt-click: add the recorded signals of the symbol to the waveform.
    AddToWaveform,
}

impl SourceCodeTile {
    pub(crate) fn open(&mut self, location: SourceLocation, instance: Option<String>) {
        if self.file.as_ref() != Some(&location.file) {
            *self.document.borrow_mut() = None;
        }
        self.file = Some(location.file);
        self.line = location.line.max(1);
        self.column = location.column;
        self.instance = instance;
        self.last_target.set(None);
    }

    pub(crate) fn set_instance(&mut self, instance: Option<String>) {
        self.instance = instance;
    }

    /// Scrolls to the target line again on the next frame, for renderers that
    /// draw preparatory frames in another context.
    #[cfg(test)]
    pub(crate) fn rearm_scroll(&self) {
        self.last_target.set(None);
    }

    /// Applies a setting; true when something changed.
    pub(crate) fn update(&mut self, message: SourceCodeMessage) -> bool {
        match message {
            SourceCodeMessage::ValuesLayout(layout) => {
                let changed = self.values_layout != Some(layout);
                self.values_layout = Some(layout);
                changed
            }
        }
    }

    fn layout(&self, state: &SystemState) -> ValuesLayout {
        self.values_layout
            .unwrap_or(state.user.config.source.values_layout)
    }

    fn load(&self, file: &Utf8PathBuf) -> Result<Arc<str>, String> {
        if self
            .document
            .borrow()
            .as_ref()
            .is_none_or(|document| document.file != *file)
        {
            *self.document.borrow_mut() = Some(SourceDocument {
                file: file.clone(),
                text: std::fs::read_to_string(file)
                    .map(Arc::from)
                    .map_err(|error| error.to_string()),
            });
        }
        self.document
            .borrow()
            .as_ref()
            .expect("source document loaded")
            .text
            .clone()
    }

    /// Recorded signals whose values the tile shows, so the document keeps them
    /// loaded while the tile is visible. Computed once per file, instance and design.
    pub(crate) fn demanded_signals(&self, state: &SystemState) -> Vec<VariableRef> {
        let Some(file) = &self.file else {
            return Vec::new();
        };
        let Some(index) = design_index(state) else {
            return Vec::new();
        };
        let design = Arc::as_ptr(&index.database) as usize;
        if let Some(demand) = self.demand.borrow().as_ref()
            && demand.file == *file
            && demand.instance == self.instance
            && demand.design == design
        {
            return demand.signals.clone();
        }
        let view = View::new(self, state, file);
        let mut seen = HashSet::new();
        let mut signals = Vec::new();
        let text = self.load(file).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        if let Some(tokens) = &view.tokens {
            for (line, spans) in tokens.lines() {
                let Some(line_text) = lines.get(line as usize) else {
                    continue;
                };
                for span in spans.iter().filter(|span| values::carries_value(span)) {
                    let Some(token) = line_text.get(span.start as usize..span.end as usize) else {
                        continue;
                    };
                    for symbol in view.symbols_of(span, token) {
                        for path in index.recorded_paths(&symbol) {
                            if seen.insert(path.clone()) {
                                signals.push(VariableRef::from_hierarchy_string(&path));
                            }
                        }
                    }
                }
            }
        }
        *self.demand.borrow_mut() = Some(Demand {
            file: file.clone(),
            instance: self.instance.clone(),
            design,
            signals: signals.clone(),
        });
        signals
    }

    /// Name, text and state of every value shown on a zero-based line, for tests.
    #[cfg(test)]
    pub(crate) fn values_on_line(
        &self,
        state: &SystemState,
        line: u32,
    ) -> Vec<(String, String, ValueState)> {
        let file = self.file.clone().expect("source tile has a file");
        let text = self.load(&file).expect("source readable");
        let view = View::new(self, state, &file);
        let Some(line_text) = text.lines().nth(line as usize) else {
            return Vec::new();
        };
        if view.line_inactive(line, line_text.len() as u32) {
            return Vec::new();
        }
        view.line_values(line, line_text)
            .shown
            .into_iter()
            .map(|shown| (shown.name, shown.text, shown.state))
            .collect()
    }

    pub(crate) fn ui(
        &self,
        ui: &mut Ui,
        id: crate::tiles::TileId,
        state: &SystemState,
        commands: &mut Vec<Message>,
    ) {
        let Some(file) = &self.file else {
            ui.centered_and_justified(|ui| {
                ui.label("Select a signal with VDB source information to open its source.");
            });
            return;
        };
        let contents = match self.load(file) {
            Ok(contents) => contents,
            Err(error) => {
                ui.colored_label(Color32::RED, format!("Unable to read {file}: {error}"));
                return;
            }
        };
        let scroll_to_target = self.last_target.replace(Some((self.line, self.column)))
            != Some((self.line, self.column));
        let view = View::new(self, state, file);
        let layout = self.layout(state);
        self.header(ui, id, state, file, &view, layout, commands);
        ui.separator();
        ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);
        let theme = &state.user.config.theme;
        let target = self.line.saturating_sub(1) as usize;
        let font = FontId::monospace(FONT_SIZE);
        let modifiers = ui.input(|i| i.modifiers);
        let lines: Vec<&str> = contents.lines().collect();
        let row_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
        let row_pitch = row_height + ui.spacing().item_spacing.y;
        let mut cache = self.values.borrow_mut();
        let key = view.values_key();
        if cache.key.as_ref() != key.as_ref() {
            cache.key = key;
            cache.lines.clear();
        }
        let mut scroll = egui::ScrollArea::both().auto_shrink([false, false]);
        if scroll_to_target {
            let offset = target as f32 * row_pitch + row_height / 2.0 - ui.available_height() / 2.0;
            scroll = scroll.vertical_scroll_offset(offset.max(0.0));
        }
        scroll.show_rows(ui, row_height, lines.len(), |ui, rows| {
            for index in rows {
                let text = lines[index];
                let line = index as u32;
                let spans = view.spans(line);
                let line_inactive = view.line_inactive(line, text.len() as u32);
                let values = if line_inactive || spans.is_empty() {
                    None
                } else if let Some(values) = cache.lines.get(&line) {
                    Some(Arc::clone(values))
                } else {
                    let values = Arc::new(view.line_values(line, text));
                    if values.complete {
                        cache.lines.insert(line, Arc::clone(&values));
                    }
                    Some(values)
                };
                let normal = if line_inactive {
                    theme.source.inactive
                } else {
                    ui.visuals().text_color()
                };
                let mut laid = layout_line(
                    &LineInput {
                        number: index + 1,
                        text,
                        spans,
                        values: values.as_deref(),
                        line_inactive,
                        normal,
                        gutter_color: ui.visuals().weak_text_color(),
                        layout,
                        column: state.user.config.source.values_column as usize,
                    },
                    &view,
                    &theme.source,
                    &font,
                );
                let galley = ui.painter().layout_job(std::mem::take(&mut laid.job));
                let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::click());
                if index == target {
                    ui.painter()
                        .rect_filled(rect, 0.0, theme.source.target_line);
                } else if line_inactive {
                    ui.painter()
                        .rect_filled(rect, 0.0, theme.source.inactive_background);
                }
                ui.painter().galley(rect.min, galley.clone(), normal);
                for (start, end, color) in &laid.chips {
                    let from = galley.pos_from_cursor(egui::text::CCursor::new(*start));
                    let to = galley.pos_from_cursor(egui::text::CCursor::new(*end));
                    let chip = egui::Rect::from_min_max(
                        egui::pos2(rect.min.x + from.min.x, rect.min.y + 1.0),
                        egui::pos2(rect.min.x + to.min.x, rect.max.y - 1.0),
                    );
                    ui.painter().rect_stroke(
                        chip,
                        2.0,
                        Stroke::new(1.0, *color),
                        egui::StrokeKind::Inside,
                    );
                }
                // Hover and clicks on symbol tokens.
                let Some(pointer) = response.hover_pos() else {
                    continue;
                };
                let char_index = galley.cursor_from_pos(pointer - rect.min).index.0;
                let Some(Some(byte)) = laid.bytes.get(char_index).copied() else {
                    continue;
                };
                let Some(span) = spans
                    .iter()
                    .find(|span| span.start <= byte as u32 && (byte as u32) < span.end)
                    .copied()
                else {
                    continue;
                };
                if !span.class.is_symbol() {
                    continue;
                }
                let token = &text[span.start as usize..span.end as usize];
                let activate = modifiers.command || modifiers.alt;
                if activate {
                    let from_char = laid.char_of(span.start as usize);
                    let to_char =
                        from_char + text[span.start as usize..span.end as usize].chars().count();
                    let from = galley.pos_from_cursor(egui::text::CCursor::new(from_char));
                    let to = galley.pos_from_cursor(egui::text::CCursor::new(to_char));
                    let y = rect.min.y + from.max.y - 1.0;
                    ui.painter().line_segment(
                        [
                            egui::pos2(rect.min.x + from.min.x, y),
                            egui::pos2(rect.min.x + to.min.x, y),
                        ],
                        Stroke::new(1.0, span_color(&theme.source, &span, normal)),
                    );
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    if response.clicked() {
                        let intent = if modifiers.alt {
                            Intent::AddToWaveform
                        } else {
                            Intent::Navigate
                        };
                        if let Err(notice) = view.activate(&span, token, intent, commands) {
                            self.notices.borrow_mut().push(notice);
                        }
                    }
                } else if state.show_tooltip() {
                    response.on_hover_ui_at_pointer(|ui| {
                        view.hover_ui(ui, token, &span);
                    });
                }
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn header(
        &self,
        ui: &mut Ui,
        id: crate::tiles::TileId,
        state: &SystemState,
        file: &Utf8PathBuf,
        view: &View,
        layout: ValuesLayout,
        commands: &mut Vec<Message>,
    ) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(file.file_name().unwrap_or(file.as_str()))
                    .strong()
                    .monospace(),
            )
            .on_hover_text(file.as_str());
            if self.line > 0 {
                ui.label(format!(":{}:{}", self.line, self.column));
            }
            let siblings = view.sibling_instances();
            if let Some(instance) = &self.instance {
                ui.separator();
                if siblings.len() > 1 {
                    let mut selected = instance.clone();
                    egui::ComboBox::from_id_salt("source_instance")
                        .selected_text(RichText::new(&selected).monospace())
                        .show_ui(ui, |ui| {
                            for sibling in &siblings {
                                ui.selectable_value(
                                    &mut selected,
                                    sibling.clone(),
                                    RichText::new(sibling).monospace(),
                                );
                            }
                        })
                        .response
                        .on_hover_text("Design instance the file is viewed in");
                    if &selected != instance {
                        commands.push(Message::SourceInstance(Some(selected)));
                    }
                } else {
                    ui.label(RichText::new(instance).monospace())
                        .on_hover_text("Design instance the file is viewed in");
                }
            }
            ui.separator();
            let colors = &state.user.config.theme.source;
            match view.cursor_text() {
                Some(time) => {
                    ui.label(
                        RichText::new(format!("@ {time}"))
                            .monospace()
                            .color(colors.value),
                    )
                    .on_hover_text("Waveform cursor; every value on this page is sampled there");
                }
                None => {
                    ui.label(RichText::new("no cursor").weak())
                        .on_hover_text("Place the waveform cursor to see values");
                }
            }
            for (choice, name, hint) in [
                (
                    ValuesLayout::Trailing,
                    "Trailing",
                    "Values after the code of each line",
                ),
                (
                    ValuesLayout::Inline,
                    "Inline",
                    "Values as chips after each identifier",
                ),
            ] {
                let response = ui
                    .selectable_label(layout == choice, RichText::new(name).small())
                    .on_hover_text(hint);
                if response.clicked() && layout != choice {
                    commands.push(Message::ToTile(
                        id,
                        TileMessage::SourceCode(SourceCodeMessage::ValuesLayout(choice)),
                    ));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let status = view.status(state);
                let mut notices = self.notices.borrow_mut();
                if notices.len() > 3 {
                    let drop = notices.len() - 3;
                    notices.drain(..drop);
                }
                let label = ui.label(RichText::new(status.text).color(status.color).small());
                if let Some(notice) = notices.last() {
                    label.on_hover_text(notice.clone());
                    ui.label(RichText::new(notice).small().weak());
                }
            });
        });
    }
}

/// Source index of the loaded design, when the recording has a companion.
fn design_index(state: &SystemState) -> Option<&SourceIndex> {
    state
        .user
        .waves
        .as_ref()
        .and_then(|w| w.inner.as_waves())
        .and_then(|w| w.source_index())
}

/// Everything one line needs to be laid out.
struct LineInput<'a> {
    number: usize,
    text: &'a str,
    spans: &'a [Span],
    values: Option<&'a LineValues>,
    line_inactive: bool,
    normal: Color32,
    gutter_color: Color32,
    layout: ValuesLayout,
    column: usize,
}

/// A laid-out line and the map from its characters back to source bytes.
struct LaidLine {
    job: LayoutJob,
    /// Source byte of each character of the galley; `None` for the gutter and values.
    bytes: Vec<Option<usize>>,
    /// Character ranges of inline chips and their outline color.
    chips: Vec<(usize, usize, Color32)>,
}

impl LaidLine {
    fn append(&mut self, text: &str, font: &FontId, color: Color32, byte: Option<usize>) {
        self.job.append(
            text,
            0.0,
            TextFormat {
                font_id: font.clone(),
                color,
                ..Default::default()
            },
        );
        self.bytes.extend(
            text.char_indices()
                .map(|(offset, _)| byte.map(|byte| byte + offset)),
        );
    }

    fn append_value(&mut self, text: &str, color: Color32, background: Color32, italics: bool) {
        self.job.append(
            text,
            0.0,
            TextFormat {
                font_id: FontId::monospace(VALUE_FONT_SIZE),
                color,
                background,
                italics,
                valign: egui::Align::Center,
                ..Default::default()
            },
        );
        self.bytes.extend(text.chars().map(|_| None));
    }

    /// Character index of the first character drawn for source byte `byte`.
    fn char_of(&self, byte: usize) -> usize {
        self.bytes
            .iter()
            .position(|b| b.is_some_and(|b| b >= byte))
            .unwrap_or(self.bytes.len())
    }
}

fn layout_line(input: &LineInput, view: &View, colors: &SourceColors, font: &FontId) -> LaidLine {
    let mut laid = LaidLine {
        job: LayoutJob::default(),
        bytes: Vec::new(),
        chips: Vec::new(),
    };
    let text = input.text;
    laid.append(
        &format!("{:>width$}  ", input.number, width = GUTTER - 2),
        font,
        input.gutter_color,
        None,
    );
    // Chips are inserted after the byte their value belongs to.
    let mut chips: Vec<&Shown> = match (input.layout, input.values) {
        (ValuesLayout::Inline, Some(values)) => values.shown.iter().collect(),
        _ => Vec::new(),
    };
    chips.sort_by_key(|shown| shown.end);
    let mut next_chip = 0usize;
    let mut emit = |laid: &mut LaidLine, start: usize, end: usize, color: Color32| {
        let mut at = start;
        while next_chip < chips.len() && (chips[next_chip].end as usize) <= end {
            let shown = chips[next_chip];
            let insert = (shown.end as usize).max(at);
            if at < insert {
                laid.append(&text[at..insert], font, color, Some(at));
            }
            let value_color = value_color(colors, shown.state);
            let from = laid.bytes.len();
            laid.append_value(
                &format!(" {} ", shown.text),
                value_color,
                value_color.gamma_multiply(0.15),
                false,
            );
            laid.chips.push((from, laid.bytes.len(), value_color));
            at = insert;
            next_chip += 1;
        }
        if at < end {
            laid.append(&text[at..end], font, color, Some(at));
        }
    };
    let mut at = 0usize;
    for span in input.spans {
        let start = (span.start as usize).min(text.len());
        let end = (span.end as usize).min(text.len());
        if start < at || end <= start {
            continue;
        }
        if at < start {
            emit(&mut laid, at, start, input.normal);
        }
        let color = if input.line_inactive || view.inactive_span(input, span) {
            colors.inactive
        } else {
            span_color(colors, span, input.normal)
        };
        emit(&mut laid, start, end, color);
        at = end;
    }
    if at < text.len() {
        emit(&mut laid, at, text.len(), input.normal);
    }
    if text.is_empty() {
        laid.append(" ", font, input.normal, None);
    }
    if let (ValuesLayout::Trailing, Some(values)) = (input.layout, input.values)
        && !values.shown.is_empty()
    {
        {
            let code_chars = text.chars().count();
            let pad = (code_chars + TRAILING_GAP).max(input.column) - code_chars;
            laid.append(&" ".repeat(pad), font, input.normal, None);
            for (i, shown) in values.shown.iter().enumerate() {
                if i > 0 {
                    laid.append_value("  ", Color32::TRANSPARENT, Color32::TRANSPARENT, true);
                }
                let color = value_color(colors, shown.state);
                laid.append_value(
                    &format!("{} ", shown.name),
                    colors.value.gamma_multiply(0.7),
                    Color32::TRANSPARENT,
                    true,
                );
                laid.append_value(&shown.text, color, Color32::TRANSPARENT, true);
            }
        }
    }
    laid
}

fn value_color(colors: &SourceColors, state: ValueState) -> Color32 {
    match state {
        ValueState::Normal => colors.value,
        ValueState::Changed => colors.value_changed,
        ValueState::Unknown => colors.value_unknown,
        ValueState::Missing => colors.inactive,
    }
}

fn span_color(colors: &SourceColors, span: &Span, normal: Color32) -> Color32 {
    match span.class {
        TokenClass::Keyword => colors.keyword,
        TokenClass::Comment => colors.comment,
        TokenClass::Number => colors.number,
        TokenClass::String => colors.string,
        TokenClass::Operator => colors.operator,
        TokenClass::Macro => colors.macro_,
        TokenClass::Variable => {
            if span.modifiers.contains(Modifiers::CLOCK) {
                colors.clock
            } else if span.modifiers.contains(Modifiers::INOUT) {
                colors.inout
            } else if span.modifiers.contains(Modifiers::OUTPUT) {
                colors.output
            } else if span.modifiers.contains(Modifiers::INPUT) {
                colors.input
            } else {
                colors.variable
            }
        }
        TokenClass::Parameter => colors.parameter,
        TokenClass::EnumMember => colors.enum_member,
        TokenClass::Type => colors.type_,
        TokenClass::Module | TokenClass::Interface | TokenClass::Package => colors.module,
        TokenClass::Instance => colors.instance,
        TokenClass::Function => colors.function,
        TokenClass::Property => colors.property,
        TokenClass::Namespace | TokenClass::Other => normal,
    }
}

struct StatusLine {
    text: String,
    color: Color32,
}

/// A sampled recorded signal at the cursor.
struct Sample {
    raw: surfer_translation_types::VariableValue,
    width: u32,
    text: String,
    unknown: bool,
    changed: bool,
    /// Direct subfields the translator produced, in order.
    fields: Vec<(String, String)>,
}

/// Outcome of resolving one token to a value.
enum Resolved {
    Value(String, ValueState),
    /// The signal is still loading.
    Pending,
    None,
}

/// Everything the draw loop needs from the design for one file and one frame.
struct View<'a> {
    index: Option<&'a SourceIndex>,
    tokens: Option<Arc<FileTokens>>,
    inactive: Vec<vtr_vdb::InactiveRange>,
    state: &'a SystemState,
    instance: Option<String>,
    cursor: Option<num::BigUint>,
}

impl<'a> View<'a> {
    fn new(tile: &SourceCodeTile, state: &'a SystemState, file: &Utf8PathBuf) -> Self {
        let index = design_index(state);
        let tokens = index.and_then(|index| index.file_tokens(file));
        let inactive = index
            .zip(tile.instance.as_deref())
            .map(|(index, instance)| index.inactive_ranges(file, instance))
            .unwrap_or_default();
        let cursor = state
            .user
            .waves
            .as_ref()
            .and_then(|waves| waves.cursor.as_ref())
            .and_then(num::BigInt::to_biguint);
        Self {
            index,
            tokens,
            inactive,
            state,
            instance: tile.instance.clone(),
            cursor,
        }
    }

    fn spans(&self, line: u32) -> &[Span] {
        self.tokens.as_ref().map_or(&[], |tokens| tokens.line(line))
    }

    fn inactive(&self, line: u32, start: u32, end: u32) -> bool {
        // Ranges are one-based; the tile counts lines from zero.
        self.inactive
            .iter()
            .any(|range| range.covers(line + 1, start + 1, end + 1))
    }

    fn inactive_span(&self, input: &LineInput, span: &Span) -> bool {
        self.inactive(input.number as u32 - 1, span.start, span.end)
    }

    /// Whether the whole line (after leading whitespace) lies in an inactive block.
    fn line_inactive(&self, line: u32, length: u32) -> bool {
        length > 0 && self.inactive(line, 0, length)
    }

    fn sibling_instances(&self) -> Vec<String> {
        match (self.index, &self.instance) {
            (Some(index), Some(instance)) => index.sibling_instances(instance),
            _ => Vec::new(),
        }
    }

    fn waves(&self) -> Option<&'a crate::wave_data::WaveData> {
        self.state.user.waves.as_ref()
    }

    /// The cursor formatted like the status bar.
    fn cursor_text(&self) -> Option<String> {
        let waves = self.waves()?;
        let cursor = waves.cursor.as_ref()?;
        Some(crate::time::time_string(
            cursor,
            &waves.inner.metadata().timescale,
            &self.state.user.wanted_timeunit,
            &self.state.get_time_format(),
        ))
    }

    /// Identity of everything the cached line values depend on.
    fn values_key(&self) -> Option<ValuesKey> {
        let index = self.index?;
        let waves = self.waves()?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for list in self.state.user.workspace.item_lists().values() {
            for item in list.displayed_items.values() {
                if let crate::displayed_item::DisplayedItem::Variable(variable) = item {
                    variable.variable_ref.full_path_string().hash(&mut hasher);
                    variable.format.hash(&mut hasher);
                }
            }
        }
        Some(ValuesKey {
            file: self
                .tokens
                .as_ref()
                .map(|_| Utf8PathBuf::new())
                .unwrap_or_default(),
            instance: self.instance.clone(),
            cursor: waves.cursor.clone(),
            design: Arc::as_ptr(&index.database) as usize,
            formats: hasher.finish(),
        })
    }

    fn status(&self, state: &SystemState) -> StatusLine {
        let theme = &state.user.config.theme;
        match (self.index, &self.tokens) {
            (Some(index), Some(tokens)) => StatusLine {
                text: format!(
                    "{} tokens indexed by {}",
                    tokens.len(),
                    index.producer().unwrap_or("unknown")
                ),
                color: theme.accent_info.foreground,
            },
            (Some(index), None) => StatusLine {
                text: if index.producer().is_some() {
                    "file not in the source index".to_owned()
                } else {
                    "VDB has no source index".to_owned()
                },
                color: theme.accent_warn.foreground,
            },
            (None, _) => StatusLine {
                text: "no design database".to_owned(),
                color: theme.alt_text_color,
            },
        }
    }

    /// Elaborated symbol paths declared where `span` (spelled `token`) points, those
    /// under the viewed instance first when any are.
    fn symbols_of(&self, span: &Span, token: &str) -> Vec<String> {
        self.in_context(
            self.declared(span, token)
                .map_or(&[], |d| d.symbols.as_slice()),
        )
    }

    fn instances_of(&self, span: &Span, token: &str) -> Vec<String> {
        self.in_context(
            self.declared(span, token)
                .map_or(&[], |d| d.instances.as_slice()),
        )
    }

    fn declared(&self, span: &Span, token: &str) -> Option<&crate::source_index::Declared> {
        self.index?.declared_named(span.declaration?, token)
    }

    fn in_context(&self, paths: &[String]) -> Vec<String> {
        let under_instance = |path: &String| {
            self.instance.as_deref().is_some_and(|instance| {
                path.strip_prefix(instance)
                    .is_some_and(|rest| rest.starts_with(['.', '[']))
            })
        };
        let inside: Vec<String> = paths
            .iter()
            .filter(|p| under_instance(p))
            .cloned()
            .collect();
        if inside.is_empty() {
            paths.to_vec()
        } else {
            inside
        }
    }

    // --- values ---------------------------------------------------------------

    /// Values of every symbol a line references, in order of first appearance.
    fn line_values(&self, line: u32, text: &str) -> LineValues {
        let mut out = LineValues {
            shown: Vec::new(),
            complete: true,
        };
        if self.index.is_none() || self.cursor.is_none() {
            return out;
        }
        let spans = self.spans(line);
        let mut names = HashSet::new();
        for (i, span) in spans.iter().enumerate() {
            if !values::carries_value(span) || self.inactive(line, span.start, span.end) {
                continue;
            }
            // A member access shows the member, not the aggregate before the dot.
            let accessed = text[span.end as usize..].starts_with('.')
                && spans
                    .get(i + 1..)
                    .and_then(|rest| rest.iter().find(|next| next.start > span.end))
                    .is_some_and(|next| next.class == TokenClass::Property);
            if accessed {
                continue;
            }
            let (start, name) = values::dotted_name(text, spans, i);
            // Brackets after a declaration are dimensions, not an element.
            let index = if span.modifiers.contains(Modifiers::DECLARATION) {
                None
            } else {
                values::index_after(text, span.end)
            };
            let element = match index {
                Some((Index::Constant(n), _)) => Some(Some(n)),
                Some((Index::Identifier(a, b), _)) => Some(self.constant_at(line, text, a, b)),
                Some((Index::Dynamic, _)) => Some(None),
                None => None,
            };
            let token = &text[span.start as usize..span.end as usize];
            let symbols = self.symbols_of(span, token);
            let (resolved, indexed) = if symbols.is_empty() {
                (self.member_value(text, spans, i), false)
            } else if span.class == TokenClass::Parameter {
                (self.parameter_value(&symbols), false)
            } else {
                self.signal_value(&symbols, element)
            };
            let end = match (indexed, index) {
                (true, Some((_, end))) => end,
                _ => span.end,
            };
            let display = if indexed {
                text[start as usize..end as usize].to_owned()
            } else {
                name
            };
            match resolved {
                Resolved::Value(value, state) => {
                    if names.insert(display.clone()) {
                        out.shown.push(Shown {
                            start,
                            end,
                            name: display,
                            text: value,
                            state,
                        });
                    }
                }
                Resolved::Pending => out.complete = false,
                Resolved::None => {}
            }
        }
        out
    }

    /// Elaborated value of a parameter identifier at bytes `a..b` of a line.
    fn constant_at(&self, line: u32, text: &str, a: u32, b: u32) -> Option<u64> {
        let span = self
            .spans(line)
            .iter()
            .find(|span| span.start == a && span.end == b && span.class == TokenClass::Parameter)?;
        let symbols = self.symbols_of(span, text.get(a as usize..b as usize)?);
        let text = self.constant_of(&symbols)?;
        u64::from_str_radix(&text, 16)
            .ok()
            .or_else(|| text.parse().ok())
    }

    /// The elaborated constant shared by every symbol, when they agree.
    fn constant_of(&self, symbols: &[String]) -> Option<String> {
        let index = self.index?;
        let mut constants: Vec<String> = symbols
            .iter()
            .filter_map(|path| index.database.symbols.get(path))
            .filter_map(|symbol| symbol.value.as_deref())
            .map(values::constant_text)
            .collect();
        constants.dedup();
        match constants.as_slice() {
            [one] => Some(one.clone()),
            _ => None,
        }
    }

    fn parameter_value(&self, symbols: &[String]) -> Resolved {
        match self.constant_of(symbols) {
            Some(text) => Resolved::Value(text, ValueState::Normal),
            None if symbols.len() == 1 => self.signal_value(symbols, None).0,
            None => Resolved::None,
        }
    }

    /// Value of the first symbol, or of one element of it. The flag tells whether an
    /// index in the source selected among recorded elements.
    fn signal_value(&self, symbols: &[String], element: Option<Option<u64>>) -> (Resolved, bool) {
        let Some(index) = self.index else {
            return (Resolved::None, false);
        };
        let symbol = &symbols[0];
        let mut recorded = index.recorded_paths(symbol);
        if recorded.is_empty() {
            // Unpacked arrays have no signal of their own; an element may.
            if let Some(Some(n)) = element {
                recorded = index.recorded_paths(&format!("{symbol}[{n}]"));
                if let [path] = recorded.as_slice() {
                    return match self.sample(path) {
                        Ok(Some(sample)) => (self.sampled(index, symbol, sample), true),
                        Ok(None) => (Resolved::Pending, true),
                        Err(resolved) => (resolved, true),
                    };
                }
            }
            return (Resolved::Value("–".to_owned(), ValueState::Missing), false);
        }
        let path = if recorded.len() > 1 {
            match element {
                Some(Some(n)) => {
                    let suffix = format!("[{n}]");
                    match recorded.iter().find(|path| path.ends_with(&suffix)) {
                        Some(path) => path,
                        None => {
                            return (Resolved::Value("–".to_owned(), ValueState::Missing), true);
                        }
                    }
                }
                _ => {
                    return (
                        Resolved::Value(format!("{} elements", recorded.len()), ValueState::Normal),
                        element.is_some(),
                    );
                }
            }
        } else {
            &recorded[0]
        };
        let indexed = recorded.len() > 1;
        match self.sample(path) {
            Ok(Some(sample)) => (self.sampled(index, symbol, sample), indexed),
            Ok(None) => (Resolved::Pending, indexed),
            Err(resolved) => (resolved, indexed),
        }
    }

    /// Text and state of a sample. Structs show their fields as a brace list, from
    /// the translator when it knows them and from the elaborated type otherwise.
    fn sampled(&self, index: &SourceIndex, symbol: &str, sample: Sample) -> Resolved {
        if sample.unknown {
            return Resolved::Value(sample.text, ValueState::Unknown);
        }
        let state = if sample.changed {
            ValueState::Changed
        } else {
            ValueState::Normal
        };
        if !sample.fields.is_empty() {
            let list: Vec<&str> = sample.fields.iter().map(|(_, v)| v.as_str()).collect();
            return Resolved::Value(format!("{{{}}}", list.join(", ")), state);
        }
        if let Some(fields) = index
            .database
            .symbols
            .get(symbol)
            .and_then(|s| values::struct_fields(&s.ty.text))
        {
            let mut hi = fields.iter().map(|f| f.width).sum::<u32>();
            if hi == sample.width {
                let mut parts = Vec::new();
                for field in &fields {
                    let lo = hi - field.width;
                    parts.push(values::slice_text(&sample.raw, sample.width, hi - 1, lo).0);
                    hi = lo;
                }
                return Resolved::Value(format!("{{{}}}", parts.join(", ")), state);
            }
        }
        Resolved::Value(sample.text, state)
    }

    /// Value of a struct member token: the preceding variable's sample sliced to the
    /// field the elaborated type places under that name.
    fn member_value(&self, text: &str, spans: &[Span], at: usize) -> Resolved {
        let span = &spans[at];
        let Some(index) = self.index else {
            return Resolved::None;
        };
        if span.class != TokenClass::Property || !text[..span.start as usize].ends_with('.') {
            return Resolved::None;
        }
        // The dot is an operator token of its own.
        let Some(previous) = spans[..at]
            .iter()
            .rev()
            .find(|previous| previous.end + 1 == span.start && previous.class.is_symbol())
        else {
            return Resolved::None;
        };
        let symbols = self.symbols_of(
            previous,
            &text[previous.start as usize..previous.end as usize],
        );
        let Some(symbol) = symbols.first() else {
            return Resolved::None;
        };
        let field = &text[span.start as usize..span.end as usize];
        let recorded = index.recorded_paths(symbol);
        let [path] = recorded.as_slice() else {
            return Resolved::None;
        };
        let sample = match self.sample(path) {
            Ok(Some(sample)) => sample,
            Ok(None) => return Resolved::Pending,
            Err(resolved) => return resolved,
        };
        let state = if sample.changed {
            ValueState::Changed
        } else {
            ValueState::Normal
        };
        if let Some((_, value)) = sample.fields.iter().find(|(name, _)| name == field) {
            return Resolved::Value(value.clone(), state);
        }
        let Some(fields) = index
            .database
            .symbols
            .get(symbol)
            .and_then(|s| values::struct_fields(&s.ty.text))
        else {
            return Resolved::None;
        };
        let mut hi = fields.iter().map(|f| f.width).sum::<u32>();
        if hi != sample.width {
            return Resolved::None;
        }
        for candidate in &fields {
            let lo = hi - candidate.width;
            if candidate.name == field {
                let (text, unknown) = values::slice_text(&sample.raw, sample.width, hi - 1, lo);
                let state = if unknown { ValueState::Unknown } else { state };
                return Resolved::Value(text, state);
            }
            hi = lo;
        }
        Resolved::None
    }

    /// Samples a recorded signal at the cursor. `Ok(None)` while it is still loading;
    /// `Err` carries what to show instead.
    fn sample(&self, path: &str) -> Result<Option<Sample>, Resolved> {
        use crate::translation::TranslationResultExt;
        use surfer_translation_types::ValueKind;
        let (Some(waves), Some(cursor)) = (
            self.waves().and_then(|w| w.inner.as_waves()),
            self.cursor.as_ref(),
        ) else {
            return Err(Resolved::None);
        };
        let variable = VariableRef::from_hierarchy_string(path);
        let Ok(meta) = waves.variable_meta(&variable) else {
            return Err(Resolved::Value("–".to_owned(), ValueState::Missing));
        };
        let query = match waves.query_variable(&variable, cursor) {
            Ok(Some(query)) => query,
            Ok(None) => return Ok(None),
            Err(_) => return Err(Resolved::Value("–".to_owned(), ValueState::Missing)),
        };
        let Some((time, raw)) = query.current else {
            // Nothing recorded yet: the signal holds its undefined initial value.
            return Err(Resolved::Value("x".to_owned(), ValueState::Unknown));
        };
        let translators = &self.state.translators;
        let format = self.displayed_format(&variable);
        let translator =
            crate::wave_data::variable_translator(format.as_ref(), &[], translators, || {
                Ok(meta.clone())
            });
        let flat = translator
            .translate(&meta, &raw)
            .ok()
            .map(|result| result.format_flat(&None, &[], translators))
            .unwrap_or_default();
        let root = flat
            .iter()
            .find(|field| field.names.is_empty())
            .and_then(|field| field.value.as_ref());
        let unknown =
            root.is_some_and(|value| matches!(value.kind, ValueKind::Undef | ValueKind::HighImp));
        let text = root
            .map(|value| value.value.clone())
            .unwrap_or_else(|| format!("{raw}"));
        let fields = flat
            .iter()
            .filter(|field| field.names.len() == 1)
            .filter_map(|field| {
                field
                    .value
                    .as_ref()
                    .map(|value| (field.names[0].clone(), value.value.clone()))
            })
            .collect();
        Ok(Some(Sample {
            width: meta.num_bits.unwrap_or(0),
            raw,
            text,
            unknown,
            changed: time == *cursor,
            fields,
        }))
    }

    /// The translator chosen for the signal in a waveform tile, when it is displayed.
    fn displayed_format(&self, variable: &VariableRef) -> Option<String> {
        let path = variable.full_path_string();
        self.state
            .user
            .workspace
            .item_lists()
            .values()
            .flat_map(|list| list.displayed_items.values())
            .find_map(|item| match item {
                crate::displayed_item::DisplayedItem::Variable(shown)
                    if shown.variable_ref.full_path_string() == path =>
                {
                    Some(shown.format.clone())
                }
                _ => None,
            })
            .flatten()
    }

    /// Value of an elaborated symbol at the cursor for the hover tooltip: one value,
    /// or `name=value` per recorded element, or the elaborated constant.
    fn value_text(&self, design_path: &str) -> String {
        let Some(index) = self.index else {
            return "no design attached".to_owned();
        };
        let recorded = index.recorded_paths(design_path);
        let constant = index
            .database
            .symbols
            .get(design_path)
            .and_then(|symbol| symbol.value.clone());
        if self.cursor.is_none() {
            return constant.unwrap_or_else(|| "set the cursor to see values".to_owned());
        }
        let mut texts = Vec::new();
        for path in recorded.iter().take(4) {
            let text = match self.sample(path) {
                Ok(Some(sample)) => sample.text,
                Ok(None) => "loading".to_owned(),
                Err(Resolved::Value(text, _)) => text,
                Err(_) => continue,
            };
            texts.push(match recorded.len() {
                1 => text,
                _ => format!(
                    "{}={text}",
                    path.rsplit_once('.')
                        .map_or(path.as_str(), |(_, tail)| tail)
                ),
            });
        }
        if texts.is_empty() {
            return constant.unwrap_or_else(|| "not recorded".to_owned());
        }
        if recorded.len() > 4 {
            texts.push("…".to_owned());
        }
        texts.join(", ")
    }

    // --- clicks and hover ---------------------------------------------------

    /// Performs a modified click. Errors are notices for the tile header.
    fn activate(
        &self,
        span: &Span,
        token: &str,
        intent: Intent,
        commands: &mut Vec<Message>,
    ) -> Result<(), String> {
        let index = self.index.ok_or("no design database attached")?;
        match intent {
            Intent::AddToWaveform => {
                let symbols = self.symbols_of(span, token);
                let signals: Vec<String> = symbols
                    .iter()
                    .flat_map(|path| index.recorded_paths(path))
                    .collect();
                if signals.is_empty() {
                    return Err(if symbols.is_empty() {
                        "no design symbol at this token".to_owned()
                    } else {
                        format!("{} is not recorded in the trace", symbols.join(", "))
                    });
                }
                commands.push(Message::AddVariables(
                    signals
                        .iter()
                        .map(|path| VariableRef::from_hierarchy_string(path))
                        .collect(),
                ));
                Ok(())
            }
            Intent::Navigate => {
                let open =
                    |location: SourceLocation, instance: Option<String>| Message::OpenSource {
                        file: location.file,
                        line: location.line,
                        column: location.column,
                        instance,
                    };
                let instances = self.instances_of(span, token);
                if let (TokenClass::Instance, Some(instance)) = (span.class, instances.first()) {
                    // An instance name opens its module, viewed in that instance.
                    let module = index
                        .instance_definition(instance)
                        .ok_or("instance is not part of the elaborated design")?;
                    let location = index
                        .definition(module)
                        .ok_or_else(|| format!("module {module} is not indexed"))?;
                    commands.push(open(location, Some(instance.clone())));
                    return Ok(());
                }
                if matches!(
                    span.class,
                    TokenClass::Module | TokenClass::Interface | TokenClass::Package
                ) {
                    // A module or interface name has no owner; view it in its first instance.
                    let location = index
                        .definition(token)
                        .ok_or_else(|| format!("{token} is not declared in the indexed sources"))?;
                    let instance = index.instances_of_module(token).into_iter().next();
                    commands.push(open(location, instance));
                    return Ok(());
                }
                let declaration = span
                    .declaration
                    .and_then(|at| index.location_of(at))
                    .ok_or("no declaration found for this token")?;
                let owner = self
                    .symbols_of(span, token)
                    .iter()
                    .find_map(|path| index.owner_of(path))
                    .map(|instance| instance.path.clone());
                commands.push(open(declaration, owner));
                Ok(())
            }
        }
    }

    fn hover_ui(&self, ui: &mut Ui, token: &str, span: &Span) {
        ui.set_max_width(ui.spacing().tooltip_width.max(320.0));
        let mut kind = span.class.label().to_owned();
        for (modifier, name) in [
            (Modifiers::INPUT, "input"),
            (Modifiers::OUTPUT, "output"),
            (Modifiers::INOUT, "inout"),
            (Modifiers::CLOCK, "clock"),
            (Modifiers::ARGUMENT, "argument"),
        ] {
            if span.modifiers.contains(modifier) {
                kind = format!("{name} {kind}");
            }
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(token).strong().monospace());
            ui.label(RichText::new(kind).weak());
        });
        let Some(index) = self.index else {
            ui.label(RichText::new("no design database attached").weak());
            return;
        };
        let symbols = self.symbols_of(span, token);
        let instances = self.instances_of(span, token);
        if let Some(first) = symbols
            .first()
            .and_then(|path| index.database.symbols.get(path))
        {
            ui.label(
                RichText::new(format!("{} in {}", first.ty.text, first.owner))
                    .monospace()
                    .small(),
            );
        }
        if symbols.is_empty() {
            if let Some(instance) = instances.first() {
                let module = index.instance_definition(instance).unwrap_or("?");
                ui.label(
                    RichText::new(format!("{module} {}", instances.join(", ")))
                        .monospace()
                        .small(),
                );
            } else if matches!(span.class, TokenClass::Module | TokenClass::Interface) {
                let count = index.instances_of_module(token).len();
                ui.label(RichText::new(format!("{count} instances")).weak());
            } else {
                ui.label(RichText::new("not part of the elaborated design").weak());
            }
            return;
        }
        egui::Grid::new("source_hover_values")
            .num_columns(2)
            .spacing([12.0, 2.0])
            .show(ui, |ui| {
                for path in symbols.iter().take(8) {
                    ui.label(RichText::new(path.as_str()).monospace());
                    ui.label(RichText::new(self.value_text(path)).monospace());
                    ui.end_row();
                }
                if symbols.len() > 8 {
                    ui.label(RichText::new(format!("… {} more", symbols.len() - 8)).weak());
                    ui.end_row();
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_location_and_instance_are_persisted_with_the_tile() {
        let mut tile = SourceCodeTile::default();
        tile.open(
            SourceLocation {
                file: Utf8PathBuf::from("rtl/top.sv"),
                line: 17,
                column: 4,
            },
            Some("top.u0".into()),
        );
        tile.update(SourceCodeMessage::ValuesLayout(ValuesLayout::Inline));
        let encoded = ron::to_string(&tile).unwrap();
        let restored: SourceCodeTile = ron::from_str(&encoded).unwrap();
        assert_eq!(restored.file, tile.file);
        assert_eq!((restored.line, restored.column), (17, 4));
        assert_eq!(restored.instance.as_deref(), Some("top.u0"));
        assert_eq!(restored.values_layout, Some(ValuesLayout::Inline));
        // Payloads written before the instance and layout fields existed still load.
        let legacy: SourceCodeTile =
            ron::from_str(r#"(file:Some("a.sv"),line:1,column:1)"#).unwrap();
        assert_eq!(legacy.instance, None);
        assert_eq!(legacy.values_layout, None);
    }

    #[test]
    fn reopening_replaces_the_instance() {
        let mut tile = SourceCodeTile::default();
        tile.open(
            SourceLocation {
                file: "a.sv".into(),
                line: 1,
                column: 1,
            },
            Some("top.u1".into()),
        );
        tile.open(
            SourceLocation {
                file: "a.sv".into(),
                line: 5,
                column: 2,
            },
            None,
        );
        assert_eq!(tile.instance, None);
        tile.set_instance(Some("top.u0".into()));
        assert_eq!(tile.instance.as_deref(), Some("top.u0"));
    }

    #[test]
    fn layout_updates_report_whether_anything_changed() {
        let mut tile = SourceCodeTile::default();
        assert!(tile.update(SourceCodeMessage::ValuesLayout(ValuesLayout::Inline)));
        assert!(!tile.update(SourceCodeMessage::ValuesLayout(ValuesLayout::Inline)));
        assert!(tile.update(SourceCodeMessage::ValuesLayout(ValuesLayout::Trailing)));
    }

    #[test]
    fn laid_lines_map_characters_back_to_source_bytes() {
        let mut laid = LaidLine {
            job: LayoutJob::default(),
            bytes: Vec::new(),
            chips: Vec::new(),
        };
        let font = FontId::monospace(FONT_SIZE);
        laid.append("   1  ", &font, Color32::WHITE, None);
        laid.append("aé", &font, Color32::WHITE, Some(0));
        laid.append_value(" 1 ", Color32::WHITE, Color32::TRANSPARENT, false);
        laid.append("b", &font, Color32::WHITE, Some(3));
        assert_eq!(laid.bytes[6], Some(0));
        assert_eq!(laid.bytes[7], Some(1));
        assert_eq!(laid.bytes[8], None);
        assert_eq!(laid.char_of(3), 11);
        assert_eq!(laid.char_of(1), 7);
    }
}
