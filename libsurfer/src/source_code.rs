//! Source-code tile.
//!
//! The tile shows one file in the context of one elaborated design instance. Token
//! classes, declarations and the generate blocks an instance leaves uninstantiated
//! come from the static source index of the VDB companion ([`crate::source_index`]);
//! which recorded signal a token denotes, and its value at the cursor, come from the
//! same attachment and the loaded recording. Nothing is computed outside this
//! process: hover a symbol for its value, ctrl-click to navigate, alt-click to add it
//! to the waveform.

use camino::Utf8PathBuf;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, RichText, Sense, Stroke, TextWrapMode, Ui};
use serde::{Deserialize, Serialize};
use std::{
    cell::{Cell, RefCell},
    sync::Arc,
};

use crate::message::Message;
use crate::source_index::{FileTokens, Modifiers, SourceIndex, SourceLocation, Span, TokenClass};
use crate::system_state::SystemState;

const FONT_SIZE: f32 = 13.0;
/// Characters of line number and padding before the source text of each line.
pub(crate) const GUTTER: usize = 7;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceCodeTile {
    pub file: Option<Utf8PathBuf>,
    pub line: u32,
    pub column: u32,
    /// Elaborated design instance the file is viewed in, when known.
    #[serde(default)]
    pub instance: Option<String>,
    #[serde(skip)]
    document: RefCell<Option<SourceDocument>>,
    #[serde(skip)]
    last_target: Cell<Option<(u32, u32)>>,
    #[serde(skip)]
    notices: RefCell<Vec<String>>,
}

#[derive(Clone)]
struct SourceDocument {
    file: Utf8PathBuf,
    text: Result<Arc<str>, String>,
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

    pub(crate) fn ui(&self, ui: &mut Ui, state: &SystemState, commands: &mut Vec<Message>) {
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
        self.header(ui, state, file, &view, commands);
        ui.separator();
        ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);
        let theme = &state.user.config.theme;
        let target = self.line.saturating_sub(1) as usize;
        let font = FontId::monospace(FONT_SIZE);
        let modifiers = ui.input(|i| i.modifiers);
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (index, text) in contents.lines().enumerate() {
                    let line = index as u32;
                    let spans = view.spans(line);
                    let line_inactive = view.line_inactive(line, text.len() as u32);
                    let mut job = LayoutJob::default();
                    job.append(
                        &format!("{:>5}  ", index + 1),
                        0.0,
                        TextFormat {
                            font_id: font.clone(),
                            color: ui.visuals().weak_text_color(),
                            ..Default::default()
                        },
                    );
                    let normal = if line_inactive {
                        theme.source.inactive
                    } else {
                        ui.visuals().text_color()
                    };
                    let mut at = 0usize;
                    for span in spans {
                        let start = (span.start as usize).min(text.len());
                        let end = (span.end as usize).min(text.len());
                        if start < at || end <= start {
                            continue;
                        }
                        if at < start {
                            append(&mut job, &text[at..start], &font, normal);
                        }
                        let color = if line_inactive || view.inactive(line, span.start, span.end)
                        {
                            theme.source.inactive
                        } else {
                            span_color(&theme.source, span, normal)
                        };
                        append(&mut job, &text[start..end], &font, color);
                        at = end;
                    }
                    if at < text.len() {
                        append(&mut job, &text[at..], &font, normal);
                    }
                    if text.is_empty() {
                        append(&mut job, " ", &font, normal);
                    }
                    let galley = ui.painter().layout_job(job);
                    let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::click());
                    if index == target {
                        ui.painter()
                            .rect_filled(rect, 0.0, theme.source.target_line);
                    } else if line_inactive {
                        ui.painter()
                            .rect_filled(rect, 0.0, theme.source.inactive_background);
                    }
                    ui.painter().galley(rect.min, galley.clone(), normal);
                    if index == target && scroll_to_target {
                        // Navigation jumps, so no scroll animation.
                        response.scroll_to_me_animation(
                            Some(egui::Align::Center),
                            egui::style::ScrollAnimation::none(),
                        );
                    }
                    // Hover and clicks on symbol tokens.
                    let Some(pointer) = response.hover_pos() else {
                        continue;
                    };
                    let char_index = galley.cursor_from_pos(pointer - rect.min).index.0;
                    let Some(byte) = char_to_byte(text, char_index.saturating_sub(GUTTER)) else {
                        continue;
                    };
                    if char_index < GUTTER {
                        continue;
                    }
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
                        let from = galley.pos_from_cursor(egui::text::CCursor::new(
                            GUTTER + byte_to_char(text, span.start as usize),
                        ));
                        let to = galley.pos_from_cursor(egui::text::CCursor::new(
                            GUTTER + byte_to_char(text, span.end as usize),
                        ));
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

    fn header(
        &self,
        ui: &mut Ui,
        state: &SystemState,
        file: &Utf8PathBuf,
        view: &View,
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

fn append(job: &mut LayoutJob, text: &str, font: &FontId, color: Color32) {
    job.append(
        text,
        0.0,
        TextFormat {
            font_id: font.clone(),
            color,
            ..Default::default()
        },
    );
}

fn char_to_byte(text: &str, char_index: usize) -> Option<usize> {
    text.char_indices().nth(char_index).map(|(byte, _)| byte)
}

fn byte_to_char(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].chars().count()
}

fn span_color(colors: &crate::config::SourceColors, span: &Span, normal: Color32) -> Color32 {
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

/// Everything the draw loop needs from the design for one file and one frame.
struct View<'a> {
    index: Option<&'a SourceIndex>,
    tokens: Option<Arc<FileTokens>>,
    inactive: Vec<vtr_vdb::InactiveRange>,
    state: &'a SystemState,
    instance: Option<String>,
}

impl<'a> View<'a> {
    fn new(tile: &SourceCodeTile, state: &'a SystemState, file: &Utf8PathBuf) -> Self {
        let index = state
            .user
            .waves
            .as_ref()
            .and_then(|w| w.inner.as_waves())
            .and_then(|w| w.source_index());
        let tokens = index.and_then(|index| index.file_tokens(file));
        let inactive = index
            .zip(tile.instance.as_deref())
            .map(|(index, instance)| index.inactive_ranges(file, instance))
            .unwrap_or_default();
        Self {
            index,
            tokens,
            inactive,
            state,
            instance: tile.instance.clone(),
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

    /// Elaborated symbol paths declared where `span` points, those under the viewed
    /// instance first when any are.
    fn symbols_of(&self, span: &Span) -> Vec<String> {
        self.in_context(self.declared(span).map_or(&[], |d| d.symbols.as_slice()))
    }

    fn instances_of(&self, span: &Span) -> Vec<String> {
        self.in_context(self.declared(span).map_or(&[], |d| d.instances.as_slice()))
    }

    fn declared(&self, span: &Span) -> Option<&crate::source_index::Declared> {
        self.index?.declared(span.declaration?)
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
                let symbols = self.symbols_of(span);
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
                        .map(|path| {
                            crate::wave_container::VariableRefExt::from_hierarchy_string(path)
                        })
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
                let instances = self.instances_of(span);
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
                    .symbols_of(span)
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
        let symbols = self.symbols_of(span);
        let instances = self.instances_of(span);
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
        let cursor = self
            .state
            .user
            .waves
            .as_ref()
            .and_then(|waves| waves.cursor.as_ref())
            .and_then(num::BigInt::to_biguint);
        egui::Grid::new("source_hover_values")
            .num_columns(2)
            .spacing([12.0, 2.0])
            .show(ui, |ui| {
                for path in symbols.iter().take(8) {
                    ui.label(RichText::new(path.as_str()).monospace());
                    ui.label(RichText::new(self.value_text(path, cursor.as_ref())).monospace());
                    ui.end_row();
                }
                if symbols.len() > 8 {
                    ui.label(RichText::new(format!("… {} more", symbols.len() - 8)).weak());
                    ui.end_row();
                }
            });
    }

    /// Value of an elaborated symbol at the cursor, formatted by the preferred translator,
    /// falling back to the elaborated constant for parameters.
    fn value_text(&self, design_path: &str, cursor: Option<&num::BigUint>) -> String {
        use crate::translation::TranslationResultExt;
        use crate::wave_container::{VariableRef, VariableRefExt};
        let Some(index) = self.index else {
            return "no design attached".to_owned();
        };
        let recorded = index.recorded_paths(design_path);
        let constant = index
            .database
            .symbols
            .get(design_path)
            .and_then(|symbol| symbol.value.clone());
        let Some(cursor) = cursor else {
            return constant.unwrap_or_else(|| "set the cursor to see values".to_owned());
        };
        let Some(waves) = self
            .state
            .user
            .waves
            .as_ref()
            .and_then(|w| w.inner.as_waves())
        else {
            return "no waveform".to_owned();
        };
        let mut values = Vec::new();
        for path in recorded.iter().take(4) {
            let variable = VariableRef::from_hierarchy_string(path);
            let Ok(meta) = waves.variable_meta(&variable) else {
                continue;
            };
            let Ok(Some(query)) = waves.query_variable(&variable, cursor) else {
                continue;
            };
            let Some((_, value)) = query.current else {
                values.push("undefined".to_owned());
                continue;
            };
            let translators = &self.state.translators;
            let name = crate::wave_data::select_preferred_translator(&meta, translators);
            let translator: &crate::translation::DynTranslator =
                translators.get_translator(&name) as _;
            let text = translator
                .translate(&meta, &value)
                .ok()
                .and_then(|result| {
                    result
                        .format_flat(&None, &[], translators)
                        .into_iter()
                        .find(|field| field.names.is_empty())
                        .and_then(|field| field.value.map(|v| v.value))
                })
                .unwrap_or_else(|| format!("{value}"));
            values.push(match recorded.len() {
                1 => text,
                _ => format!(
                    "{}={text}",
                    path.rsplit_once('.')
                        .map_or(path.as_str(), |(_, tail)| tail)
                ),
            });
        }
        if values.is_empty() {
            return constant.unwrap_or_else(|| "not recorded".to_owned());
        }
        if recorded.len() > 4 {
            values.push("…".to_owned());
        }
        values.join(", ")
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
        let encoded = ron::to_string(&tile).unwrap();
        let restored: SourceCodeTile = ron::from_str(&encoded).unwrap();
        assert_eq!(restored.file, tile.file);
        assert_eq!((restored.line, restored.column), (17, 4));
        assert_eq!(restored.instance.as_deref(), Some("top.u0"));
        // Payloads written before the instance field existed still load.
        let legacy: SourceCodeTile =
            ron::from_str(r#"(file:Some("a.sv"),line:1,column:1)"#).unwrap();
        assert_eq!(legacy.instance, None);
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
    fn char_and_byte_offsets_round_trip_through_multibyte_text() {
        let text = "aé b";
        assert_eq!(char_to_byte(text, 2), Some(3));
        assert_eq!(byte_to_char(text, 3), 2);
        assert_eq!(char_to_byte(text, 9), None);
    }
}
