use crate::{
    Message,
    system_state::SystemState,
    translation::{TranslationResultExt, ValueKindExt},
    wave_container::{ScopeRef, ScopeRefExt},
};
use egui::collapsing_header::CollapsingState;
use egui::{Button, DragValue};
use egui_extras::{Column, TableBuilder};
use egui_remixicon::icons;
use fuzzy_matcher::{FuzzyMatcher, skim::SkimMatcherV2};
use regex::RegexBuilder;
use std::rc::Rc;
use surfer_translation_types::{TranslationPreference, Translator, ValueKind};
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MemoryViewerFormat {
    Decimal,
    Hexadecimal,
    Binary,
}
impl MemoryViewerFormat {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Decimal => "Decimal",
            Self::Hexadecimal => "Hexadecimal",
            Self::Binary => "Binary",
        }
    }
}
pub(crate) struct MemoryViewerState {
    pub(crate) open: bool,
    pub(crate) scope: Option<ScopeRef>,
    pub(crate) name: Option<String>,
    jump_to_index: String,
    search_value: String,
    highlight_value: String,
    search_match_mode: ValueMatchMode,
    highlight_match_mode: ValueMatchMode,
    highlight_case_insensitive: bool,
    search_case_insensitive: bool,
    filter_value: String,
    filter_match_mode: ValueMatchMode,
    filter_case_insensitive: bool,
    index_format: MemoryViewerFormat,
    value_format: String,
    scroll_to_row: Option<usize>,
    color_values: bool,
    filter_mode: ChangeModes,
    highlight_mode: ChangeModes,
    value_column_count: usize,
    selected_value_position: Option<usize>,
}
impl Default for MemoryViewerState {
    fn default() -> Self {
        Self {
            open: false,
            scope: None,
            name: None,
            jump_to_index: String::new(),
            search_value: String::new(),
            search_match_mode: ValueMatchMode::Contains,
            search_case_insensitive: true,
            highlight_value: String::new(),
            highlight_match_mode: ValueMatchMode::Contains,
            highlight_case_insensitive: true,
            filter_value: String::new(),
            filter_match_mode: ValueMatchMode::Contains,
            filter_case_insensitive: true,
            index_format: MemoryViewerFormat::Decimal,
            value_format: "Hexadecimal".to_string(),
            scroll_to_row: None,
            color_values: false,
            selected_value_position: None,
            value_column_count: 1,
            filter_mode: ChangeModes::AllValues,
            highlight_mode: ChangeModes::AllValues,
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ChangeModes {
    AllValues,
    ChangedAtCursor,
    ChangedBtwCursorAndMarker(u8),
}
#[derive(Clone)]
pub(crate) struct MemoryRow {
    index: i64,
    value: String,
    kind: ValueKind,
    changed_at_cursor: bool,
    changed_for_filter: bool,
    changed_for_highlight: bool,
}
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct MemoryViewerCacheKey {
    pub scope: crate::wave_container::ScopeRef,
    pub cursor: num::BigUint,
    pub value_format: String,
    pub filter_mode: ChangeModes,
    pub highlight_mode: ChangeModes,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueMatchMode {
    Contains,
    StartsWith,
    Regex,
    Fuzzy,
}

impl ValueMatchMode {
    fn label(self) -> &'static str {
        match self {
            Self::Contains => "Value contains",
            Self::StartsWith => "Value starts with",
            Self::Regex => "Regular expression",
            Self::Fuzzy => "Fuzzy",
        }
    }
}
#[derive(Clone)]
pub(crate) struct MemoryViewerCache {
    pub key: MemoryViewerCacheKey,
    pub rows: std::rc::Rc<Vec<MemoryRow>>,
}
fn format_index(index: i64, format: MemoryViewerFormat, max_index: i64) -> String {
    match format {
        MemoryViewerFormat::Decimal => {
            let width = max_index.to_string().len();
            format!("{index:0width$}")
        }
        MemoryViewerFormat::Hexadecimal => {
            let width = format!("{max_index:x}").len();
            format!("{index:0width$x}")
        }
        MemoryViewerFormat::Binary => {
            let width = format!("{max_index:b}").len();
            format!("{index:0width$b}")
        }
    }
}

fn parse_index_from_name(name: &str) -> Option<i64> {
    name.trim()
        .trim_start_matches('[')
        .split(']')
        .next()
        .and_then(|index| index.parse::<i64>().ok())
}

fn parse_jump_to_index(input: &str) -> Option<i64> {
    let input = input.trim();

    if let Some(hex) = input.strip_prefix("0x") {
        i64::from_str_radix(hex, 16).ok()
    } else if let Some(binary) = input.strip_prefix("0b") {
        i64::from_str_radix(binary, 2).ok()
    } else {
        input.parse::<i64>().ok()
    }
}

fn closest_row_index(rows: &[&MemoryRow], target_index: i64) -> Option<usize> {
    rows.iter()
        .enumerate()
        .min_by_key(|(_position, row)| (row.index - target_index).abs())
        .map(|(row_index, _)| row_index)
}

type ValueMatcher = Box<dyn Fn(&str) -> bool>;

fn build_value_matcher(
    pattern: &str,
    mode: ValueMatchMode,
    case_insensitive: bool,
) -> Option<ValueMatcher> {
    let pattern = pattern.trim();

    if pattern.is_empty() {
        return None;
    }

    match mode {
        ValueMatchMode::Contains => {
            if case_insensitive {
                let pattern = pattern.to_lowercase();

                Some(Box::new(move |value| {
                    value.to_lowercase().contains(&pattern)
                }))
            } else {
                let pattern = pattern.to_string();

                Some(Box::new(move |value| value.contains(&pattern)))
            }
        }

        ValueMatchMode::StartsWith => {
            if case_insensitive {
                let pattern = pattern.to_lowercase();

                Some(Box::new(move |value| {
                    value.to_lowercase().starts_with(&pattern)
                }))
            } else {
                let pattern = pattern.to_string();

                Some(Box::new(move |value| value.starts_with(&pattern)))
            }
        }

        ValueMatchMode::Regex => {
            let regex = RegexBuilder::new(pattern)
                .case_insensitive(case_insensitive)
                .build()
                .ok()?;

            Some(Box::new(move |value| regex.is_match(value)))
        }

        ValueMatchMode::Fuzzy => {
            let pattern = pattern.to_string();

            let matcher = if case_insensitive {
                SkimMatcherV2::default().ignore_case()
            } else {
                SkimMatcherV2::default().respect_case()
            };

            Some(Box::new(move |value| {
                matcher.fuzzy_match(value, &pattern).is_some()
            }))
        }
    }
}
fn value_match_menu(ui: &mut egui::Ui, mode: &mut ValueMatchMode, case_insensitive: &mut bool) {
    ui.checkbox(case_insensitive, "Case insensitive");

    ui.separator();

    for candidate in [
        ValueMatchMode::Fuzzy,
        ValueMatchMode::Regex,
        ValueMatchMode::StartsWith,
        ValueMatchMode::Contains,
    ] {
        ui.radio_value(mode, candidate, candidate.label());
    }
}
fn matching_text_edit(
    ui: &mut egui::Ui,
    value: &mut String,
    hint: &str,
    width: f32,
    mode: &mut ValueMatchMode,
    case_insensitive: &mut bool,
) {
    let text_response = ui
        .add_sized(
            [width, 20.0],
            egui::TextEdit::singleline(value).hint_text(hint),
        )
        .on_hover_text("Matching options");

    text_response.context_menu(|ui| {
        value_match_menu(ui, mode, case_insensitive);
    });
    ui.menu_button(egui::RichText::new(icons::FILTER_FILL).size(12.0), |ui| {
        value_match_menu(ui, mode, case_insensitive);
    });

    if ui
        .add_enabled(
            !value.is_empty(),
            Button::new(icons::CLOSE_FILL).frame(false),
        )
        .on_hover_text("Clear")
        .clicked()
    {
        value.clear();
    }
}
fn change_range(
    mode: ChangeModes,
    cursor: &num::BigUint,
    markers: &std::collections::HashMap<u8, num::BigInt>,
) -> Option<(num::BigUint, num::BigUint)> {
    let ChangeModes::ChangedBtwCursorAndMarker(marker_index) = mode else {
        return None;
    };

    let marker_time = markers.get(&marker_index)?.to_biguint()?;

    if cursor <= &marker_time {
        Some((cursor.clone(), marker_time))
    } else {
        Some((marker_time, cursor.clone()))
    }
}
fn marker_combo(ui: &mut egui::Ui, id: &'static str, selected: &mut ChangeModes) {
    let selected_marker = match selected {
        ChangeModes::ChangedBtwCursorAndMarker(index) => *index,
        _ => 1,
    };

    egui::ComboBox::from_id_salt(id)
        .selected_text(format!("Marker {selected_marker}"))
        .show_ui(ui, |ui| {
            for marker_index in 0..=254 {
                ui.selectable_value(
                    selected,
                    ChangeModes::ChangedBtwCursorAndMarker(marker_index),
                    format!("Marker {marker_index}"),
                );
            }
        });
}
fn change_mode_menu(ui: &mut egui::Ui, marker_id: &'static str, selected: &mut ChangeModes) {
    ui.radio_value(selected, ChangeModes::AllValues, "None");

    ui.radio_value(selected, ChangeModes::ChangedAtCursor, "Cursor");

    let marker_index = match *selected {
        ChangeModes::ChangedBtwCursorAndMarker(index) => index,
        _ => 1,
    };

    ui.radio_value(
        selected,
        ChangeModes::ChangedBtwCursorAndMarker(marker_index),
        "Cursor and marker",
    );

    if matches!(selected, ChangeModes::ChangedBtwCursorAndMarker(_)) {
        marker_combo(ui, marker_id, selected);
    }
}
fn left_aligned_header(ui: &mut egui::Ui, text: &str) {
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
        ui.label(text);
    });
}
fn highlight_dropdown(
    ui: &mut egui::Ui,
    selected: &mut ChangeModes,
    highlight_value: &mut String,
    match_mode: &mut ValueMatchMode,
    case_insensitive: &mut bool,
) {
    ui.vertical(|ui| {
        CollapsingState::load_with_default_open(
            ui.ctx(),
            ui.make_persistent_id("memory_viewer_highlight"),
            false,
        )
        .show_header(ui, |ui| {
            left_aligned_header(ui, "Highlight");
        })
        .body(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(260.0, 0.0),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    change_mode_menu(ui, "memory_viewer_highlight_marker", selected);

                    ui.horizontal(|ui| {
                        ui.label("Value:");

                        matching_text_edit(
                            ui,
                            highlight_value,
                            "Highlight",
                            120.0,
                            match_mode,
                            case_insensitive,
                        );
                    });
                },
            );
        });
    });
}
fn filter_dropdown(
    ui: &mut egui::Ui,
    selected: &mut ChangeModes,
    filter_value: &mut String,
    match_mode: &mut ValueMatchMode,
    case_insensitive: &mut bool,
) {
    ui.vertical(|ui| {
        CollapsingState::load_with_default_open(
            ui.ctx(),
            ui.make_persistent_id("memory_viewer_filter"),
            false,
        )
        .show_header(ui, |ui| {
            left_aligned_header(ui, "Filter");
        })
        .body(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(260.0, 0.0),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    change_mode_menu(ui, "memory_viewer_filter_marker", selected);

                    ui.horizontal(|ui| {
                        ui.label("Value:");

                        matching_text_edit(
                            ui,
                            filter_value,
                            "Filter",
                            120.0,
                            match_mode,
                            case_insensitive,
                        );
                    });
                },
            );
        });
    });
}
impl SystemState {
    pub fn draw_memory_viewer_window(&mut self, ctx: &egui::Context, _msgs: &mut Vec<Message>) {
        if !self.memory_viewer.open {
            return;
        }

        let mut open = self.memory_viewer.open;
        let translator_name = self.memory_viewer.value_format.clone();
        let translator = self.translators.get_translator(&translator_name);
        egui::Window::new("Memory Viewer")
            .open(&mut open)
            .resizable(true)
            .default_size([520.0, 500.0])
            .show(ctx, |ui| {
                ui.heading("Memory Viewer");

                let Some(scope) = self.memory_viewer.scope.clone() else {
                    ui.label("No variable selected");
                    return;
                };

                let display_name = self
                    .memory_viewer
                    .name
                    .clone()
                    .unwrap_or_else(|| scope.name());

                ui.label(format!("Variable: {display_name}"));

                let Some(waves) = &self.user.waves else {
                    ui.label("No waveform loaded");
                    return;
                };

                let Some(cursor) = waves.cursor.as_ref().and_then(num::BigInt::to_biguint) else {
                    ui.label("Place the cursor to inspect values.");
                    return;
                };

                ui.label(format!("Time: {cursor}"));

                let Some(wave_container) = waves.inner.as_waves() else {
                    ui.label("No wave container available");
                    return;
                };

                let mut rows = Vec::new();
                let cache_key = MemoryViewerCacheKey {
                    scope: scope.clone(),
                    cursor: cursor.clone(),
                    value_format: translator_name.clone(),
                    filter_mode: self.memory_viewer.filter_mode,
                    highlight_mode: self.memory_viewer.highlight_mode,
                };
                let cache_hit = self
                    .memory_viewer_cache
                    .as_ref()
                    .filter(|cache| cache.key == cache_key)
                    .map(|cache| Rc::clone(&cache.rows));

                if let Some(cached_rows) = cache_hit {
                    rows = cached_rows.as_ref().clone();
                }

                let mut variables = wave_container.variables_in_scope(&scope);
                variables.sort_by_key(|v| v.index.unwrap_or(i64::MAX));
                let filter_change_range =
                    change_range(self.memory_viewer.filter_mode, &cursor, &waves.markers);

                let highlight_change_range =
                    change_range(self.memory_viewer.highlight_mode, &cursor, &waves.markers);
                if rows.is_empty() {
                    for var_ref in &variables {
                        let Some(index) = var_ref
                            .index
                            .or_else(|| parse_index_from_name(&var_ref.name))
                        else {
                            continue;
                        };

                        let Some((change_time, raw_value)) = wave_container
                            .query_variable(var_ref, &cursor)
                            .ok()
                            .flatten()
                            .and_then(|query_result| query_result.current)
                        else {
                            continue;
                        };

                        let changed_at_cursor = change_time == cursor;
                        let changed_in_range =
                            |range: Option<&(num::BigUint, num::BigUint)>| -> bool {
                                range
                                    .and_then(|(start_time, end_time)| {
                                        wave_container
                                            .query_variable(var_ref, start_time)
                                            .ok()
                                            .flatten()
                                            .and_then(|query_result| query_result.next)
                                            .map(|next_change| {
                                                next_change > *start_time && next_change < *end_time
                                            })
                                    })
                                    .unwrap_or(false)
                            };

                        let changed_for_filter = changed_in_range(filter_change_range.as_ref());

                        let changed_for_highlight = if filter_change_range == highlight_change_range
                        {
                            changed_for_filter
                        } else {
                            changed_in_range(highlight_change_range.as_ref())
                        };
                        let display_value = wave_container
                            .variable_meta(var_ref)
                            .ok()
                            .and_then(|meta| {
                                translator
                                    .translate(&meta, &raw_value)
                                    .ok()
                                    .and_then(|result| {
                                        result
                                            .format_flat(
                                                &Some(translator_name.clone()),
                                                &[],
                                                &self.translators,
                                            )
                                            .into_iter()
                                            .next()
                                            .and_then(|formatted| {
                                                formatted
                                                    .value
                                                    .map(|value| (value.value, value.kind))
                                            })
                                    })
                            })
                            .unwrap_or_else(|| (raw_value.to_string(), ValueKind::Normal));

                        let (value, kind) = display_value;

                        rows.push(MemoryRow {
                            index,
                            value,
                            kind,
                            changed_at_cursor,
                            changed_for_filter,
                            changed_for_highlight,
                        });
                    }

                    self.memory_viewer_cache = Some(MemoryViewerCache {
                        key: cache_key,
                        rows: Rc::new(rows.clone()),
                    });
                }
                ui.label(format!("Structured entries: {}", rows.len()));

                ui.separator();

                ui.horizontal_top(|ui| {
                    if self.memory_viewer.value_column_count == 1 {
                        filter_dropdown(
                            ui,
                            &mut self.memory_viewer.filter_mode,
                            &mut self.memory_viewer.filter_value,
                            &mut self.memory_viewer.filter_match_mode,
                            &mut self.memory_viewer.filter_case_insensitive,
                        );
                    }
                    highlight_dropdown(
                        ui,
                        &mut self.memory_viewer.highlight_mode,
                        &mut self.memory_viewer.highlight_value,
                        &mut self.memory_viewer.highlight_match_mode,
                        &mut self.memory_viewer.highlight_case_insensitive,
                    );

                    ui.checkbox(&mut self.memory_viewer.color_values, "Color values");
                });
                let search_matcher = build_value_matcher(
                    &self.memory_viewer.search_value,
                    self.memory_viewer.search_match_mode,
                    self.memory_viewer.search_case_insensitive,
                );
                let highlight_matcher = build_value_matcher(
                    &self.memory_viewer.highlight_value,
                    self.memory_viewer.highlight_match_mode,
                    self.memory_viewer.highlight_case_insensitive,
                );
                let filter_matcher = build_value_matcher(
                    &self.memory_viewer.filter_value,
                    self.memory_viewer.filter_match_mode,
                    self.memory_viewer.filter_case_insensitive,
                );
                let visible_rows: Vec<_> = rows
                    .iter()
                    .filter(|row| {
                        let matches_change_filter = match self.memory_viewer.filter_mode {
                            ChangeModes::AllValues => true,
                            ChangeModes::ChangedAtCursor => row.changed_at_cursor,
                            ChangeModes::ChangedBtwCursorAndMarker(_) => row.changed_for_filter,
                        };

                        let matches_text_filter = filter_matcher
                            .as_ref()
                            .is_none_or(|matcher| matcher(&row.value));

                        matches_change_filter && matches_text_filter
                    })
                    .collect();
                let matching_positions: Vec<usize> = search_matcher
                    .as_ref()
                    .map(|matcher| {
                        visible_rows
                            .iter()
                            .enumerate()
                            .filter_map(|(position, row)| matcher(&row.value).then_some(position))
                            .collect()
                    })
                    .unwrap_or_default();
                let current_match = self
                    .memory_viewer
                    .selected_value_position
                    .and_then(|selected| {
                        matching_positions
                            .iter()
                            .position(|&position| position == selected)
                    })
                    .map_or(0, |index| index + 1);
                let total_matches = matching_positions.len();
                let mut find_previous_requested = false;
                let mut find_next_requested = false;
                ui.horizontal(|ui| {
                    ui.label("Find:");
                    matching_text_edit(
                        ui,
                        &mut self.memory_viewer.search_value,
                        "Find",
                        160.0,
                        &mut self.memory_viewer.search_match_mode,
                        &mut self.memory_viewer.search_case_insensitive,
                    );

                    if ui
                        .add(egui::Button::new(icons::ARROW_UP_LINE).frame(false))
                        .on_hover_text("Previous match")
                        .clicked()
                    {
                        find_previous_requested = true;
                    }

                    if ui
                        .add(egui::Button::new(icons::ARROW_DOWN_LINE).frame(false))
                        .on_hover_text("Next match")
                        .clicked()
                    {
                        find_next_requested = true;
                    }
                    if !self.memory_viewer.search_value.is_empty() {
                        if total_matches == 0 {
                            ui.label("No results");
                        } else {
                            ui.label(format!("{current_match} of {total_matches}"));
                        }
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Index format:");
                    egui::ComboBox::from_id_salt("memory_viewer_index_format")
                        .selected_text(self.memory_viewer.index_format.label())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.memory_viewer.index_format,
                                MemoryViewerFormat::Decimal,
                                "Decimal",
                            );
                            ui.selectable_value(
                                &mut self.memory_viewer.index_format,
                                MemoryViewerFormat::Hexadecimal,
                                "Hexadecimal",
                            );
                            ui.selectable_value(
                                &mut self.memory_viewer.index_format,
                                MemoryViewerFormat::Binary,
                                "Binary",
                            );
                        });

                    ui.label("Value format:");

                    let (mut preferred_translators, mut bad_translators): (Vec<_>, Vec<_>) =
                        variables
                            .first()
                            .and_then(|first_var_ref| {
                                wave_container.variable_meta(first_var_ref).ok()
                            })
                            .map_or_else(
                                || (vec![], self.translators.all_translator_names()),
                                |meta| {
                                    self.translators
                                        .all_translator_names()
                                        .into_iter()
                                        .partition(|translator_name| {
                                            let translator =
                                                self.translators.get_translator(translator_name);

                                            match translator.translates(&meta) {
                                                Ok(TranslationPreference::Yes) => true,
                                                Ok(TranslationPreference::Prefer) => true,
                                                Ok(TranslationPreference::No) => false,
                                                Err(_) => false,
                                            }
                                        })
                                },
                            );

                    preferred_translators.sort_by(|a, b| numeric_sort::cmp(a, b));
                    bad_translators.sort_by(|a, b| numeric_sort::cmp(a, b));

                    egui::ComboBox::from_id_salt("memory_viewer_value_format")
                        .selected_text(self.memory_viewer.value_format.clone())
                        .show_ui(ui, |ui| {
                            for name in preferred_translators {
                                ui.selectable_value(
                                    &mut self.memory_viewer.value_format,
                                    name.to_string(),
                                    name,
                                );
                            }

                            if !bad_translators.is_empty() {
                                ui.separator();
                                ui.label("Not recommended");

                                for name in bad_translators {
                                    ui.selectable_value(
                                        &mut self.memory_viewer.value_format,
                                        name.to_string(),
                                        name,
                                    );
                                }
                            }
                        });
                });

                let mut jump_requested = false;
                let mut shrink_columns_requested = false;

                ui.horizontal(|ui| {
                    ui.label("Jump to index:");
                    let response = ui.add_sized(
                        [60.0, 20.0],
                        egui::TextEdit::singleline(&mut self.memory_viewer.jump_to_index),
                    );

                    if ui.button("Jump").clicked()
                        || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                    {
                        jump_requested = true;
                    }
                    ui.label("Columns:");

                    ui.add(
                        DragValue::new(&mut self.memory_viewer.value_column_count).range(1..=32),
                    );
                    if ui
                        .add(Button::new(icons::ASPECT_RATIO_FILL).frame(false))
                        .on_hover_text("Auto-size")
                        .clicked()
                    {
                        shrink_columns_requested = true;
                    }
                    if self.memory_viewer.value_column_count > 1 {
                        self.memory_viewer.filter_mode = ChangeModes::AllValues;
                        self.memory_viewer.filter_value.clear();
                    }

                    ui.separator();
                    ui.separator();
                });

                ui.separator();

                let max_index = rows.iter().map(|row| row.index).max().unwrap_or(0);
                let text_height = egui::TextStyle::Body
                    .resolve(ui.style())
                    .size
                    .max(ui.spacing().interact_size.y);

                // let available_height = ui.available_height().max(250.0);
                let value_column_count = self
                    .memory_viewer
                    .value_column_count
                    .clamp(1, visible_rows.len().max(1));
                self.memory_viewer.value_column_count = value_column_count;
                let table_rows = visible_rows.len().div_ceil(value_column_count);

                let mut navigation_target = None;
                // jump to the closest memory index
                if jump_requested
                    && let Some(target_index) =
                        parse_jump_to_index(&self.memory_viewer.jump_to_index)
                    && let Some(value_position) = closest_row_index(&visible_rows, target_index)
                {
                    navigation_target = Some(value_position);
                }
                // Move to the next matching value, wrapping to the first match.
                if find_next_requested {
                    navigation_target = self
                        .memory_viewer
                        .selected_value_position
                        .and_then(|selected_position| {
                            matching_positions
                                .iter()
                                .copied()
                                .find(|position| *position > selected_position)
                        })
                        .or_else(|| matching_positions.first().copied());
                }
                // Move to the previous matching value, wrapping to the last match.
                if find_previous_requested {
                    navigation_target = self
                        .memory_viewer
                        .selected_value_position
                        .and_then(|selected_position| {
                            matching_positions
                                .iter()
                                .rev()
                                .copied()
                                .find(|position| *position < selected_position)
                        })
                        .or_else(|| matching_positions.last().copied());
                }
                if let Some(value_position) = navigation_target {
                    self.memory_viewer.selected_value_position = Some(value_position);

                    self.memory_viewer.scroll_to_row = Some(value_position / value_column_count);
                }

                let table_width = ui.available_width();
                let table_height = ui.available_height();

                let horizontal_navigation_target = navigation_target;
                ui.allocate_ui_with_layout(
                    egui::vec2(table_width, table_height),
                    egui::Layout::top_down(egui::Align::LEFT),
                    |ui| {
                        egui::ScrollArea::horizontal()
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                let mut table = TableBuilder::new(ui)
                                    .striped(true)
                                    .resizable(true)
                                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                                    .column(Column::auto());

                                for _ in 0..value_column_count {
                                    table = table.column(Column::auto());
                                }
                                if shrink_columns_requested {
                                    table.reset();
                                }
                                if let Some(row_index) = self.memory_viewer.scroll_to_row.take() {
                                    table =
                                        table.scroll_to_row(row_index, Some(egui::Align::Center));
                                }
                                table
                                    .header(24.0, |mut header| {
                                        header.col(|ui| {
                                            ui.monospace("Index");
                                        });

                                        for column_index in 0..value_column_count {
                                            header.col(|ui| {
                                                if column_index == 0 {
                                                    ui.monospace("Value");
                                                } else {
                                                    ui.monospace(format!("+{column_index}"));
                                                }
                                            });
                                        }
                                    })
                                    .body(|body| {
                                        body.rows(text_height, table_rows, |mut row| {
                                            let table_row_index = row.index();
                                            let row_start = table_row_index * value_column_count;

                                            let Some(first_row_data) = visible_rows.get(row_start)
                                            else {
                                                return;
                                            };

                                            row.col(|ui| {
                                                ui.monospace(format_index(
                                                    first_row_data.index,
                                                    self.memory_viewer.index_format,
                                                    max_index,
                                                ));
                                            });

                                            for column_index in 0..value_column_count {
                                                row.col(|ui| {
                                                    let value_index = row_start + column_index;

                                                    let Some(row_data) =
                                                        visible_rows.get(value_index)
                                                    else {
                                                        return;
                                                    };

                                                    let is_selected =
                                                        self.memory_viewer.selected_value_position
                                                            == Some(value_index);
                                                    let highlighted_by_change = match self
                                                        .memory_viewer
                                                        .highlight_mode
                                                    {
                                                        ChangeModes::AllValues => false,

                                                        ChangeModes::ChangedAtCursor => {
                                                            row_data.changed_at_cursor
                                                        }

                                                        ChangeModes::ChangedBtwCursorAndMarker(
                                                            _,
                                                        ) => row_data.changed_for_highlight,
                                                    };

                                                    let highlighted_by_value =
                                                        highlight_matcher.as_ref().is_some_and(
                                                            |matcher| matcher(&row_data.value),
                                                        );

                                                    let is_highlighted = highlighted_by_change
                                                        || highlighted_by_value;
                                                    let frame = if is_selected {
                                                        egui::Frame::NONE
                                                            .fill(
                                                                self.user
                                                                    .config
                                                                    .theme
                                                                    .accent_info
                                                                    .background,
                                                            )
                                                            .inner_margin(egui::Margin::symmetric(
                                                                4, 1,
                                                            ))
                                                    } else if is_highlighted {
                                                        egui::Frame::NONE
                                                            .fill(
                                                                self.user
                                                                    .config
                                                                    .theme
                                                                    .selected_elements_colors
                                                                    .background,
                                                            )
                                                            .inner_margin(egui::Margin::symmetric(
                                                                4, 1,
                                                            ))
                                                    } else {
                                                        egui::Frame::NONE.inner_margin(
                                                            egui::Margin::symmetric(4, 1),
                                                        )
                                                    };

                                                    let frame_response = frame.show(ui, |ui| {
                                                        if is_selected {
                                                            ui.visuals_mut().override_text_color =
                                                                Some(
                                                                    self.user
                                                                        .config
                                                                        .theme
                                                                        .accent_info
                                                                        .foreground,
                                                                );
                                                        } else if is_highlighted {
                                                            ui.visuals_mut().override_text_color =
                                                                Some(
                                                                    self.user
                                                                        .config
                                                                        .theme
                                                                        .selected_elements_colors
                                                                        .foreground,
                                                                );
                                                        }

                                                        if self.memory_viewer.color_values {
                                                            let color = row_data.kind.color(
                                                                self.user
                                                                    .config
                                                                    .theme
                                                                    .variable_default,
                                                                &self.user.config.theme,
                                                            );

                                                            ui.horizontal(|ui| {
                                                                ui.colored_label(color, "■");
                                                                ui.monospace(&row_data.value);
                                                            });
                                                        } else {
                                                            ui.monospace(&row_data.value);
                                                        }
                                                    });
                                                    if horizontal_navigation_target
                                                        == Some(value_index)
                                                    {
                                                        ui.scroll_to_rect(
                                                            frame_response.response.rect,
                                                            Some(egui::Align::Center),
                                                        );
                                                    }
                                                });
                                            }
                                        });
                                    });
                            });
                    },
                );
            });

        self.memory_viewer.open = open;
    }
}
