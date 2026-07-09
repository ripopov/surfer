use ecolor::Color32;
use egui::{FontId, PointerButton, Response, Sense, Ui};
use emath::{Align2, Pos2, Rect, RectTransform, Vec2};
use epaint::{CornerRadius, CubicBezierShape, PathShape, PathStroke, RectShape, Shape, Stroke};
use eyre::WrapErr as _;
use ftr_parser::types::{Transaction, TxGenerator};
use itertools::Itertools;
use num::bigint::{ToBigInt, ToBigUint};
use num::{BigInt, ToPrimitive};
use rayon::prelude::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use std::collections::HashMap;
use std::f32::consts::PI;
use surfer_translation_types::{
    NumericRange, SubFieldFlatTranslationResult, TranslatedValue, ValueKind, VariableInfo,
};
use tracing::{error, warn};

use crate::CachedDrawData::TransactionDrawData;
use crate::analog_renderer::{AnalogDrawingCommand, variable_analog_draw_commands};
use crate::clock_highlighting::draw_clock_edge_marks;
use crate::config::SurferTheme;
use crate::data_container::DataContainer;
use crate::displayed_item::{
    AnalogSettings, DisplayedFieldRef, DisplayedItemRef, DisplayedVariable,
};
use crate::source::{SourceId, SourceTransactionRef};
use crate::time::TimeFormatter;
use crate::tooltips::handle_transaction_tooltip_for_source;
use crate::trace_style::{TraceStyle, TraceValue};
use crate::transaction_container::{TransactionRef, TransactionStreamRef};
use crate::transaction_events::EventDisplayMode;
use crate::translation::{TranslationResultExt, TranslatorList, ValueKindExt, VariableInfoExt};
use crate::view::{DrawConfig, DrawingContext, ItemDrawingInfo};
use crate::wave_container::{QueryResult, VariableRefExt};
use crate::wave_data::WaveData;
use crate::{
    CachedDrawData, CachedMixedDrawData, CachedTransactionDrawData, CachedWaveDrawData, Message,
    SystemState, displayed_item::DisplayedItem,
};

pub struct DrawnRegion {
    pub inner: Option<TranslatedValue>,
    /// True if a transition should be drawn even if there is no change in the value
    /// between the previous and next pixels. Only used by the bool drawing logic to
    /// draw draw a vertical line and prevent apparent aliasing
    force_anti_alias: bool,
    trace_value: TraceValue,
}

pub enum DrawingCommands {
    Digital(DigitalDrawingCommands),
    Analog(AnalogDrawingCommands),
}

pub enum AnalogDrawingCommands {
    /// Cache is still being built
    Loading,
    /// Cache is ready with drawing data
    Ready {
        /// Viewport min/max for the visible signal range (used for Y-axis scaling)
        viewport_min: f64,
        viewport_max: f64,
        /// Global min/max across entire signal (used for global Y-axis scaling)
        global_min: f64,
        global_max: f64,
        /// Type limits min/max from the translator (used for `TypeLimits` Y-axis scaling)
        type_limits: Option<NumericRange>,
        /// Per-pixel drawing commands with flat spans and ranges
        values: Vec<AnalogDrawingCommand>,
        /// Pixel position of timestamp 0 (start of signal data).
        min_valid_pixel: f32,
        /// Pixel position of last timestamp (end of signal data).
        max_valid_pixel: f32,
        analog_settings: AnalogSettings,
    },
}
#[derive(Clone, PartialEq, Debug)]
pub enum DigitalDrawingType {
    Bool,
    Clock,
    Event,
    Vector,
}

impl From<&VariableInfo> for DigitalDrawingType {
    fn from(info: &VariableInfo) -> Self {
        match info {
            VariableInfo::Bool => DigitalDrawingType::Bool,
            VariableInfo::Clock => DigitalDrawingType::Clock,
            VariableInfo::Event => DigitalDrawingType::Event,
            _ => DigitalDrawingType::Vector,
        }
    }
}
/// List of values to draw for a variable. It is an ordered list of values that should
/// be drawn at the *start time* until the *start time* of the next value
pub struct DigitalDrawingCommands {
    pub drawing_type: DigitalDrawingType,
    pub values: Vec<(f32, DrawnRegion)>,
}

impl DigitalDrawingCommands {
    #[must_use]
    pub fn new_from_variable_info(info: &VariableInfo) -> Self {
        DigitalDrawingCommands {
            drawing_type: DigitalDrawingType::from(info),
            values: vec![],
        }
    }

    pub fn push(&mut self, val: (f32, DrawnRegion)) {
        self.values.push(val);
    }
}

/// Minimum on-screen width of the clickable area of a transaction. Transactions
/// narrower than this (e.g. zero-duration events) get their hit area expanded to
/// this width so they can still be hovered and clicked.
const MIN_TRANSACTION_CLICK_WIDTH: f32 = 6.0;

/// Event markers whose start positions are within this many pixels aggregate
/// into one cluster glyph with a count badge.
const EVENT_CLUSTER_MERGE_PX: f32 = 8.0;

/// Minimum on-screen duration for an event to get a duration bracket along
/// the bottom edge in addition to its diamond marker.
const EVENT_DURATION_BRACKET_MIN_PX: f32 = 3.0;

/// How a transaction is drawn on the canvas.
pub enum TxDrawKind {
    /// Ordinary transaction rectangle
    Rect,
    /// Diamond marker at the start time (zero-duration transactions and FTR
    /// events). Non-zero durations additionally draw a bracket along the
    /// bottom edge of the lane.
    EventMarker {
        /// Event recorded outside its parent's time range: drawn hollow with
        /// a warning tint
        out_of_range: bool,
    },
    /// Aggregated marker for events that share a pixel column. Clicking it
    /// zooms the viewport to the covered time span.
    EventCluster {
        count: usize,
        /// Per-event-name counts, in first-seen order
        names: Vec<(String, usize)>,
        time_span: (BigInt, BigInt),
        contains_focused: bool,
        any_out_of_range: bool,
    },
}

pub struct TxDrawingCommands {
    min: Pos2,
    max: Pos2,
    kind: TxDrawKind,
    gen_ref: TransactionStreamRef, // makes it easier to later access the actual Transaction object
}

/// Where a generator pass places its transactions within a displayed row.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EventPlacement {
    /// The generator's own lanes (`tx.row`), shifted down by `lane_offset`
    /// (used by the separate-row event mode)
    OwnLanes { lane_offset: usize },
    /// Each event draws on the lane of its parent transaction
    ParentLanes,
}

/// One generator drawn into a displayed row.
struct GeneratorPass<'a> {
    generator: &'a TxGenerator,
    placement: EventPlacement,
    /// Apply FTR event semantics: marker clustering, out-of-range tinting,
    /// and orphan handling
    event_semantics: bool,
}

/// One event considered for cluster aggregation.
struct EventClusterMember {
    tx_ref: TransactionRef,
    name: Option<String>,
    min: Pos2,
    max: Pos2,
    start_time: BigInt,
    end_time: BigInt,
    out_of_range: bool,
    focused: bool,
}

/// Accumulates events that share a pixel column on one lane.
struct EventClusterAccum {
    /// First member: provides the command identity and marker anchor
    rep: TransactionRef,
    anchor_px: f32,
    min: Pos2,
    max: Pos2,
    count: usize,
    names: Vec<(String, usize)>,
    time_span: (BigInt, BigInt),
    any_out_of_range: bool,
    contains_focused: bool,
}

impl EventClusterAccum {
    fn new(member: EventClusterMember) -> Self {
        let mut accum = EventClusterAccum {
            rep: member.tx_ref.clone(),
            anchor_px: member.min.x,
            min: member.min,
            max: member.max,
            count: 0,
            names: vec![],
            time_span: (member.start_time.clone(), member.end_time.clone()),
            any_out_of_range: false,
            contains_focused: false,
        };
        accum.merge(member);
        accum
    }

    fn merge(&mut self, member: EventClusterMember) {
        self.count += 1;
        self.max.x = self.max.x.max(member.max.x);
        self.time_span.1 = self.time_span.1.clone().max(member.end_time);
        self.any_out_of_range |= member.out_of_range;
        self.contains_focused |= member.focused;
        let name = member.name.unwrap_or_default();
        if let Some(entry) = self.names.iter_mut().find(|(n, _)| *n == name) {
            entry.1 += 1;
        } else {
            self.names.push((name, 1));
        }
    }

    /// Emits a single marker or a cluster command for the accumulated events.
    fn emit(
        self,
        gen_ref: &TransactionStreamRef,
        commands: &mut HashMap<TransactionRef, TxDrawingCommands>,
        displayed_transactions: &mut Vec<TransactionRef>,
    ) {
        let kind = if self.count == 1 {
            TxDrawKind::EventMarker {
                out_of_range: self.any_out_of_range,
            }
        } else {
            TxDrawKind::EventCluster {
                count: self.count,
                names: self.names,
                time_span: self.time_span,
                contains_focused: self.contains_focused,
                any_out_of_range: self.any_out_of_range,
            }
        };
        displayed_transactions.push(self.rep.clone());
        commands.insert(
            self.rep,
            TxDrawingCommands {
                min: self.min,
                max: self.max,
                kind,
                gen_ref: gen_ref.clone(),
            },
        );
    }
}

pub(crate) struct VariableDrawCommands {
    pub(crate) draw_clock_edges: bool,
    pub(crate) clock_edges: Vec<f32>,
    pub(crate) display_id: DisplayedItemRef,
    pub(crate) local_commands: HashMap<Vec<String>, DrawingCommands>,
    pub(crate) local_msgs: Vec<Message>,
}

/// Common setup for variable draw commands: extracts metadata and determines rendering mode.
/// Routes to either analog or digital command generation.
#[allow(clippy::too_many_arguments)]
fn variable_draw_commands(
    displayed_variable: &DisplayedVariable,
    display_id: DisplayedItemRef,
    timestamps: &[(f32, num::BigUint)],
    waves: &WaveData,
    translators: &TranslatorList,
    view_width: f32,
    viewport_idx: usize,
    trace_style: TraceStyle,
) -> Option<VariableDrawCommands> {
    let wave_container = waves.waves_for_source(displayed_variable.source)?;

    let signal_id = wave_container
        .signal_id(&displayed_variable.variable_ref)
        .ok()?;
    if !wave_container.is_signal_loaded(&signal_id) {
        return None;
    }

    let meta = match wave_container
        .variable_meta(&displayed_variable.variable_ref)
        .context("failed to get variable meta")
    {
        Ok(meta) => meta,
        Err(e) => {
            warn!("{e:#?}");
            return None;
        }
    };

    let displayed_field_ref: DisplayedFieldRef = display_id.into();
    let translator = waves.variable_translator_with_meta(&displayed_field_ref, translators, &meta);
    let info = translator.variable_info(&meta).unwrap();

    let is_analog_mode = displayed_variable.analog.is_some();
    let is_bool = matches!(
        info,
        VariableInfo::Bool | VariableInfo::Clock | VariableInfo::Event
    );

    if is_analog_mode && !is_bool {
        variable_analog_draw_commands(
            displayed_variable,
            display_id,
            waves,
            translators,
            view_width,
            viewport_idx,
        )
    } else {
        variable_digital_draw_commands(
            displayed_variable,
            display_id,
            timestamps,
            waves,
            translators,
            wave_container,
            &meta,
            translator,
            &info,
            view_width,
            viewport_idx,
            trace_style,
        )
    }
}

/// Generate draw commands for digital waveform rendering.
#[allow(clippy::too_many_arguments)]
fn variable_digital_draw_commands(
    displayed_variable: &DisplayedVariable,
    display_id: DisplayedItemRef,
    timestamps: &[(f32, num::BigUint)],
    waves: &WaveData,
    translators: &TranslatorList,
    wave_container: &crate::wave_container::WaveContainer,
    meta: &crate::wave_container::VariableMeta,
    translator: &crate::translation::DynTranslator,
    info: &VariableInfo,
    view_width: f32,
    viewport_idx: usize,
    trace_style: TraceStyle,
) -> Option<VariableDrawCommands> {
    let mut clock_edges = vec![];
    let mut local_msgs = vec![];
    let displayed_field_ref: DisplayedFieldRef = display_id.into();
    let num_timestamps = waves.safe_canvas_num_timestamps();

    let mut local_commands: HashMap<Vec<String>, DigitalDrawingCommands> = HashMap::new();

    let mut prev_values = HashMap::new();

    // In order to insert a final draw command at the end of a trace,
    // we need to know if this is the last timestamp to draw
    let end_pixel = timestamps.iter().last().map(|t| t.0).unwrap_or_default();
    // The first pixel we actually draw is the second pixel in the
    // list, since we skip one pixel to have a previous value
    let start_pixel = timestamps.get(1).map(|t| t.0).unwrap_or_default();

    // Iterate over all the time stamps to draw on
    let mut next_change = timestamps.first().map(|t| t.0).unwrap_or_default();
    for ((_, prev_time), (pixel, time)) in timestamps.iter().zip(timestamps.iter().skip(1)) {
        let is_last_timestep = pixel == &end_pixel;
        let is_first_timestep = pixel == &start_pixel;

        if *pixel < next_change && !is_first_timestep && !is_last_timestep {
            continue;
        }

        let query_result = wave_container.query_variable(&displayed_variable.variable_ref, time);
        next_change = match &query_result {
            Ok(Some(QueryResult {
                next: Some(timestamp),
                ..
            })) => waves.viewports[viewport_idx].pixel_from_time(
                &timestamp.to_bigint().unwrap(),
                view_width,
                &num_timestamps,
            ),
            // If we don't have a next timestamp, we don't need to recheck until the last time
            // step
            Ok(_) => timestamps.last().map(|t| t.0).unwrap_or_default(),
            // If we get an error here, we'll let the next match block handle it, but we'll take
            // note that we need to recheck every pixel until the end
            _ => timestamps.first().map(|t| t.0).unwrap_or_default(),
        };

        let (change_time, val) = match query_result {
            Ok(Some(QueryResult {
                current: Some((change_time, val)),
                ..
            })) => (change_time, val),
            Ok(Some(QueryResult { current: None, .. }) | None) => continue,
            Err(e) => {
                error!("Variable query error {e:#?}");
                continue;
            }
        };

        // Check if the value remains unchanged between this pixel
        // and the last
        if &change_time < prev_time && !is_first_timestep && !is_last_timestep {
            continue;
        }

        let translation_result = match translator.translate(meta, &val) {
            Ok(result) => result,
            Err(e) => {
                error!(
                    "{translator_name} for {variable_name} failed. Disabling:",
                    translator_name = translator.name(),
                    variable_name = displayed_variable.variable_ref.full_path_string_no_index()
                );
                error!("{e:#}");
                local_msgs.push(Message::ResetVariableFormat(displayed_field_ref));
                return None;
            }
        };

        let fields = translation_result.format_flat(
            &displayed_variable.format,
            &displayed_variable.field_formats,
            translators,
        );

        let trace_value = TraceValue::from_value(&val, meta.num_bits, trace_style);

        for SubFieldFlatTranslationResult { names, value } in fields {
            let entry = local_commands.entry(names.clone()).or_insert_with(|| {
                DigitalDrawingCommands::new_from_variable_info(info.get_subinfo(&names))
            });

            let prev = prev_values.get(&names);

            // If the value changed between this and the previous pixel, we want to
            // draw a transition even if the translated value didn't change.  We
            // only want to do this for root variables, because resolving when a
            // sub-field change is tricky without more information from the
            // translators
            let anti_alias = &change_time > prev_time
                && names.is_empty()
                && wave_container.wants_anti_aliasing();
            let new_value = prev != Some(&value);

            // This is not the value we drew last time
            if new_value || is_last_timestep || anti_alias {
                prev_values
                    .entry(names.clone())
                    .or_insert(value.clone())
                    .clone_from(&value);

                if entry.drawing_type == DigitalDrawingType::Clock {
                    match value.as_ref().map(|result| result.value.as_str()) {
                        Some("1") if !is_last_timestep && !is_first_timestep => {
                            clock_edges.push(*pixel);
                        }
                        Some(_) => {}
                        None => {}
                    }
                }

                entry.push((
                    *pixel,
                    DrawnRegion {
                        inner: value,
                        force_anti_alias: anti_alias && !new_value,
                        trace_value,
                    },
                ));
            }
        }
    }
    let draw_clock_edges = match clock_edges.as_slice() {
        [] => false,
        [_single] => true,
        [first, second, ..] => second - first > 20.,
    };

    Some(VariableDrawCommands {
        draw_clock_edges,
        clock_edges,
        display_id,
        local_commands: local_commands
            .into_iter()
            .map(|(k, v)| (k, DrawingCommands::Digital(v)))
            .collect(),
        local_msgs,
    })
}

impl SystemState {
    fn canvas_pos_to_item_space(&self, response: &Response, waves: &WaveData, pos: Pos2) -> Pos2 {
        let item_y_offset = (waves.top_item_draw_offset - response.rect.top()).max(0.0);

        Pos2 {
            x: pos.x,
            y: pos.y - item_y_offset,
        }
    }

    fn sorted_drawing_infos(waves: &WaveData) -> Vec<&ItemDrawingInfo> {
        let mut sorted = waves.drawing_infos.iter().collect::<Vec<_>>();
        sorted.sort_by(|a, b| a.top().total_cmp(&b.top()));
        sorted
    }

    fn source_tail_start_pixel(
        waves: &WaveData,
        source: SourceId,
        viewport_idx: usize,
        cfg: &DrawConfig,
    ) -> Option<f32> {
        let canvas_max_timestamp = waves.canvas_num_timestamps()?.to_biguint()?;
        let source_domain = waves.time_domain_for_source(source)?;
        if source_domain.max_timestamp >= canvas_max_timestamp {
            return None;
        }

        let source_end = source_domain.max_timestamp.to_bigint()?;
        Some(waves.viewports[viewport_idx].pixel_from_time(
            &source_end,
            cfg.canvas_size.x - 1.0,
            &waves.safe_canvas_num_timestamps(),
        ))
    }

    fn draw_source_tail_overlay(
        &self,
        tail_start_pixel: f32,
        drawing_info: &ItemDrawingInfo,
        ctx: &mut DrawingContext,
    ) {
        if tail_start_pixel >= ctx.cfg.canvas_size.x {
            return;
        }

        let tail_start_pixel = tail_start_pixel.clamp(0.0, ctx.cfg.canvas_size.x);
        let left = (ctx.to_screen)(tail_start_pixel, 0.0).x;
        let right = (ctx.to_screen)(ctx.cfg.canvas_size.x, 0.0).x;
        if right <= left {
            return;
        }

        let top = drawing_info.top();
        let bottom = drawing_info.bottom();
        let fill_source = ctx.theme.accent_warn.background;
        let fill =
            Color32::from_rgba_unmultiplied(fill_source.r(), fill_source.g(), fill_source.b(), 72);
        ctx.painter.rect_filled(
            Rect {
                min: Pos2::new(left, top),
                max: Pos2::new(right, bottom),
            },
            CornerRadius::ZERO,
            fill,
        );

        let marker_color = ctx.theme.accent_warn.foreground;
        ctx.painter
            .vline(left, top..=bottom, Stroke::new(2.0, marker_color));

        let hatch_stroke = Stroke::new(1.0, marker_color.gamma_multiply(0.45));
        let row_height = bottom - top;
        let mut hatch_x = left + 8.0;
        while hatch_x < right + row_height {
            ctx.painter.line_segment(
                [
                    Pos2::new(hatch_x, bottom),
                    Pos2::new(hatch_x + row_height, top),
                ],
                hatch_stroke,
            );
            hatch_x += 8.0;
        }
    }

    fn draw_tick_lines(
        &self,
        waves: &WaveData,
        ticks: &[(String, f32, i64)],
        ctx: &mut DrawingContext,
    ) {
        if ticks.is_empty() || !self.show_ticks() {
            return;
        }

        let stroke = Stroke::from(&self.user.config.theme.ticks.style);
        for (_, x, _) in ticks {
            waves.draw_tick_line(*x, ctx, &stroke);
        }
    }

    pub fn invalidate_draw_commands(&mut self) {
        if let Some(waves) = &self.user.waves {
            for viewport in 0..waves.viewports.len() {
                self.draw_data.borrow_mut()[viewport] = None;
            }
        }
    }

    pub fn generate_draw_commands(
        &self,
        cfg: &DrawConfig,
        msgs: &mut Vec<Message>,
        viewport_idx: usize,
    ) {
        #[cfg(feature = "performance_plot")]
        self.timing.borrow_mut().start("Generate draw commands");
        if let Some(waves) = &self.user.waves {
            let has_wave_rows = waves
                .displayed_items
                .values()
                .any(|item| matches!(item, DisplayedItem::Variable(_)));
            let has_stream_rows = waves
                .displayed_items
                .values()
                .any(|item| matches!(item, DisplayedItem::Stream(_)));
            let has_source_rows = has_wave_rows || has_stream_rows;
            let draw_data = if has_source_rows && waves.source_count() > 1 {
                self.generate_mixed_draw_commands(waves, cfg, msgs, viewport_idx)
            } else {
                match waves.inner {
                    DataContainer::Waves(_) => {
                        self.generate_wave_draw_commands(waves, cfg, msgs, viewport_idx)
                    }
                    DataContainer::Transactions(_) => {
                        self.generate_transaction_draw_commands(waves, cfg, msgs, viewport_idx)
                    }
                    DataContainer::Empty => None,
                }
            };
            self.draw_data.borrow_mut()[viewport_idx] = draw_data;
        }
        #[cfg(feature = "performance_plot")]
        self.timing.borrow_mut().end("Generate draw commands");
    }

    fn generate_mixed_draw_commands(
        &self,
        waves: &WaveData,
        cfg: &DrawConfig,
        msgs: &mut Vec<Message>,
        viewport_idx: usize,
    ) -> Option<CachedDrawData> {
        let wave = match self.generate_wave_draw_commands(waves, cfg, msgs, viewport_idx) {
            Some(CachedDrawData::WaveDrawData(data)) => Some(data),
            _ => None,
        };

        let tx_cfg = DrawConfig::new(
            cfg.canvas_size,
            self.user.config.layout.transactions_line_height,
            cfg.text_size,
        );
        let transaction_sources = waves
            .items_tree
            .iter_visible()
            .filter_map(|node| waves.displayed_items.get(&node.item_ref))
            .filter_map(|item| match item {
                DisplayedItem::Stream(stream) => Some(stream.source),
                _ => None,
            })
            .unique()
            .collect::<Vec<_>>();

        let transactions = transaction_sources
            .into_iter()
            .filter_map(|source| {
                self.generate_transaction_draw_commands_for_source(
                    waves,
                    &tx_cfg,
                    msgs,
                    viewport_idx,
                    source,
                )
                .map(|data| (source, data))
            })
            .collect::<HashMap<_, _>>();

        Some(CachedDrawData::MixedDrawData(CachedMixedDrawData {
            wave,
            transactions,
        }))
    }

    fn generate_wave_draw_commands(
        &self,
        waves: &WaveData,
        cfg: &DrawConfig,
        msgs: &mut Vec<Message>,
        viewport_idx: usize,
    ) -> Option<CachedDrawData> {
        let mut draw_commands = HashMap::new();

        let num_timestamps = waves.safe_canvas_num_timestamps();
        let max_time = num_timestamps.to_f64().unwrap_or(f64::MAX);
        let mut clock_edges_by_clock = vec![];
        let viewport = waves.viewports[viewport_idx];
        // Compute which timestamp to draw in each pixel. We'll draw from -extra_draw_width to
        // width + extra_draw_width in order to draw initial transitions outside the screen
        let timestamps = (-cfg.extra_draw_width..(cfg.canvas_size.x as i32 + cfg.extra_draw_width))
            .into_par_iter()
            .filter_map(|x| {
                let time = viewport
                    .as_absolute_time(f64::from(x), cfg.canvas_size.x, &num_timestamps)
                    .0;
                if time < 0. || time > max_time {
                    None
                } else {
                    Some((x as f32, time.to_biguint().unwrap_or_default()))
                }
            })
            .collect::<Vec<_>>();

        let trace_style = self.trace_style();
        let translators = &self.translators;
        let commands = waves
            .items_tree
            .iter_visible()
            .map(|node| (node.item_ref, waves.displayed_items.get(&node.item_ref)))
            .filter_map(|(id, item)| match item {
                Some(DisplayedItem::Variable(variable_ref)) => Some((id, variable_ref)),
                _ => None,
            })
            .collect::<Vec<_>>()
            .par_iter()
            .cloned()
            // Iterate over the variables, generating draw commands for all the
            // subfields
            .filter_map(|(id, displayed_variable)| {
                variable_draw_commands(
                    displayed_variable,
                    id,
                    &timestamps,
                    waves,
                    translators,
                    cfg.canvas_size.x,
                    viewport_idx,
                    trace_style,
                )
            })
            .collect::<Vec<_>>();

        let mut clock_variable_count = 0usize;
        for VariableDrawCommands {
            draw_clock_edges,
            clock_edges: mut new_clock_edges,
            display_id,
            local_commands,
            mut local_msgs,
        } in commands
        {
            msgs.append(&mut local_msgs);
            for (field, val) in local_commands {
                draw_commands.insert(
                    DisplayedFieldRef {
                        item: display_id,
                        field,
                    },
                    val,
                );
            }

            let is_clock_variable = !new_clock_edges.is_empty();
            if is_clock_variable {
                if draw_clock_edges {
                    clock_edges_by_clock
                        .push((clock_variable_count, std::mem::take(&mut new_clock_edges)));
                }
                clock_variable_count += 1;
            }
        }

        let clock_edges = self.get_clock_hightlight_data(clock_edges_by_clock);

        let ticks = self.get_ticks_for_viewport_idx(waves, viewport_idx, cfg);

        Some(CachedDrawData::WaveDrawData(CachedWaveDrawData {
            draw_commands,
            clock_edges,
            ticks,
        }))
    }

    fn generate_transaction_draw_commands(
        &self,
        waves: &WaveData,
        cfg: &DrawConfig,
        msgs: &mut Vec<Message>,
        viewport_idx: usize,
    ) -> Option<CachedDrawData> {
        self.generate_transaction_draw_commands_for_source(
            waves,
            cfg,
            msgs,
            viewport_idx,
            WaveData::primary_source_id(),
        )
        .map(TransactionDrawData)
    }

    fn generate_transaction_draw_commands_for_source(
        &self,
        waves: &WaveData,
        cfg: &DrawConfig,
        msgs: &mut Vec<Message>,
        viewport_idx: usize,
        source: SourceId,
    ) -> Option<CachedTransactionDrawData> {
        let mut draw_commands: HashMap<
            TransactionStreamRef,
            HashMap<TransactionRef, TxDrawingCommands>,
        > = HashMap::new();
        let mut stream_to_displayed_txs = HashMap::new();
        let mut inc_relation_tx_ids = vec![];
        let mut out_relation_tx_ids = vec![];

        let (session_focused_tx_ref, old_focused_tx) = &waves.focused_transaction;
        let focused_tx_ref = session_focused_tx_ref
            .as_ref()
            .filter(|focused| focused.source == source)
            .map(|focused| &focused.inner);
        let mut new_focused_tx: Option<&Transaction> = None;
        // The focused event was drawn on its parent's lane in some row, so
        // the parent_of relation arrow would be noise
        let mut focused_event_overlaid = false;

        let events_enabled = self.user.config.behavior.ftr_events_enabled();
        let container = waves.transactions_for_source(source)?;
        let event_index = container.event_index();

        let viewport = waves.viewports[viewport_idx];
        let num_timestamps = waves.safe_canvas_num_timestamps();

        let displayed_streams = waves
            .items_tree
            .iter_visible()
            .map(|node| node.item_ref)
            .collect::<Vec<_>>()
            .par_iter()
            .map(|id| waves.displayed_items.get(id))
            .filter_map(|item| match item {
                Some(DisplayedItem::Stream(stream_ref)) if stream_ref.source == source => {
                    Some(stream_ref)
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        let first_visible_timestamp =
            viewport.curr_left.absolute(&num_timestamps).0.max(0.0) as u64;

        for displayed_stream in displayed_streams {
            let tx_stream_ref = &displayed_stream.transaction_stream_ref;
            let mut displayed_transactions = vec![];
            let mut row_commands: HashMap<TransactionRef, TxDrawingCommands> = HashMap::new();

            // Plan the generator passes for this row: every generator draws
            // either on its own lanes or overlaid onto parent lanes
            let mut passes: Vec<GeneratorPass> = vec![];
            if let Some(gen_id) = tx_stream_ref.gen_id {
                let generator = container.get_generator(gen_id)?;
                passes.push(GeneratorPass {
                    generator,
                    placement: EventPlacement::OwnLanes { lane_offset: 0 },
                    event_semantics: events_enabled
                        && event_index.is_conforming_events_generator(gen_id),
                });
                if events_enabled
                    && let Some(events_gen_id) = event_index.conforming_events_generator_of(gen_id)
                    && let Some(events_generator) = container.get_generator(events_gen_id)
                {
                    match displayed_stream.event_display_mode {
                        EventDisplayMode::Overlay => passes.push(GeneratorPass {
                            generator: events_generator,
                            placement: EventPlacement::ParentLanes,
                            event_semantics: true,
                        }),
                        EventDisplayMode::SeparateRow => passes.push(GeneratorPass {
                            generator: events_generator,
                            placement: EventPlacement::OwnLanes {
                                lane_offset: event_index.lane_count(gen_id),
                            },
                            event_semantics: true,
                        }),
                        EventDisplayMode::Hidden => {}
                    }
                }
            } else {
                let stream = container.get_stream(tx_stream_ref.stream_id)?;
                for gen_id in &stream.generators {
                    let generator = container.get_generator(*gen_id)?;
                    if events_enabled && event_index.is_conforming_events_generator(*gen_id) {
                        // In whole-stream rows, events always overlay their
                        // parent's lanes instead of occupying generator lanes
                        if displayed_stream.event_display_mode != EventDisplayMode::Hidden {
                            passes.push(GeneratorPass {
                                generator,
                                placement: EventPlacement::ParentLanes,
                                event_semantics: true,
                            });
                        }
                    } else {
                        passes.push(GeneratorPass {
                            generator,
                            placement: EventPlacement::OwnLanes { lane_offset: 0 },
                            event_semantics: false,
                        });
                    }
                }
            }

            for pass in passes {
                let generator = pass.generator;
                let gen_ref = TransactionStreamRef::new_gen(
                    generator.stream_id,
                    generator.id,
                    generator.name.clone(),
                );

                // find first visible transaction
                let first_visible_transaction_index =
                    match generator.transactions.binary_search_by_key(
                        &first_visible_timestamp,
                        ftr_parser::types::Transaction::get_end_time,
                    ) {
                        Ok(i) | Err(i) => i,
                    }
                    .saturating_sub(1);
                let transactions = generator
                    .transactions
                    .iter()
                    .skip(first_visible_transaction_index);

                let mut last_px = f32::NAN;
                // Per-lane cluster accumulators for event passes
                let mut clusters: HashMap<usize, EventClusterAccum> = HashMap::new();

                for tx in transactions {
                    let start_time = tx.get_start_time();
                    let end_time = tx.get_end_time();
                    let curr_tx_id = tx.get_tx_id();

                    // stop drawing after last visible transaction
                    if start_time.to_f64().unwrap()
                        > viewport.curr_right.absolute(&num_timestamps).0
                    {
                        break;
                    }

                    if let Some(focused_tx_ref) = focused_tx_ref
                        && curr_tx_id == focused_tx_ref.id
                    {
                        new_focused_tx = Some(tx);
                    }

                    let min_px = viewport.pixel_from_time(
                        &start_time.to_bigint().unwrap(),
                        cfg.canvas_size.x - 1.,
                        &num_timestamps,
                    );
                    let max_px = viewport.pixel_from_time(
                        &end_time.to_bigint().unwrap(),
                        cfg.canvas_size.x - 1.,
                        &num_timestamps,
                    );

                    let tx_ref = TransactionRef { id: curr_tx_id };
                    let event_info = pass
                        .event_semantics
                        .then(|| container.event_info(curr_tx_id))
                        .flatten();
                    if pass.event_semantics
                        && pass.placement == EventPlacement::ParentLanes
                        && event_info.is_none()
                    {
                        continue;
                    }

                    let lane = match pass.placement {
                        EventPlacement::OwnLanes { lane_offset } => tx.row + lane_offset,
                        // Overlay passes only reach conforming events, which
                        // have a resolved parent lane.
                        EventPlacement::ParentLanes => event_info
                            .and_then(|info| {
                                let (gen_id, idx) = event_index.lookup_tx(info.parent_tx)?;
                                Some(container.get_generator(gen_id)?.transactions.get(idx)?.row)
                            })
                            .unwrap_or(0),
                    };
                    let lane_min_y = cfg.line_height * lane as f32 + 4.0;
                    let lane_max_y = cfg.line_height * (lane + 1) as f32 - 4.0;

                    if pass.event_semantics && event_info.is_some() {
                        let is_focused = focused_tx_ref
                            .as_ref()
                            .is_some_and(|focused| focused.id == curr_tx_id);
                        if is_focused && pass.placement == EventPlacement::ParentLanes {
                            focused_event_overlaid = true;
                        }
                        let member = EventClusterMember {
                            tx_ref,
                            name: crate::transaction_events::event_name(tx),
                            min: Pos2::new(min_px, lane_min_y),
                            max: Pos2::new(max_px, lane_max_y),
                            start_time: start_time.to_bigint().unwrap(),
                            end_time: end_time.to_bigint().unwrap(),
                            out_of_range: event_info.is_some_and(|info| info.out_of_range),
                            focused: is_focused,
                        };
                        match clusters.entry(lane) {
                            std::collections::hash_map::Entry::Occupied(mut entry) => {
                                if min_px - entry.get().anchor_px <= EVENT_CLUSTER_MERGE_PX {
                                    entry.get_mut().merge(member);
                                } else {
                                    let full = entry.insert(EventClusterAccum::new(member));
                                    full.emit(
                                        &gen_ref,
                                        &mut row_commands,
                                        &mut displayed_transactions,
                                    );
                                }
                            }
                            std::collections::hash_map::Entry::Vacant(entry) => {
                                entry.insert(EventClusterAccum::new(member));
                            }
                        }
                    } else {
                        // skip transactions that are rendered completely in the previous pixel
                        if (min_px == max_px) && (min_px == last_px) {
                            last_px = max_px;
                            continue;
                        }
                        last_px = max_px;

                        displayed_transactions.push(tx_ref.clone());
                        row_commands.insert(
                            tx_ref,
                            TxDrawingCommands {
                                min: Pos2::new(min_px, lane_min_y),
                                max: Pos2::new(max_px, lane_max_y),
                                kind: if start_time == end_time {
                                    TxDrawKind::EventMarker {
                                        out_of_range: false,
                                    }
                                } else {
                                    TxDrawKind::Rect
                                },
                                gen_ref: gen_ref.clone(),
                            },
                        );
                    }
                }

                for (_, accum) in clusters {
                    accum.emit(&gen_ref, &mut row_commands, &mut displayed_transactions);
                }
            }

            draw_commands.insert(tx_stream_ref.clone(), row_commands);
            stream_to_displayed_txs.insert(tx_stream_ref.clone(), displayed_transactions);
        }

        let focused_event_info = events_enabled
            .then(|| focused_tx_ref.and_then(|focused| container.event_info(focused.id)))
            .flatten();
        let parent_highlight_tx =
            focused_event_info.map(|info| TransactionRef { id: info.parent_tx });

        if let Some(focused_tx) = new_focused_tx {
            for rel in focused_tx
                .inc_relations
                .iter()
                .filter_map(|idx| container.get_relation(*idx))
            {
                // The parent_of link of an overlaid event reads as one unit
                // with its parent: the parent gets a co-highlight instead of
                // a relation arrow
                let is_overlaid_parent_link = focused_event_overlaid
                    && rel.name.as_ref() == crate::transaction_events::EVENT_PARENT_RELATION
                    && focused_event_info.is_some_and(|info| info.parent_tx == rel.source_tx_id);
                if is_overlaid_parent_link {
                    continue;
                }
                inc_relation_tx_ids.push(TransactionRef {
                    id: rel.source_tx_id,
                });
            }
            for rel in focused_tx
                .out_relations
                .iter()
                .filter_map(|idx| container.get_relation(*idx))
            {
                out_relation_tx_ids.push(TransactionRef { id: rel.sink_tx_id });
            }
            if old_focused_tx.is_none() || Some(focused_tx) != old_focused_tx.as_ref() {
                msgs.push(Message::FocusTransactionFromSource(
                    session_focused_tx_ref.clone(),
                    Some(focused_tx.clone()),
                ));
            }
        }

        Some(CachedTransactionDrawData {
            draw_commands,
            stream_to_displayed_txs,
            inc_relation_tx_ids,
            out_relation_tx_ids,
            parent_highlight_tx,
        })
    }

    // Transform from screen coordinates taking timeline into account if `consider_timeline` is true.
    pub fn transform_pos(
        &self,
        to_screen: RectTransform,
        p: Pos2,
        default_timeline_height: f32,
        consider_timeline: bool,
    ) -> Pos2 {
        to_screen
            .inverse()
            .transform_pos(if consider_timeline && self.show_default_timeline() {
                Pos2 {
                    x: p.x,
                    y: p.y - default_timeline_height,
                }
            } else {
                p
            })
    }

    //Calculate the offset for annotations on the canvas.
    pub fn get_annotation_offset(&self, default_timeline_height: f32) -> f32 {
        let mut offset = 0.;
        if self.show_default_timeline() {
            offset += default_timeline_height + self.user.config.layout.waveforms_gap * 4.;
        }
        offset
    }

    pub fn draw_items(&mut self, ui: &mut Ui, msgs: &mut Vec<Message>, viewport_idx: usize) {
        let Some(waves) = &self.user.waves else {
            return;
        };

        let (response, mut painter) =
            ui.allocate_painter(ui.available_size(), Sense::click_and_drag());

        let frame_size = response.rect.size();
        let frame_height = frame_size.y;
        let frame_width = frame_size.x;

        if frame_width < 1. || frame_height < 1. {
            return;
        }

        let cfg = match waves.inner {
            DataContainer::Waves(_) => DrawConfig::new(
                Vec2::new(frame_width, frame_height),
                self.user.config.layout.waveforms_line_height,
                self.user.config.layout.waveforms_text_size,
            ),
            DataContainer::Transactions(_) => DrawConfig::new(
                Vec2::new(frame_width, frame_height),
                self.user.config.layout.transactions_line_height,
                self.user.config.layout.waveforms_text_size,
            ),
            DataContainer::Empty => return,
        };
        // the draw commands have been invalidated, recompute
        if self.draw_data.borrow()[viewport_idx].is_none()
            || Some(response.rect) != *self.last_canvas_rect.borrow()
        {
            self.generate_draw_commands(&cfg, msgs, viewport_idx);
            *self.last_canvas_rect.borrow_mut() = Some(response.rect);
        }

        let to_screen =
            RectTransform::from_to(Rect::from_min_size(Pos2::ZERO, frame_size), response.rect);
        let y_zero = to_screen.transform_pos(Pos2::ZERO).y;
        let default_timeline_height = cfg.text_size;
        let pointer_pos_global = ui.input(|i| i.pointer.interact_pos());
        let pointer_pos_mouse_gesture = pointer_pos_global
            .map(|p| self.transform_pos(to_screen, p, default_timeline_height, false));
        let num_timestamps = waves.safe_canvas_num_timestamps();

        if response.clicked_by(PointerButton::Primary)
            || response.clicked_by(PointerButton::Secondary)
            || response.drag_started()
        {
            msgs.push(Message::SetActiveViewport(viewport_idx));
        }

        if ui.ui_contains_pointer() {
            let pointer_pos = pointer_pos_global.unwrap();
            let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
            let mouse_ptr_pos = to_screen.inverse().transform_pos(pointer_pos);
            if scroll_delta != Vec2::ZERO {
                msgs.push(Message::CanvasScroll {
                    delta: scroll_delta,
                    viewport_idx,
                });
            }

            let zoom_delta = ui.input(egui::InputState::zoom_delta);
            if zoom_delta != 1. {
                let mouse_ptr = Some(waves.viewports[viewport_idx].as_time_bigint(
                    mouse_ptr_pos.x,
                    frame_width,
                    &num_timestamps,
                ));

                msgs.push(Message::CanvasZoom {
                    mouse_ptr,
                    delta: zoom_delta,
                    viewport_idx,
                });
            }
        }

        ui.input(|i| {
            // If we have a single touch, we'll interpret that as a pan
            let touch = i.any_touches() && i.multi_touch().is_none();
            let right_mouse = i.pointer.button_down(PointerButton::Secondary);
            if touch || right_mouse {
                msgs.push(Message::CanvasScroll {
                    delta: Vec2 {
                        x: i.pointer.delta().y,
                        y: i.pointer.delta().x,
                    },
                    viewport_idx,
                });
            }
        });

        let modifiers = ui.input(|i| i.modifiers);
        let do_measure = self.do_measure(&modifiers);
        let handle_cursor = !modifiers.command
            && ((response.dragged_by(PointerButton::Primary) && !do_measure)
                || response.clicked_by(PointerButton::Primary));
        let needs_pointer_pos_canvas = self.annotation_kind.is_none() || handle_cursor;
        let pointer_pos_canvas = if needs_pointer_pos_canvas {
            pointer_pos_global
                .map(|p| to_screen.inverse().transform_pos(p))
                .map(|p| self.canvas_pos_to_item_space(&response, waves, p))
        } else {
            None
        };

        // Handle cursor
        if handle_cursor
            && let Some(snap_point) =
                self.snap_to_edge(pointer_pos_canvas, waves, frame_width, viewport_idx)
        {
            msgs.push(Message::CursorSet(snap_point));
        }

        // Draw background
        painter.rect_filled(
            response.rect,
            CornerRadius::ZERO,
            self.user.config.theme.canvas_colors.background,
        );

        // Check for mouse gesture starting
        if response.drag_started_by(PointerButton::Middle)
            || modifiers.command && response.drag_started_by(PointerButton::Primary)
        {
            msgs.push(Message::SetMouseGestureDragStart(
                ui.input(|i| i.pointer.press_origin())
                    .map(|p| self.transform_pos(to_screen, p, default_timeline_height, false)),
                None,
            ));
        }
        let annotation_offset = self.get_annotation_offset(default_timeline_height);

        if self.annotation_kind.is_some() && response.drag_started_by(PointerButton::Primary) {
            let start = ui
                .input(|i| i.pointer.press_origin())
                .map(|p| self.transform_pos(to_screen, p, default_timeline_height, false));
            let time = waves.viewports[viewport_idx].as_time_bigint(
                start.unwrap().x,
                frame_width,
                &num_timestamps,
            );
            msgs.push(Message::SetMouseGestureDragStart(
                ui.input(|i| i.pointer.press_origin())
                    .map(|p| self.transform_pos(to_screen, p, default_timeline_height, false)),
                Some(time),
            ));
        }

        // Check for measure drag starting. Snap the start X to the nearest transition
        // using the same logic as when placing cursors, but keep the original Y.
        if do_measure && response.drag_started_by(PointerButton::Primary) {
            let press_origin_local = ui
                .input(|i| i.pointer.press_origin())
                .map(|p| self.transform_pos(to_screen, p, default_timeline_height, false));
            let press_origin_canvas =
                press_origin_local.map(|p| self.canvas_pos_to_item_space(&response, waves, p));

            let snapped_pos = if let (Some(start_pos), Some(start_pos_canvas)) =
                (press_origin_local, press_origin_canvas)
            {
                // Snap to nearest edge/time then convert back to pixel X
                if let Some(snap_time) =
                    self.snap_to_edge(Some(start_pos_canvas), waves, frame_width, viewport_idx)
                {
                    let x = waves.viewports[viewport_idx].pixel_from_time(
                        &snap_time,
                        frame_width,
                        &num_timestamps,
                    );
                    Some(Pos2 { x, y: start_pos.y })
                } else {
                    Some(start_pos)
                }
            } else {
                None
            };

            msgs.push(Message::SetMeasureDragStart(snapped_pos));
        }

        let mut ctx = DrawingContext {
            painter: &mut painter,
            cfg: &cfg,
            to_screen: &|x, y| to_screen.transform_pos(Pos2::new(x, y)),
            theme: &self.user.config.theme,
        };

        let sorted_drawing_infos = Self::sorted_drawing_infos(waves);

        // We draw in absolute coords, but the variable offset in the y
        // direction is also in absolute coordinates, so we need to
        // compensate for that
        for drawing_info in sorted_drawing_infos.iter().copied() {
            // Use vidx so all sub-fields of a compound share the same stripe index
            let background_color =
                self.get_background_color(waves, drawing_info.vidx(), drawing_info.vidx().0);

            self.draw_background(drawing_info, &ctx, background_color);
        }

        let ticks = self.get_ticks_for_viewport_idx(waves, viewport_idx, &cfg);
        self.draw_tick_lines(waves, &ticks, &mut ctx);

        #[cfg(feature = "performance_plot")]
        self.timing.borrow_mut().start("Wave drawing");

        match &self.draw_data.borrow()[viewport_idx] {
            Some(CachedDrawData::WaveDrawData(draw_data)) => {
                self.draw_wave_data(
                    waves,
                    draw_data,
                    viewport_idx,
                    &sorted_drawing_infos,
                    &mut ctx,
                );
            }
            Some(CachedDrawData::TransactionDrawData(draw_data)) => {
                self.draw_transaction_data(
                    waves,
                    draw_data,
                    None,
                    viewport_idx,
                    ui,
                    msgs,
                    &sorted_drawing_infos,
                    &mut ctx,
                );
            }
            Some(CachedDrawData::MixedDrawData(draw_data)) => {
                if let Some(wave) = &draw_data.wave {
                    self.draw_wave_data(waves, wave, viewport_idx, &sorted_drawing_infos, &mut ctx);
                }
                for (source, tx_data) in &draw_data.transactions {
                    self.draw_transaction_data(
                        waves,
                        tx_data,
                        Some(*source),
                        viewport_idx,
                        ui,
                        msgs,
                        &sorted_drawing_infos,
                        &mut ctx,
                    );
                }
            }
            None => {}
        }
        #[cfg(feature = "performance_plot")]
        self.timing.borrow_mut().end("Wave drawing");

        let viewport = &waves.viewports[viewport_idx];
        waves.draw_graphics(&mut ctx, viewport, &self.user.config.theme);

        //Draw cursor and allow measure if no annotation is currently being drawn
        if self.annotation_kind.is_none() {
            waves.draw_cursor(&self.user.config.theme, &mut ctx, viewport);

            self.draw_measure_widget(
                ui,
                waves,
                pointer_pos_canvas,
                pointer_pos_mouse_gesture,
                &response,
                msgs,
                &mut ctx,
                viewport_idx,
            );
        }

        waves.draw_markers(&self.user.config.theme, &mut ctx, viewport);

        self.draw_marker_boxes(waves, &mut ctx, viewport, y_zero);

        if self.show_default_timeline() {
            let rect = Rect {
                min: Pos2 { x: 0.0, y: y_zero },
                max: Pos2 {
                    x: response.rect.max.x,
                    y: y_zero + default_timeline_height,
                },
            };
            ctx.painter.rect_filled(
                rect,
                CornerRadius::ZERO,
                self.user.config.theme.canvas_colors.background,
            );
            self.draw_default_timeline(waves, &ctx, viewport_idx, frame_width, &cfg);
        }

        let time_formatter = TimeFormatter::new(
            &waves.inner.metadata().timescale,
            &self.user.wanted_timeunit,
            &self.get_time_format(),
        );

        self.draw_mouse_gesture_widget(
            ui,
            waves,
            pointer_pos_mouse_gesture,
            &response,
            msgs,
            &mut ctx,
            viewport_idx,
            annotation_offset,
        );

        waves.draw_annotations(
            ui,
            &waves.viewports[viewport_idx],
            viewport_idx,
            &mut ctx,
            &self.user.config.theme,
            msgs,
            annotation_offset,
            response.rect,
            to_screen,
            &time_formatter,
        );

        self.handle_canvas_context_menu(&response, waves, to_screen, &mut ctx, msgs, viewport_idx);
    }

    fn draw_wave_data(
        &self,
        waves: &WaveData,
        draw_data: &CachedWaveDrawData,
        viewport_idx: usize,
        sorted_drawing_infos: &[&ItemDrawingInfo],
        ctx: &mut DrawingContext,
    ) {
        let clock_edges = &draw_data.clock_edges;
        let draw_commands = &draw_data.draw_commands;
        let draw_clock_edges = clock_edges.has_edges();
        let draw_clock_rising_marker =
            draw_clock_edges && self.user.config.theme.clock_rising_marker;
        let ticks = &draw_data.ticks;

        if draw_clock_edges {
            draw_clock_edge_marks(clock_edges, ctx, &self.user.config);
        }
        let zero_y = (ctx.to_screen)(0., 0.).y;
        for (item_count, drawing_info) in sorted_drawing_infos.iter().copied().enumerate() {
            // We draw in absolute coords, but the variable offset in the y
            // direction is also in absolute coordinates, so we need to
            // compensate for that
            let y_offset = drawing_info.top() - zero_y;

            let displayed_item = waves
                .items_tree
                .get_visible(drawing_info.vidx())
                .and_then(|node| waves.displayed_items.get(&node.item_ref));
            let color = displayed_item
                .and_then(super::displayed_item::DisplayedItem::color)
                .and_then(|color| self.user.config.theme.get_color(color));

            match drawing_info {
                ItemDrawingInfo::Variable(variable_info) => {
                    let variable_source = match displayed_item {
                        Some(DisplayedItem::Variable(variable)) => Some(variable.source),
                        _ => None,
                    };
                    if let Some(commands) = draw_commands.get(&variable_info.displayed_field_ref) {
                        let height_scaling_factor = displayed_item.map_or(
                            1.0,
                            super::displayed_item::DisplayedItem::height_scaling_factor,
                        );
                        let y_offset = y_offset + self.user.config.layout.waveforms_gap;

                        let color = color.unwrap_or_else(|| {
                            if let Some(DisplayedItem::Variable(variable)) = displayed_item {
                                waves
                                    .waves_for_source(variable.source)
                                    .and_then(|w| w.variable_meta(&variable.variable_ref).ok())
                                    .and_then(|meta| {
                                        if meta.is_event() {
                                            Some(self.user.config.theme.variable_event)
                                        } else if meta.is_parameter() {
                                            Some(self.user.config.theme.variable_parameter)
                                        } else {
                                            None
                                        }
                                    })
                                    .unwrap_or(self.user.config.theme.variable_default)
                            } else {
                                self.user.config.theme.variable_default
                            }
                        });
                        match commands {
                            DrawingCommands::Digital(digital_commands) => {
                                match digital_commands.drawing_type {
                                    DigitalDrawingType::Bool | DigitalDrawingType::Clock => {
                                        let draw_clock = (digital_commands.drawing_type
                                            == DigitalDrawingType::Clock)
                                            && draw_clock_rising_marker;
                                        let draw_background = self.fill_high_values();
                                        for (old, new) in digital_commands
                                            .values
                                            .iter()
                                            .zip(digital_commands.values.iter().skip(1))
                                        {
                                            self.draw_bool_transition(
                                                (old, new),
                                                new.1.force_anti_alias,
                                                color,
                                                y_offset,
                                                height_scaling_factor,
                                                draw_clock,
                                                draw_background,
                                                ctx,
                                            );
                                        }
                                    }
                                    DigitalDrawingType::Event => {
                                        for event in &digital_commands.values {
                                            self.draw_event(
                                                event,
                                                color,
                                                y_offset,
                                                height_scaling_factor,
                                                ctx,
                                            );
                                        }
                                    }
                                    DigitalDrawingType::Vector => {
                                        // Get background color and determine best text color
                                        let background_color = self.get_background_color(
                                            waves,
                                            drawing_info.vidx(),
                                            item_count,
                                        );

                                        let text_color = self
                                            .user
                                            .config
                                            .theme
                                            .get_best_text_color(background_color);

                                        for (old, new) in digital_commands
                                            .values
                                            .iter()
                                            .zip(digital_commands.values.iter().skip(1))
                                        {
                                            self.draw_region(
                                                (old, new),
                                                color,
                                                y_offset,
                                                height_scaling_factor,
                                                ctx,
                                                text_color,
                                            );
                                        }
                                    }
                                }
                            }
                            DrawingCommands::Analog(analog_commands) => {
                                crate::analog_renderer::draw_analog(
                                    analog_commands,
                                    color,
                                    y_offset,
                                    height_scaling_factor,
                                    ctx,
                                );
                            }
                        }
                    }
                    if let Some(source) = variable_source
                        && let Some(tail_start) =
                            Self::source_tail_start_pixel(waves, source, viewport_idx, ctx.cfg)
                    {
                        self.draw_source_tail_overlay(tail_start, drawing_info, ctx);
                    }
                }
                ItemDrawingInfo::Divider(_) | ItemDrawingInfo::Group(_) => {
                    if !self.show_divider_text() {
                        continue;
                    }

                    let text_color = color.unwrap_or(
                        // Get background color and determine best text color
                        self.user
                            .config
                            .theme
                            .get_best_text_color(self.get_background_color(
                                waves,
                                drawing_info.vidx(),
                                item_count,
                            )),
                    );

                    let wave_y_offset = y_offset + self.user.config.layout.waveforms_gap;
                    waves.draw_divider_text(
                        Some(text_color),
                        displayed_item
                            .map(super::displayed_item::DisplayedItem::name)
                            .unwrap_or_default(),
                        ticks,
                        ctx,
                        wave_y_offset,
                        &self.user.config,
                    );
                }
                ItemDrawingInfo::Marker(_) => {}
                ItemDrawingInfo::TimeLine(_) => {
                    let text_color = color.unwrap_or(
                        // Get background color and determine best text color
                        self.user
                            .config
                            .theme
                            .get_best_text_color(self.get_background_color(
                                waves,
                                drawing_info.vidx(),
                                item_count,
                            )),
                    );
                    let wave_y_offset = y_offset + self.user.config.layout.waveforms_gap;
                    waves.draw_ticks(text_color, ticks, ctx, wave_y_offset, Align2::CENTER_TOP);
                }
                ItemDrawingInfo::Stream(_) => {}
                ItemDrawingInfo::Placeholder(_) => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_transaction_data(
        &self,
        waves: &WaveData,
        draw_data: &CachedTransactionDrawData,
        source_filter: Option<SourceId>,
        viewport_idx: usize,
        ui: &mut Ui,
        msgs: &mut Vec<Message>,
        sorted_drawing_infos: &[&ItemDrawingInfo],
        ctx: &mut DrawingContext,
    ) {
        let draw_commands = &draw_data.draw_commands;
        let stream_to_displayed_txs = &draw_data.stream_to_displayed_txs;
        let inc_relation_tx_ids = &draw_data.inc_relation_tx_ids;
        let out_relation_tx_ids = &draw_data.out_relation_tx_ids;
        let parent_highlight_tx = &draw_data.parent_highlight_tx;

        let mut inc_relation_starts = vec![];
        let mut out_relation_starts = vec![];
        let mut focused_transaction_start: Option<Pos2> = None;

        let ticks = self.get_ticks_for_viewport_idx(waves, viewport_idx, ctx.cfg);

        // Draws the surrounding border of the stream
        let border_stroke = Stroke::new(
            self.user.config.theme.linewidth,
            self.user.config.theme.foreground,
        );

        let zero_y = (ctx.to_screen)(0., 0.).y;
        for (item_count, drawing_info) in sorted_drawing_infos.iter().copied().enumerate() {
            let y_offset = drawing_info.top() - zero_y;

            let displayed_item = waves
                .items_tree
                .get_visible(drawing_info.vidx())
                .and_then(|node| waves.displayed_items.get(&node.item_ref));
            let color = displayed_item
                .and_then(super::displayed_item::DisplayedItem::color)
                .and_then(|color| self.user.config.theme.get_color(color));
            let tx_color = color.unwrap_or(self.user.config.theme.transaction_default);

            match drawing_info {
                ItemDrawingInfo::Stream(stream) => {
                    let Some(DisplayedItem::Stream(displayed_stream)) = displayed_item else {
                        continue;
                    };
                    if let Some(source_filter) = source_filter
                        && displayed_stream.source != source_filter
                    {
                        continue;
                    }
                    if let Some(tx_refs) =
                        stream_to_displayed_txs.get(&stream.transaction_stream_ref)
                    {
                        let row_commands = draw_commands.get(&stream.transaction_stream_ref);
                        for tx_ref in tx_refs {
                            if let Some(tx_draw_command) =
                                row_commands.and_then(|commands| commands.get(tx_ref))
                            {
                                let mut min = tx_draw_command.min;
                                let mut max = tx_draw_command.max;

                                min.x = min.x.max(0.);
                                max.x = max.x.min(ctx.cfg.canvas_size.x - 1.);

                                let min = (ctx.to_screen)(min.x, y_offset + min.y);
                                let max = (ctx.to_screen)(max.x, y_offset + max.y);

                                let start = Pos2::new(min.x, f32::midpoint(min.y, max.y));

                                let is_transaction_focused =
                                    waves.focused_transaction.0.as_ref().is_some_and(|focused| {
                                        focused.source == displayed_stream.source
                                            && &focused.inner == tx_ref
                                    }) || matches!(
                                        &tx_draw_command.kind,
                                        TxDrawKind::EventCluster {
                                            contains_focused: true,
                                            ..
                                        }
                                    );

                                if inc_relation_tx_ids.contains(tx_ref) {
                                    inc_relation_starts.push(start);
                                } else if out_relation_tx_ids.contains(tx_ref) {
                                    out_relation_starts.push(start);
                                } else if is_transaction_focused {
                                    focused_transaction_start = Some(start);
                                }

                                let transaction_rect = Rect { min, max };
                                // Skip transactions that were clamped entirely off-canvas
                                if max.x < min.x {
                                    continue;
                                }

                                // Expand the hit area of narrow transactions (e.g.
                                // zero-duration events) so they remain clickable
                                let hit_rect =
                                    if transaction_rect.width() < MIN_TRANSACTION_CLICK_WIDTH {
                                        Rect::from_center_size(
                                            transaction_rect.center(),
                                            Vec2::new(
                                                MIN_TRANSACTION_CLICK_WIDTH,
                                                transaction_rect.height(),
                                            ),
                                        )
                                    } else {
                                        transaction_rect
                                    };

                                let response = ui.allocate_rect(hit_rect, Sense::click());

                                // A color the user assigned to the stream row takes
                                // precedence over the event default
                                let is_event_kind =
                                    !matches!(&tx_draw_command.kind, TxDrawKind::Rect);
                                let base_color = if is_event_kind {
                                    color.unwrap_or(self.user.config.theme.transaction_event)
                                } else {
                                    tx_color
                                };

                                let tx_fill_color = if is_transaction_focused {
                                    // Complementary color for focused transaction
                                    Color32::from_rgb(
                                        255 - base_color.r(),
                                        255 - base_color.g(),
                                        255 - base_color.b(),
                                    )
                                } else {
                                    base_color
                                };

                                match &tx_draw_command.kind {
                                    TxDrawKind::Rect => {
                                        let response = handle_transaction_tooltip_for_source(
                                            response,
                                            waves,
                                            displayed_stream.source,
                                            &tx_draw_command.gen_ref,
                                            tx_ref,
                                        );
                                        if response.clicked() {
                                            msgs.push(Message::FocusTransactionFromSource(
                                                Some(SourceTransactionRef::new(
                                                    displayed_stream.source,
                                                    tx_ref.clone(),
                                                )),
                                                None,
                                            ));
                                        }

                                        if transaction_rect.width() > 1.0 {
                                            let stroke =
                                                Stroke::new(1.5, tx_fill_color.gamma_multiply(1.2));
                                            ctx.painter.rect(
                                                transaction_rect,
                                                CornerRadius::same(5),
                                                tx_fill_color,
                                                stroke,
                                                epaint::StrokeKind::Middle,
                                            );
                                        } else {
                                            let tx_fill_color = tx_fill_color.gamma_multiply(1.2);

                                            let stroke = Stroke::new(1.5, tx_fill_color);
                                            ctx.painter.rect(
                                                transaction_rect,
                                                CornerRadius::ZERO,
                                                tx_fill_color,
                                                stroke,
                                                epaint::StrokeKind::Middle,
                                            );
                                        }

                                        // Co-highlight the parent of a focused
                                        // event so the pair reads as one unit
                                        if parent_highlight_tx.as_ref() == Some(tx_ref) {
                                            ctx.painter.rect_stroke(
                                                transaction_rect.expand(1.5),
                                                CornerRadius::same(5),
                                                Stroke::new(
                                                    2.0,
                                                    self.user
                                                        .config
                                                        .theme
                                                        .transaction_parent_highlight,
                                                ),
                                                epaint::StrokeKind::Outside,
                                            );
                                        }
                                    }
                                    TxDrawKind::EventMarker { out_of_range } => {
                                        let response = handle_transaction_tooltip_for_source(
                                            response,
                                            waves,
                                            displayed_stream.source,
                                            &tx_draw_command.gen_ref,
                                            tx_ref,
                                        );
                                        if response.clicked() {
                                            msgs.push(Message::FocusTransactionFromSource(
                                                Some(SourceTransactionRef::new(
                                                    displayed_stream.source,
                                                    tx_ref.clone(),
                                                )),
                                                None,
                                            ));
                                        }

                                        let marker_color = if *out_of_range
                                            && !is_transaction_focused
                                        {
                                            self.user.config.theme.transaction_event_out_of_range
                                        } else {
                                            tx_fill_color
                                        };

                                        // Bracket along the bottom edge for
                                        // events with visible duration
                                        if transaction_rect.width() > EVENT_DURATION_BRACKET_MIN_PX
                                        {
                                            ctx.painter.hline(
                                                transaction_rect.min.x..=transaction_rect.max.x,
                                                transaction_rect.max.y,
                                                Stroke::new(2.0, marker_color),
                                            );
                                        }

                                        let marker_rect = Rect {
                                            min: transaction_rect.min,
                                            max: Pos2::new(
                                                transaction_rect.min.x,
                                                transaction_rect.max.y,
                                            ),
                                        };
                                        self.draw_transaction_event_marker(
                                            marker_rect,
                                            marker_color,
                                            *out_of_range,
                                            ctx,
                                        );
                                    }
                                    TxDrawKind::EventCluster {
                                        count,
                                        names,
                                        time_span,
                                        any_out_of_range,
                                        ..
                                    } => {
                                        let time_scale = waves
                                            .transactions_for_source(displayed_stream.source)
                                            .map(|t| t.inner.time_scale.to_string())
                                            .unwrap_or_default();
                                        let source_label = (waves.source_count() > 1)
                                            .then(|| {
                                                waves.source_label_for(displayed_stream.source)
                                            })
                                            .flatten();
                                        let response =
                                            crate::tooltips::handle_event_cluster_tooltip(
                                                response,
                                                *count,
                                                names,
                                                time_span,
                                                &time_scale,
                                                source_label.as_deref(),
                                            );
                                        if response.clicked() {
                                            // Zoom in until the cluster
                                            // resolves into individual markers
                                            let (start, end) = time_span;
                                            let span = end - start;
                                            let padding = span.clone().max(BigInt::from(1));
                                            msgs.push(Message::ZoomToRange {
                                                start: start - &padding,
                                                end: end + &padding,
                                                viewport_idx,
                                            });
                                        }

                                        let marker_color = if *any_out_of_range
                                            && !is_transaction_focused
                                        {
                                            self.user.config.theme.transaction_event_out_of_range
                                        } else {
                                            tx_fill_color
                                        };
                                        self.draw_transaction_event_cluster(
                                            transaction_rect,
                                            marker_color,
                                            *count,
                                            ctx,
                                        );
                                    }
                                }
                            }
                        }
                    }
                    if let Some(tail_start) = Self::source_tail_start_pixel(
                        waves,
                        displayed_stream.source,
                        viewport_idx,
                        ctx.cfg,
                    ) {
                        self.draw_source_tail_overlay(tail_start, drawing_info, ctx);
                    }
                    ctx.painter.hline(
                        0.0..=((ctx.to_screen)(ctx.cfg.canvas_size.x, 0.0).x),
                        drawing_info.bottom(),
                        border_stroke,
                    );
                }
                ItemDrawingInfo::TimeLine(_) => {
                    let text_color = color.unwrap_or(
                        // Get background color and determine best text color
                        self.user
                            .config
                            .theme
                            .get_best_text_color(self.get_background_color(
                                waves,
                                drawing_info.vidx(),
                                item_count,
                            )),
                    );
                    waves.draw_ticks(text_color, &ticks, ctx, y_offset, Align2::CENTER_TOP);
                }
                ItemDrawingInfo::Variable(_) => {}
                ItemDrawingInfo::Divider(_) => {}
                ItemDrawingInfo::Marker(_) => {}
                ItemDrawingInfo::Group(_) => {}
                ItemDrawingInfo::Placeholder(_) => {}
            }
        }

        // Draws the relations of the focused transaction
        if let Some(focused_pos) = focused_transaction_start {
            let path_stroke = PathStroke::from(&ctx.theme.relation_arrow.style);
            // let stroke = PathStroke::from({
            // color = self.user.config.theme.annotation_arrow.color
            // width = self.user.config.theme.annotation_arrow.width
            // });
            for start_pos in inc_relation_starts {
                self.draw_arrow(start_pos, focused_pos, ctx, &path_stroke);
            }

            for end_pos in out_relation_starts {
                self.draw_arrow(focused_pos, end_pos, ctx, &path_stroke);
            }
        }
    }

    fn draw_region(
        &self,
        ((old_x, prev_region), (new_x, _)): (&(f32, DrawnRegion), &(f32, DrawnRegion)),
        user_color: Color32,
        offset: f32,
        height_scaling_factor: f32,
        ctx: &mut DrawingContext,
        text_color: Color32,
    ) {
        if let Some(prev_result) = &prev_region.inner {
            let color = prev_result.kind.color(user_color, ctx.theme);
            let transition_width = (new_x - old_x).min(ctx.theme.vector_transition_width);

            let trace_coords =
                |x, y| (ctx.to_screen)(x, y * ctx.cfg.line_height * height_scaling_factor + offset);

            let points = vec![
                trace_coords(*old_x, 0.5),
                trace_coords(old_x + transition_width / 2., 0.0),
                trace_coords(new_x - transition_width / 2., 0.0),
                trace_coords(*new_x, 0.5),
                trace_coords(new_x - transition_width / 2., 1.0),
                trace_coords(old_x + transition_width / 2., 1.0),
                trace_coords(*old_x, 0.5),
            ];

            if self.user.config.theme.wide_opacity != 0.0 {
                // For performance, it might be nice to draw both the background and line with this
                // call, but using convex_polygon on our polygons create artefacts on thin transitions.
                ctx.painter.add(PathShape::convex_polygon(
                    points.clone(),
                    color.gamma_multiply(self.user.config.theme.wide_opacity),
                    PathStroke::NONE,
                ));
            }
            match prev_region.trace_value {
                TraceValue::Normal => {
                    let stroke = Stroke {
                        color,
                        width: self.user.config.theme.linewidth,
                    };

                    ctx.painter.add(PathShape::line(points, stroke));
                }
                TraceValue::AllOnes => {
                    let stroke_thick = Stroke {
                        color,
                        width: self.user.config.theme.thick_linewidth,
                    };
                    let stroke = Stroke {
                        color,
                        width: self.user.config.theme.linewidth,
                    };
                    ctx.painter
                        .add(PathShape::line(points[0..4].to_vec(), stroke_thick));
                    ctx.painter
                        .add(PathShape::line(points[3..7].to_vec(), stroke));
                }
                TraceValue::AllZeros => {
                    let stroke_thick = Stroke {
                        color,
                        width: self.user.config.theme.linewidth,
                    };
                    ctx.painter
                        .add(PathShape::line(points[3..7].to_vec(), stroke_thick));
                }
                TraceValue::AllZerosThick => {
                    let stroke_thick = Stroke {
                        color,
                        width: self.user.config.theme.thick_linewidth,
                    };
                    ctx.painter
                        .add(PathShape::line(points[3..7].to_vec(), stroke_thick));
                }
            }

            let text_size = ctx.cfg.text_size;
            let char_width = text_size * (20. / 31.);

            let text_area = (new_x - old_x) - transition_width;
            let num_chars = (text_area / char_width).floor() as usize;
            let fits_text = num_chars >= 1;

            if fits_text {
                let content = if prev_result.value.len() > num_chars {
                    prev_result
                        .value
                        .chars()
                        .take(num_chars - 1)
                        .chain(['…'])
                        .collect::<String>()
                } else {
                    prev_result.value.clone()
                };

                ctx.painter.text(
                    trace_coords(*old_x + transition_width, 0.5),
                    Align2::LEFT_CENTER,
                    content,
                    FontId::monospace(text_size),
                    text_color,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_bool_transition(
        &self,
        ((old_x, prev_region), (new_x, new_region)): (&(f32, DrawnRegion), &(f32, DrawnRegion)),
        force_anti_alias: bool,
        color: Color32,
        offset: f32,
        height_scaling_factor: f32,
        draw_clock_marker: bool,
        draw_background: bool,
        ctx: &mut DrawingContext,
    ) {
        if let (Some(prev_result), Some(new_result)) = (&prev_region.inner, &new_region.inner) {
            let trace_coords =
                |x, y| (ctx.to_screen)(x, y * ctx.cfg.line_height * height_scaling_factor + offset);

            let (old_height, old_color, old_bg) = prev_result.value.bool_drawing_spec(
                color,
                &self.user.config.theme,
                prev_result.kind,
            );
            let (new_height, _, _) =
                new_result
                    .value
                    .bool_drawing_spec(color, &self.user.config.theme, new_result.kind);

            if let (Some(old_bg), true) = (old_bg, draw_background) {
                ctx.painter.add(RectShape::new(
                    Rect {
                        min: (ctx.to_screen)(*old_x, offset),
                        max: (ctx.to_screen)(
                            *new_x,
                            offset
                                + ctx.cfg.line_height * height_scaling_factor
                                + ctx.theme.linewidth / 2.,
                        ),
                    },
                    CornerRadius::ZERO,
                    old_bg,
                    Stroke::NONE,
                    epaint::StrokeKind::Middle,
                ));
            }

            let stroke = Stroke {
                color: old_color,
                width: self.user.config.theme.linewidth,
            };

            if force_anti_alias {
                ctx.painter.add(PathShape::line(
                    vec![trace_coords(*new_x, 0.0), trace_coords(*new_x, 1.0)],
                    stroke,
                ));
            }

            ctx.painter.add(PathShape::line(
                vec![
                    trace_coords(*old_x, 1. - old_height),
                    trace_coords(*new_x, 1. - old_height),
                    trace_coords(*new_x, 1. - new_height),
                ],
                stroke,
            ));

            if draw_clock_marker && (old_height < new_height) {
                ctx.painter.add(PathShape::convex_polygon(
                    vec![
                        trace_coords(*new_x - 2.5, 0.6),
                        trace_coords(*new_x, 0.4),
                        trace_coords(*new_x + 2.5, 0.6),
                    ],
                    old_color,
                    stroke,
                ));
            }
        }
    }

    fn draw_event(
        &self,
        (x, prev_region): &(f32, DrawnRegion),
        color: Color32,
        offset: f32,
        height_scaling_factor: f32,
        ctx: &mut DrawingContext,
    ) {
        if prev_region.inner.is_some() {
            let trace_coords =
                |x, y| (ctx.to_screen)(x, y * ctx.cfg.line_height * height_scaling_factor + offset);

            let stroke = Stroke {
                color,
                width: self.user.config.theme.linewidth,
            };

            // Draw both at old_x and new_x lines until the drawing commands are reworked to deal with this as a special case
            // Otherwise, not drawing the old_x (new_x) value will cause the first (last) event to not be drawn
            let top = trace_coords(*x, 0.0);
            ctx.painter
                .add(PathShape::line(vec![top, trace_coords(*x, 1.0)], stroke));

            ctx.painter.add(PathShape::convex_polygon(
                vec![
                    trace_coords(*x - 2.5, 0.2),
                    top,
                    trace_coords(*x + 2.5, 0.2),
                ],
                color,
                stroke,
            ));
        }
    }

    /// Draws a zero-duration transaction (event) as a diamond (milestone
    /// marker) centered on the event time. Out-of-range events draw hollow:
    /// background fill with a warning-tinted outline.
    fn draw_transaction_event_marker(
        &self,
        rect: Rect,
        color: Color32,
        hollow: bool,
        ctx: &DrawingContext,
    ) {
        let center = rect.center();
        let half_height = 0.5 * rect.height();
        let half_width = (0.6 * half_height).min(5.0);

        let points = vec![
            Pos2::new(center.x, center.y - half_height),
            Pos2::new(center.x + half_width, center.y),
            Pos2::new(center.x, center.y + half_height),
            Pos2::new(center.x - half_width, center.y),
        ];

        if hollow {
            ctx.painter.add(PathShape::convex_polygon(
                points,
                ctx.theme.canvas_colors.background,
                Stroke::new(1.5, color),
            ));
        } else {
            // An outline in the background color keeps the marker visible when
            // it overlaps same-colored transaction rectangles in stream view
            let stroke = Stroke::new(1.0, ctx.theme.canvas_colors.background);
            ctx.painter
                .add(PathShape::convex_polygon(points, color, stroke));
        }
    }

    /// Draws an aggregated event cluster: a slightly larger diamond at the
    /// cluster's first event plus a count badge.
    fn draw_transaction_event_cluster(
        &self,
        rect: Rect,
        color: Color32,
        count: usize,
        ctx: &DrawingContext,
    ) {
        let center = Pos2::new(rect.min.x, rect.center().y);
        let half_height = 0.5 * rect.height();
        let half_width = (0.8 * half_height).min(7.0);

        let stroke = Stroke::new(1.0, ctx.theme.canvas_colors.background);
        ctx.painter.add(PathShape::convex_polygon(
            vec![
                Pos2::new(center.x, center.y - half_height),
                Pos2::new(center.x + half_width, center.y),
                Pos2::new(center.x, center.y + half_height),
                Pos2::new(center.x - half_width, center.y),
            ],
            color,
            stroke,
        ));

        // The count badge sits inside the diamond so neighboring clusters
        // cannot overdraw it
        ctx.painter.text(
            center,
            Align2::CENTER_CENTER,
            count.to_string(),
            FontId::proportional(0.7 * ctx.cfg.text_size),
            ctx.theme.transaction_event_cluster,
        );
    }

    /// Draws a curvy arrow from `start` to `end`.
    fn draw_arrow(&self, start: Pos2, end: Pos2, ctx: &DrawingContext, stroke: &PathStroke) {
        let x_diff = (end.x - start.x).max(100.);
        let scaled_x_diff = 0.4 * x_diff;

        let anchor1 = Pos2 {
            x: start.x + scaled_x_diff,
            y: start.y,
        };
        let anchor2 = Pos2 {
            x: end.x - scaled_x_diff,
            y: end.y,
        };

        ctx.painter.add(Shape::CubicBezier(CubicBezierShape {
            points: [start, anchor1, anchor2, end],
            closed: false,
            fill: Default::default(),
            stroke: stroke.clone(),
        }));

        self.draw_arrowheads(anchor2, end, ctx, stroke);
    }

    /// Draws arrowheads for the vector going from `vec_start` to `vec_tip`.
    /// The `angle` has to be in degrees.
    fn draw_arrowheads(
        &self,
        vec_start: Pos2,
        vec_tip: Pos2,
        ctx: &DrawingContext,
        stroke: &PathStroke,
    ) {
        let head_length = ctx.theme.relation_arrow.head_length;

        let vec_x = vec_tip.x - vec_start.x;
        let vec_y = vec_tip.y - vec_start.y;

        let alpha = (2. * PI / 360.) * ctx.theme.relation_arrow.head_angle;

        // calculate the points of the new vector, which forms an angle of the given degrees with the given vector
        let vec_angled_x = vec_x * alpha.cos() + vec_y * alpha.sin();
        let vec_angled_y = -vec_x * alpha.sin() + vec_y * alpha.cos();

        // scale the new vector to be head_length long
        let vec_angled_x = (1. / (vec_angled_y - vec_angled_x).abs()) * vec_angled_x * head_length;
        let vec_angled_y = (1. / (vec_angled_y - vec_angled_x).abs()) * vec_angled_y * head_length;

        let arrowhead_left_x = vec_tip.x - vec_angled_x;
        let arrowhead_left_y = vec_tip.y - vec_angled_y;

        let arrowhead_right_x = vec_tip.x + vec_angled_y;
        let arrowhead_right_y = vec_tip.y - vec_angled_x;

        ctx.painter.add(PathShape::line(
            vec![
                Pos2::new(arrowhead_right_x, arrowhead_right_y),
                vec_tip,
                Pos2::new(arrowhead_left_x, arrowhead_left_y),
            ],
            stroke.clone(),
        ));
    }

    fn handle_canvas_context_menu(
        &self,
        response: &Response,
        waves: &WaveData,
        to_screen: RectTransform,
        ctx: &mut DrawingContext,
        msgs: &mut Vec<Message>,
        viewport_idx: usize,
    ) {
        let frame_size = response.rect.size();
        response.context_menu(|ui| {
            let offset = f32::from(ui.spacing().menu_margin.left);
            let top_left = to_screen.inverse().transform_rect(ui.min_rect()).left_top()
                - Pos2 {
                    x: offset,
                    y: offset,
                };

            let snap_pos =
                self.snap_to_edge(Some(top_left.to_pos2()), waves, frame_size.x, viewport_idx);

            if let Some(time) = snap_pos {
                self.draw_line(&time, ctx, viewport_idx, waves);
                ui.menu_button("Set marker", |ui| {
                    for id in waves.markers.keys().sorted() {
                        ui.button(format!("{id}")).clicked().then(|| {
                            msgs.push(Message::SetMarker {
                                id: *id,
                                time: time.clone(),
                            });
                        });
                    }
                    // At the moment we only support 255 markers, and the cursor is the 255th
                    if waves.can_add_marker() {
                        ui.button("New").clicked().then(|| {
                            msgs.push(Message::AddMarker {
                                time,
                                name: None,
                                move_focus: true,
                            });
                        });
                    }
                });
            }
        });
    }

    /// Takes a pointer pos in the canvas and returns a position that is snapped to transitions
    /// if the cursor is close enough to any transition. If the cursor is on the canvas and no
    /// transitions are close enough for snapping, the raw point will be returned. If the cursor is
    /// off the canvas, `None` is returned
    pub fn snap_to_edge(
        &self,
        pointer_pos_canvas: Option<Pos2>,
        waves: &WaveData,
        frame_width: f32,
        viewport_idx: usize,
    ) -> Option<BigInt> {
        let pos = pointer_pos_canvas?;
        let viewport = &waves.viewports[viewport_idx];
        let num_timestamps = waves.safe_canvas_num_timestamps();
        let timestamp = viewport.as_time_bigint(pos.x, frame_width, &num_timestamps);
        if let Some(utimestamp) = timestamp.to_biguint()
            && let Some(item_ref) = waves.item_ref_at_canvas_y(pos.y)
            && let Some(DisplayedItem::Variable(variable)) = &waves.displayed_items.get(&item_ref)
            && let Ok(Some(res)) = waves
                .inner
                .as_waves()
                .unwrap()
                .query_variable(&variable.variable_ref, &utimestamp)
        {
            let prev_time = &res
                .current
                .and_then(|v| v.0.to_bigint())
                .unwrap_or(BigInt::ZERO);
            let next_time = &res
                .next
                .unwrap_or_default()
                .to_bigint()
                .unwrap_or(BigInt::ZERO);
            let prev = viewport.pixel_from_time(prev_time, frame_width, &num_timestamps);
            let next = viewport.pixel_from_time(next_time, frame_width, &num_timestamps);
            if (prev - pos.x).abs() < (next - pos.x).abs() {
                if (prev - pos.x).abs() <= self.user.config.snap_distance {
                    return Some(prev_time.clone());
                }
            } else if (next - pos.x).abs() <= self.user.config.snap_distance {
                return Some(next_time.clone());
            }
        }
        Some(timestamp)
    }

    /// Draw a vertical line at the given time position. Used for context menu.
    pub fn draw_line(
        &self,
        time: &BigInt,
        ctx: &mut DrawingContext,
        viewport_idx: usize,
        waves: &WaveData,
    ) {
        let x = waves.viewports[viewport_idx].pixel_from_time(
            time,
            ctx.cfg.canvas_size.x,
            &waves.safe_canvas_num_timestamps(),
        );

        draw_vertical_line(x, ctx, &self.user.config.theme.cursor);
    }
}

/// Draw a vertical line at the given x position with the specified stroke
pub fn draw_vertical_line(x: f32, ctx: &mut DrawingContext, stroke: impl Into<Stroke>) {
    ctx.painter.line_segment(
        [
            (ctx.to_screen)(x, 0.),
            (ctx.to_screen)(x, ctx.cfg.canvas_size.y),
        ],
        stroke,
    );
}

impl WaveData {}

trait VariableExt {
    fn bool_drawing_spec(
        &self,
        user_color: Color32,
        theme: &SurferTheme,
        value_kind: ValueKind,
    ) -> (f32, Color32, Option<Color32>);
}

impl VariableExt for String {
    /// Return the height and color with which to draw this value if it is a boolean
    fn bool_drawing_spec(
        &self,
        user_color: Color32,
        theme: &SurferTheme,
        value_kind: ValueKind,
    ) -> (f32, Color32, Option<Color32>) {
        let color = value_kind.color(user_color, theme);
        let (height, background) = match (value_kind, self) {
            (
                ValueKind::HighImp
                | ValueKind::Undef
                | ValueKind::DontCare
                | ValueKind::Warn
                | ValueKind::Error
                | ValueKind::Custom(_),
                _,
            ) => (0.5, None),
            (ValueKind::Weak, other) => {
                if other.to_lowercase() == "l" {
                    (0., None)
                } else {
                    (1., Some(color.gamma_multiply(theme.waveform_opacity)))
                }
            }
            (ValueKind::Normal, other) => {
                if other == "0" {
                    (0., None)
                } else {
                    (1., Some(color.gamma_multiply(theme.waveform_opacity)))
                }
            }
            (ValueKind::Event, _) => (1., Some(color.gamma_multiply(theme.waveform_opacity))),
        };
        (height, color, background)
    }
}
