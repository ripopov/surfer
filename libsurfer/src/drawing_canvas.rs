use crate::tiles::commands::DocumentCommand;
use ecolor::Color32;
use egui::{FontId, PointerButton, Response, Sense, Ui};
use emath::{Align2, Pos2, Rect, RectTransform, Vec2};
use epaint::{
    CornerRadius, CubicBezierShape, PathShape, PathStroke, RectShape, Rgba, Shape, Stroke,
};
use eyre::WrapErr as _;
use ftr_parser::types::{Transaction, TxGenerator};
use itertools::Itertools;
use num::bigint::{ToBigInt, ToBigUint};
use num::{BigInt, BigUint, ToPrimitive};
use rayon::prelude::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use std::collections::HashMap;
use std::f32::consts::PI;
use surfer_translation_types::{
    NumericRange, SubFieldFlatTranslationResult, TranslatedValue, ValueKind, VariableInfo,
};
use tracing::{error, warn};

use crate::CachedDrawData::Transactions;
use crate::analog_renderer::{AnalogDrawingCommand, variable_analog_draw_commands};
use crate::clock_highlighting::draw_clock_edge_marks;
use crate::config::{FocusHighlight, SurferTheme};
use crate::data_container::DataContainer;
use crate::displayed_item::{
    AnalogSettings, DisplayedFieldRef, DisplayedItemRef, DisplayedVariable,
};
use crate::item_drawing_info::ItemDrawingInfo;
use crate::item_list::ItemList;
use crate::time::TimeFormatter;
use crate::tooltips::handle_transaction_tooltip;
use crate::trace_style::{TraceStyle, TraceValue};
use crate::transaction_container::{TransactionRef, TransactionStreamRef};
use crate::translation::{TranslationResultExt, TranslatorList, ValueKindExt, VariableInfoExt};
use crate::view::{DrawConfig, DrawingContext};
use crate::viewport::Viewport;
use crate::wave_container::{QueryResult, VariableRefExt};
use crate::wave_data::{TimeRange, WaveData};
use crate::{
    CachedCombinedDrawData, CachedDrawData, CachedTransactionDrawData, CachedWaveDrawData, Message,
    SystemState, displayed_item::DisplayedItem,
};

/// Immutable inputs for one canvas. The item list may be shared by other views,
/// while the viewport and transaction focus belong to the requesting view.
pub(crate) struct CanvasSource<'a> {
    pub tile_id: crate::tiles::TileId,
    pub interaction: &'a crate::tile_kinds::waveform::WaveformInteraction,
    pub document: &'a WaveData,
    pub items: &'a ItemList,
    pub viewport: &'a Viewport,
    pub focused_item: Option<crate::displayed_item_tree::VisibleItemIndex>,
    pub focused_transaction: &'a Option<TransactionRef>,
}

impl std::ops::Deref for CanvasSource<'_> {
    type Target = WaveData;
    fn deref(&self) -> &Self::Target {
        self.document
    }
}

/// Presentation inputs for a complete canvas pass. Persistent state is borrowed;
/// the caller supplies the disposable draw cache separately.
pub(crate) struct CanvasView<'a> {
    pub source: CanvasSource<'a>,
    pub scroll_offset: f32,
    pub selected_annotation: Option<egui::Id>,
    pub annotation_menu: Option<(Pos2, &'a BigInt)>,
}

impl<'a> CanvasView<'a> {
    pub(crate) fn new(
        document: &'a WaveData,
        items: &'a ItemList,
        view: &'a crate::tile_kinds::waveform::WaveformView,
        tile_id: crate::tiles::TileId,
    ) -> Self {
        Self {
            source: CanvasSource {
                tile_id,
                interaction: &view.interaction,
                document,
                items,
                viewport: &view.viewport,
                focused_item: view.focused_index(items),
                focused_transaction: &view.focused_transaction,
            },
            scroll_offset: view.scroll_offset,
            selected_annotation: view.selected_annotation,
            annotation_menu: view
                .annotation_menu
                .as_ref()
                .map(|(position, time)| (*position, time)),
        }
    }
}

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
/// List of values to draw for a variable.
///
/// It is an ordered list of values that should be drawn at the *start time*
/// until the *start time* of the next value.
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

pub struct TxDrawingCommands {
    min: Pos2,
    max: Pos2,
    gen_ref: TransactionStreamRef, // makes it easier to later access the actual Transaction object
}

pub(crate) struct VariableDrawCommands {
    pub(crate) draw_clock_edges: bool,
    pub(crate) clock_edges: Vec<f32>,
    pub(crate) display_id: DisplayedItemRef,
    pub(crate) local_commands: HashMap<Vec<String>, DrawingCommands>,
    pub(crate) local_msgs: Vec<Message>,
}

/// Immutable inputs needed by parallel waveform generation. No UI or list caches.
pub(crate) struct WaveRenderData<'a> {
    pub container: &'a crate::wave_container::WaveContainer,
    pub viewport: &'a crate::viewport::Viewport,
    pub range: &'a crate::wave_data::TimeRange,
    pub generation: u64,
}

/// Common setup for variable draw commands: extracts metadata and determines rendering mode.
/// Routes to either analog or digital command generation.
#[allow(clippy::too_many_arguments)]
fn variable_draw_commands(
    displayed_variable: &DisplayedVariable,
    display_id: DisplayedItemRef,
    timestamps: &[(f32, num::BigUint)],
    source: &WaveRenderData<'_>,
    translators: &TranslatorList,
    view_width: f32,
    trace_style: TraceStyle,
) -> Option<VariableDrawCommands> {
    let wave_container = source.container;

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

    let translator = crate::wave_data::variable_translator(
        displayed_variable.get_format(&[]),
        &[],
        translators,
        || Ok(meta.clone()),
    );
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
            source,
            translator,
            view_width,
        )
    } else {
        variable_digital_draw_commands(
            displayed_variable,
            display_id,
            timestamps,
            source,
            translators,
            wave_container,
            &meta,
            translator,
            &info,
            view_width,
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
    source: &WaveRenderData<'_>,
    translators: &TranslatorList,
    wave_container: &crate::wave_container::WaveContainer,
    meta: &crate::wave_container::VariableMeta,
    translator: &crate::translation::DynTranslator,
    info: &VariableInfo,
    view_width: f32,
    trace_style: TraceStyle,
) -> Option<VariableDrawCommands> {
    let range = source.range;
    let mut clock_edges = vec![];
    let mut local_msgs = vec![];
    let displayed_field_ref: DisplayedFieldRef = display_id.into();

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
            })) => {
                source
                    .viewport
                    .pixel_from_time(&timestamp.to_bigint().unwrap(), view_width, range)
            }
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

/// One view's disposable commands and the geometry they were generated for.
#[derive(Default)]
pub(crate) struct WaveDrawCache {
    commands: Option<CachedDrawData>,
    rect: Option<Rect>,
    #[cfg(test)]
    pub(crate) builds: usize,
}

impl SystemState {
    pub fn invalidate_draw_commands(&self) {
        self.user.workspace.invalidate_all();
    }

    pub fn draw_items(&self, ui: &mut Ui, msgs: &mut Vec<Message>, tile_id: crate::tiles::TileId) {
        let Some(waves) = self.user.waveform_read_at(tile_id) else {
            return;
        };
        let view = CanvasView::new(waves.document, waves.items, waves.view, tile_id);
        self.waveform_services().draw_canvas(
            &view,
            &mut waves.view.draw_cache.borrow_mut(),
            ui,
            msgs,
            tile_id,
        );
    }
    pub(crate) fn draw_waveform_body(
        &self,
        ui: &mut Ui,
        msgs: &mut Vec<Message>,
        tile_id: crate::tiles::TileId,
        columns: crate::tile_kinds::waveform_body::WaveformColumns,
    ) {
        let Some(waves) = self.user.waveform_read_at(tile_id) else {
            ui.centered_and_justified(|ui| {
                ui.label("Open a waveform file to display signals.");
            });
            return;
        };
        let view = CanvasView::new(waves.document, waves.items, waves.view, tile_id);
        let response = self.waveform_services().draw_waveform_body(
            &view,
            &mut waves.view.draw_cache.borrow_mut(),
            columns,
            ui,
            msgs,
        );
        if (response.name_width.is_some() || response.value_width.is_some())
            && let Some(entry) = self.user.workspace.tiles().get(&tile_id)
            && let crate::tiles::kind::TileKind::Waveform(tile) = &entry.kind
        {
            msgs.push(Message::ToTile(
                tile_id,
                crate::tiles::kind::TileMessage::Waveform(
                    crate::tile_kinds::waveform::WaveformMessage::ColumnWidths {
                        names: response.name_width.unwrap_or(tile.name_column_width),
                        values: response.value_width.unwrap_or(tile.value_column_width),
                    },
                ),
            ));
        }
        if waves.view.viewport_height != response.height || response.scroll_offset.is_some() {
            msgs.push(Message::WaveformBodyMeasured {
                tile_id,
                height: response.height,
                scroll_offset: response.scroll_offset,
            });
        }
    }
}

/// Draw a vertical line at the given time with the specified stroke.
#[inline]
pub(crate) fn draw_vertical_line_at_time(
    time: &BigInt,
    ctx: &mut DrawingContext,
    stroke: impl Into<Stroke>,
    viewport: &Viewport,
    range: &TimeRange,
) {
    let x = viewport.pixel_from_time(time, ctx.cfg.canvas_size.x, range);
    ctx.painter.line_segment(
        [
            (ctx.to_screen)(x, 0.),
            (ctx.to_screen)(x, ctx.cfg.canvas_size.y),
        ],
        stroke,
    );
}

fn shift_brightness(color: Color32, delta: f32, background: Color32) -> Color32 {
    // Lighten the color on dark backgrounds (blend toward white),
    // darken it on light backgrounds (blend toward black).
    let bg_luminance = crate::config::get_luminance(background);
    let rgba = Rgba::from(color);
    let result = if bg_luminance < 0.5 {
        // Dark background: lighten
        rgba * (1.0 - delta) + Rgba::WHITE * delta
    } else {
        // Light background: darken
        rgba * (1.0 - delta) + Rgba::BLACK * delta
    };
    Color32::from(result)
}

pub(crate) fn apply_brightness_shift(
    color: Color32,
    brightness_shift: Option<f32>,
    background: Color32,
) -> Color32 {
    match brightness_shift {
        Some(delta) => shift_brightness(color, delta, background),
        None => color,
    }
}

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

impl crate::tile_kinds::waveform_services::WaveformReadServices<'_> {
    pub(crate) fn generate_draw_commands(
        &self,
        source: &CanvasSource<'_>,
        cfg: &DrawConfig,
        msgs: &mut Vec<Message>,
    ) -> Option<CachedDrawData> {
        #[cfg(feature = "performance_plot")]
        self.timing.borrow_mut().start("Generate draw commands");
        let result = match source.document.inner {
            DataContainer::Waves(_) => self.generate_wave_draw_commands(source, cfg, msgs),
            DataContainer::Transactions(_) => self.generate_transaction_draw_commands(source, cfg),
            DataContainer::Combined { .. } => {
                let Some(CachedDrawData::Waves(wave)) =
                    self.generate_wave_draw_commands(source, cfg, msgs)
                else {
                    return None;
                };
                let Some(CachedDrawData::Transactions(transaction)) =
                    self.generate_transaction_draw_commands(source, cfg)
                else {
                    return None;
                };
                Some(CachedDrawData::Combined(CachedCombinedDrawData {
                    wave,
                    transaction,
                }))
            }
            DataContainer::Empty => None,
        };
        #[cfg(feature = "performance_plot")]
        self.timing.borrow_mut().end("Generate draw commands");
        result
    }

    pub(crate) fn generate_wave_draw_commands(
        &self,
        source: &CanvasSource<'_>,
        cfg: &DrawConfig,
        msgs: &mut Vec<Message>,
    ) -> Option<CachedDrawData> {
        let waves = source.document;
        let items = source.items;
        let viewport = source.viewport;
        let mut draw_commands = HashMap::new();

        let max_timestamp = waves.safe_max_timestamp();
        let max_time = max_timestamp.to_f64().unwrap_or(f64::MAX);
        let mut clock_edges_by_clock = vec![];
        let range = waves.time_range();
        // Compute which timestamp to draw in each pixel. We'll draw from -extra_draw_width to
        // width + extra_draw_width in order to draw initial transitions outside the screen
        let timestamps = (-cfg.extra_draw_width..(cfg.canvas_size.x as i32 + cfg.extra_draw_width))
            .into_par_iter()
            .filter_map(|x| {
                let time = viewport
                    .as_absolute_time(f64::from(x), cfg.canvas_size.x, range)
                    .0;
                if time < 0. || time > max_time {
                    None
                } else {
                    Some((x as f32, time.to_biguint().unwrap_or_default()))
                }
            })
            .collect::<Vec<_>>();

        let trace_style = self.trace_style;
        let translators = &self.translators;
        let wave_source = WaveRenderData {
            container: waves.inner.as_waves()?,
            viewport,
            range,
            generation: waves.cache_generation,
        };
        let commands = items
            .items_tree
            .iter_visible()
            .map(|node| (node.item_ref, items.displayed_items.get(&node.item_ref)))
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
                    &wave_source,
                    translators,
                    cfg.canvas_size.x,
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

        let ticks = self.get_ticks_for_viewport(waves, viewport, cfg);

        Some(CachedDrawData::Waves(CachedWaveDrawData {
            draw_commands,
            clock_edges,
            ticks,
        }))
    }

    pub(crate) fn generate_transaction_draw_commands(
        &self,
        source: &CanvasSource<'_>,
        cfg: &DrawConfig,
    ) -> Option<CachedDrawData> {
        let waves = source.document;
        let items = source.items;
        let viewport = source.viewport;
        let mut draw_commands = HashMap::new();
        let mut stream_to_displayed_txs = HashMap::new();
        let mut inc_relation_tx_ids = vec![];
        let mut out_relation_tx_ids = vec![];

        let focused_tx_ref = source.focused_transaction;
        let mut new_focused_tx: Option<&Transaction> = None;

        let range = waves.time_range();

        let displayed_items = &items.displayed_items;
        let displayed_streams = items
            .items_tree
            .iter_visible()
            .map(|node| node.item_ref)
            .collect::<Vec<_>>()
            .par_iter()
            .map(|id| displayed_items.get(id))
            .filter_map(|item| match item {
                Some(DisplayedItem::Stream(stream_ref)) => Some(stream_ref),
                _ => None,
            })
            .collect::<Vec<_>>();

        let first_visible_timestamp = viewport
            .curr_left
            .absolute(range)
            .0
            .to_biguint()
            .unwrap_or(BigUint::ZERO);

        for displayed_stream in displayed_streams {
            let tx_stream_ref = &displayed_stream.transaction_stream_ref;

            let mut generators: Vec<&TxGenerator> = vec![];
            let mut displayed_transactions = vec![];

            if tx_stream_ref.is_stream() {
                let stream = waves
                    .inner
                    .as_transactions()
                    .unwrap()
                    .get_stream(tx_stream_ref.stream_id)
                    .unwrap();

                for gen_id in &stream.generators {
                    generators.push(
                        waves
                            .inner
                            .as_transactions()
                            .unwrap()
                            .get_generator(*gen_id)
                            .unwrap(),
                    );
                }
            } else {
                generators.push(
                    waves
                        .inner
                        .as_transactions()
                        .unwrap()
                        .get_generator(tx_stream_ref.gen_id.unwrap())
                        .unwrap(),
                );
            }

            for generator in generators {
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

                for tx in transactions {
                    let start_time = tx.get_start_time();
                    let end_time = tx.get_end_time();
                    let curr_tx_id = tx.get_tx_id();

                    // stop drawing after last visible transaction
                    if start_time.to_f64().unwrap() > viewport.curr_right.absolute(range).0 {
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
                        range,
                    );
                    let max_px = viewport.pixel_from_time(
                        &end_time.to_bigint().unwrap(),
                        cfg.canvas_size.x - 1.,
                        range,
                    );

                    // skip transactions that are rendered completely in the previous pixel
                    if (min_px == max_px) && (min_px == last_px) {
                        last_px = max_px;
                        continue;
                    }
                    last_px = max_px;

                    displayed_transactions.push(TransactionRef { id: curr_tx_id });
                    let min = Pos2::new(min_px, cfg.line_height * tx.row as f32 + 4.0);
                    let max = Pos2::new(max_px, cfg.line_height * (tx.row + 1) as f32 - 4.0);

                    let tx_ref = TransactionRef { id: curr_tx_id };
                    draw_commands.insert(
                        tx_ref,
                        TxDrawingCommands {
                            min,
                            max,
                            gen_ref: TransactionStreamRef::new_gen(
                                tx_stream_ref.stream_id,
                                generator.id,
                                generator.name.clone(),
                            ),
                        },
                    );
                }
            }
            stream_to_displayed_txs.insert(tx_stream_ref.clone(), displayed_transactions);
        }

        if let Some(focused_tx) = new_focused_tx {
            for rel in &focused_tx.inc_relations {
                inc_relation_tx_ids.push(TransactionRef {
                    id: rel.source_tx_id,
                });
            }
            for rel in &focused_tx.out_relations {
                out_relation_tx_ids.push(TransactionRef { id: rel.sink_tx_id });
            }
        }

        Some(Transactions(CachedTransactionDrawData {
            draw_commands,
            stream_to_displayed_txs,
            inc_relation_tx_ids,
            out_relation_tx_ids,
        }))
    }

    /// Calculate the offset reserved for the default timeline header, so the name/value
    /// columns and the canvas all start their rows at the same y.
    pub(crate) fn default_timeline_offset(&self) -> f32 {
        if self.show_default_timeline {
            self.config.layout.waveforms_text_size + self.config.layout.waveforms_gap * 4.
        } else {
            0.0
        }
    }

    pub(crate) fn draw_canvas(
        &self,
        view: &CanvasView<'_>,
        cache: &mut WaveDrawCache,
        ui: &mut Ui,
        msgs: &mut Vec<Message>,
        tile_id: crate::tiles::TileId,
    ) {
        let source = &view.source;
        let waves = source;
        self.ensure_drawing_infos_cached(source.items);
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
                self.config.layout.waveforms_line_height,
                self.config.layout.waveforms_text_size,
            ),
            DataContainer::Transactions(_) => DrawConfig::new(
                Vec2::new(frame_width, frame_height),
                self.config.layout.transactions_line_height,
                self.config.layout.waveforms_text_size,
            ),
            DataContainer::Combined { .. } => DrawConfig::new(
                Vec2::new(frame_width, frame_height),
                self.config.layout.waveforms_line_height,
                self.config.layout.waveforms_text_size,
            ),
            DataContainer::Empty => return,
        };
        if cache.commands.is_none() || Some(response.rect) != cache.rect {
            let commands = self.generate_draw_commands(source, &cfg, msgs);
            cache.commands = commands;
            cache.rect = Some(response.rect);
            #[cfg(test)]
            {
                cache.builds += 1;
            }
        }

        let to_screen =
            RectTransform::from_to(Rect::from_min_size(Pos2::ZERO, frame_size), response.rect);
        let y_zero = to_screen.transform_pos(Pos2::ZERO).y;
        let pointer_pos_global = ui.input(|i| i.pointer.interact_pos());
        let pointer_pos_mouse_gesture =
            pointer_pos_global.map(|p| to_screen.inverse().transform_pos(p));
        let range = waves.time_range();

        if ui.ui_contains_pointer() {
            let pointer_pos = pointer_pos_global.unwrap();
            let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
            let mouse_ptr_pos = to_screen.inverse().transform_pos(pointer_pos);
            if scroll_delta != Vec2::ZERO {
                msgs.push(Message::CanvasScroll {
                    delta: scroll_delta,
                    tile_id,
                });
            }

            let zoom_delta = ui.input(egui::InputState::zoom_delta);
            if zoom_delta != 1. {
                let mouse_ptr = Some(waves.viewport.as_time_bigint(
                    mouse_ptr_pos.x,
                    frame_width,
                    range,
                ));

                msgs.push(Message::CanvasZoom {
                    mouse_ptr,
                    delta: zoom_delta,
                    tile_id,
                });
            }
        }

        // Query input before inspecting the response; response helpers also access
        // egui state and must not run while an input lock is held.
        let (single_touch, pointer_delta) = ui.input(|i| {
            (
                i.any_touches() && i.multi_touch().is_none(),
                i.pointer.delta(),
            )
        });
        if (single_touch && response.dragged()) || response.dragged_by(PointerButton::Secondary) {
            msgs.push(Message::CanvasScroll {
                delta: Vec2 {
                    x: pointer_delta.y,
                    y: pointer_delta.x,
                },
                tile_id,
            });
        }

        let modifiers = ui.input(|i| i.modifiers);
        let do_measure = self.do_measure(&modifiers);
        let handle_cursor = !modifiers.command
            && ((response.dragged_by(PointerButton::Primary) && !do_measure)
                || response.clicked_by(PointerButton::Primary));
        let needs_pointer_pos_canvas =
            source.interaction.annotation_kind.is_none() || handle_cursor;
        let pointer_pos_canvas = if needs_pointer_pos_canvas {
            pointer_pos_global.map(|p| to_screen.inverse().transform_pos(p))
        } else {
            None
        };

        // Handle cursor
        if handle_cursor
            && let Some(snap_point) = self.snap_to_edge(pointer_pos_canvas, source, frame_width)
        {
            msgs.push(Message::ToDocument(DocumentCommand::CursorSet(snap_point)));
        }

        // Draw background
        painter.rect_filled(
            response.rect,
            CornerRadius::ZERO,
            self.config.theme.canvas_colors.background,
        );

        // Check for mouse gesture starting
        if response.drag_started_by(PointerButton::Middle)
            || modifiers.command && response.drag_started_by(PointerButton::Primary)
        {
            msgs.push(Message::SetMouseGestureDragStart(
                ui.input(|i| i.pointer.press_origin())
                    .map(|p| to_screen.inverse().transform_pos(p)),
                None,
                tile_id,
            ));
        }
        let timeline_offset = self.default_timeline_offset();

        if source.interaction.annotation_kind.is_some()
            && response.drag_started_by(PointerButton::Primary)
        {
            let start = ui
                .input(|i| i.pointer.press_origin())
                .map(|p| to_screen.inverse().transform_pos(p));
            let time = waves
                .viewport
                .as_time_bigint(start.unwrap().x, frame_width, range);
            msgs.push(Message::SetMouseGestureDragStart(
                start,
                Some(time),
                tile_id,
            ));
        }

        // Check for measure drag starting. Snap the start X to the nearest transition
        // using the same logic as when placing cursors, but keep the original Y.
        if do_measure && response.drag_started_by(PointerButton::Primary) {
            let press_origin_local = ui
                .input(|i| i.pointer.press_origin())
                .map(|p| to_screen.inverse().transform_pos(p));

            let snapped_pos = if let Some(start_pos) = press_origin_local {
                // Snap to nearest edge/time then convert back to pixel X
                if let Some(snap_time) = self.snap_to_edge(Some(start_pos), source, frame_width) {
                    let x = waves
                        .viewport
                        .pixel_from_time(&snap_time, frame_width, range);
                    Some(Pos2 { x, y: start_pos.y })
                } else {
                    Some(start_pos)
                }
            } else {
                None
            };

            msgs.push(Message::SetMeasureDragStart(snapped_pos, tile_id));
        }

        let mut ctx = DrawingContext {
            painter: &mut painter,
            cfg: &cfg,
            to_screen: &|x, y| to_screen.transform_pos(Pos2::new(x, y)),
            theme: &self.config.theme,
        };

        // `waves.items.drawing_infos` holds offset-free (canonical) positions; the canvas isn't
        // inside a `ScrollArea` like the name/value columns, so scrolling is applied here
        // explicitly instead.
        let row_offset = timeline_offset - view.scroll_offset;

        let background_offset = y_zero + row_offset;
        let visible_top = -row_offset;
        let visible_bottom = ctx.cfg.canvas_size.y - row_offset;
        for drawing_info in waves
            .items
            .visible_drawing_infos(visible_top, visible_bottom)
            .iter()
        {
            // Use vidx so all sub-fields of a compound share the same stripe index
            let background_color = self.get_background_color(
                waves.items,
                waves.focused_item,
                drawing_info.vidx(),
                drawing_info.vidx().0,
            );

            self.draw_background(drawing_info, background_offset, &ctx, background_color);
        }

        #[cfg(feature = "performance_plot")]
        self.timing.borrow_mut().start("Wave drawing");

        match &cache.commands {
            Some(CachedDrawData::Waves(draw_data)) => {
                self.draw_wave_data(source, draw_data, row_offset, &mut ctx);
            }
            Some(CachedDrawData::Transactions(draw_data)) => {
                self.draw_transaction_data(
                    source, draw_data, ui, msgs, row_offset, &mut ctx, tile_id,
                );
            }
            Some(CachedDrawData::Combined(draw_data)) => {
                self.draw_wave_data(source, &draw_data.wave, row_offset, &mut ctx);
                self.draw_transaction_data(
                    source,
                    &draw_data.transaction,
                    ui,
                    msgs,
                    row_offset,
                    &mut ctx,
                    tile_id,
                );
            }
            None => {}
        }
        #[cfg(feature = "performance_plot")]
        self.timing.borrow_mut().end("Wave drawing");

        let viewport = waves.viewport;
        waves
            .items
            .draw_graphics(&mut ctx, viewport, waves.time_range(), &self.config.theme);

        //Draw cursor and allow measure if no annotation is currently being drawn
        if source.interaction.annotation_kind.is_none() {
            waves.draw_cursor(&self.config.theme, &mut ctx, viewport);

            self.draw_measure_widget(
                ui,
                source,
                pointer_pos_canvas,
                pointer_pos_mouse_gesture,
                &response,
                msgs,
                &mut ctx,
            );
        }

        waves
            .items
            .draw_markers(waves.document, &self.config.theme, &mut ctx, waves.viewport);

        self.draw_marker_boxes(waves.document, waves.items, &mut ctx, viewport, row_offset);

        if self.show_default_timeline {
            let rect = Rect {
                min: Pos2 { x: 0.0, y: y_zero },
                max: Pos2 {
                    x: response.rect.max.x,
                    y: y_zero + timeline_offset,
                },
            };
            ctx.painter.rect_filled(
                rect,
                CornerRadius::ZERO,
                self.config.theme.canvas_colors.background,
            );
            self.draw_default_timeline(waves.document, &ctx, viewport);
        }

        let time_formatter = TimeFormatter::new(
            &waves.inner.metadata().timescale,
            &self.wanted_timeunit,
            &self.time_format,
        );

        self.draw_mouse_gesture_widget(
            ui,
            source,
            pointer_pos_mouse_gesture,
            &response,
            msgs,
            &mut ctx,
            tile_id,
            timeline_offset,
        );

        crate::annotation::AnnotationView {
            document: waves.document,
            items: waves.items,
            viewport,
            selected_annotation: view.selected_annotation,
            menu_position: view.annotation_menu.map(|(position, _)| position),
            menu_time: view.annotation_menu.map(|(_, time)| time),
        }
        .draw_annotations(
            ui,
            tile_id,
            &mut ctx,
            &self.config.theme,
            msgs,
            timeline_offset,
            response.rect,
            to_screen,
            &time_formatter,
        );

        self.handle_canvas_context_menu(&response, source, to_screen, &mut ctx, msgs);
    }

    pub(crate) fn draw_wave_data(
        &self,
        source: &CanvasSource<'_>,
        draw_data: &CachedWaveDrawData,
        row_offset: f32,
        ctx: &mut DrawingContext,
    ) {
        let waves = source.document;
        let items = source.items;
        let clock_edges = &draw_data.clock_edges;
        let draw_commands = &draw_data.draw_commands;
        let draw_clock_edges = clock_edges.has_edges();
        let draw_clock_rising_marker = draw_clock_edges && self.config.theme.clock_rising_marker;
        let ticks = &draw_data.ticks;
        if !ticks.is_empty() && self.show_ticks {
            let stroke = Stroke::from(&self.config.theme.ticks.style);

            for (_, x, _) in ticks {
                ctx.draw_tick_line(*x, &stroke);
            }
        }

        if draw_clock_edges {
            draw_clock_edge_marks(clock_edges, ctx, self.config);
        }
        // Only the rows visible in the current scroll position need to be drawn; `top`/`bottom`
        // are derived from `items.layout_cache`, which are themselves computed purely from the
        // Surfer config layout constants (`waveforms_line_height`/`waveforms_gap`/etc.), not
        // egui defaults.
        let visible_top = -row_offset;
        let visible_bottom = ctx.cfg.canvas_size.y - row_offset;
        for (item_count, drawing_info) in items
            .visible_drawing_infos(visible_top, visible_bottom)
            .iter()
            .enumerate()
        {
            let y_offset = drawing_info.top_at(row_offset);

            let displayed_item = items
                .items_tree
                .get_visible(drawing_info.vidx())
                .and_then(|node| items.displayed_items.get(&node.item_ref));
            let color = displayed_item
                .and_then(super::displayed_item::DisplayedItem::color)
                .and_then(|color| self.config.theme.get_color(color));

            match drawing_info {
                ItemDrawingInfo::Variable(variable_info) => {
                    if let Some(commands) = draw_commands.get(&variable_info.displayed_field_ref) {
                        let height_scaling_factor = displayed_item.map_or(
                            1.0,
                            super::displayed_item::DisplayedItem::height_scaling_factor,
                        );
                        let y_offset = y_offset + self.config.layout.waveforms_gap;
                        let focus_highlight = if source.focused_item == Some(drawing_info.vidx()) {
                            self.focus_highlight
                        } else {
                            FocusHighlight::Off
                        };
                        let line_width = if matches!(
                            focus_highlight,
                            FocusHighlight::LineWidth | FocusHighlight::LineWidthAndBrightnessShift
                        ) {
                            self.config.theme.linewidth
                                * self.config.theme.focus_highlight_line_width_multiplier
                        } else {
                            self.config.theme.linewidth
                        };

                        let color = color.unwrap_or_else(|| {
                            if let Some(DisplayedItem::Variable(variable)) = displayed_item {
                                waves
                                    .inner
                                    .as_waves()
                                    .and_then(|w| w.variable_meta(&variable.variable_ref).ok())
                                    .and_then(|meta| {
                                        if meta.is_event() {
                                            Some(self.config.theme.variable_event)
                                        } else if meta.is_parameter() {
                                            Some(self.config.theme.variable_parameter)
                                        } else {
                                            None
                                        }
                                    })
                                    .unwrap_or(self.config.theme.variable_default)
                            } else {
                                self.config.theme.variable_default
                            }
                        });
                        let brightness_shift = if matches!(
                            focus_highlight,
                            FocusHighlight::BrightnessShift
                                | FocusHighlight::LineWidthAndBrightnessShift
                        ) {
                            Some(self.config.theme.focus_highlight_brightness_shift)
                        } else {
                            None
                        };
                        match commands {
                            DrawingCommands::Digital(digital_commands) => {
                                match digital_commands.drawing_type {
                                    DigitalDrawingType::Bool | DigitalDrawingType::Clock => {
                                        let draw_clock = (digital_commands.drawing_type
                                            == DigitalDrawingType::Clock)
                                            && draw_clock_rising_marker;
                                        let draw_background = self.fill_high_values;
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
                                                line_width,
                                                brightness_shift,
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
                                                line_width,
                                                brightness_shift,
                                                ctx,
                                            );
                                        }
                                    }
                                    DigitalDrawingType::Vector => {
                                        // Get background color and determine best text color
                                        let background_color = self.get_background_color(
                                            items,
                                            source.focused_item,
                                            drawing_info.vidx(),
                                            item_count,
                                        );

                                        let text_color =
                                            self.config.theme.get_best_text_color(background_color);

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
                                                line_width,
                                                brightness_shift,
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
                                    brightness_shift,
                                    ctx,
                                );
                            }
                        }
                    }
                }
                ItemDrawingInfo::Divider(_) | ItemDrawingInfo::Group(_) => {
                    if !self.show_divider_text {
                        continue;
                    }

                    let text_color = color.unwrap_or(
                        // Get background color and determine best text color
                        self.config
                            .theme
                            .get_best_text_color(self.get_background_color(
                                items,
                                source.focused_item,
                                drawing_info.vidx(),
                                item_count,
                            )),
                    );

                    let wave_y_offset = y_offset + self.config.layout.waveforms_gap;
                    ctx.draw_divider_text(
                        Some(text_color),
                        &displayed_item
                            .map(super::displayed_item::DisplayedItem::name)
                            .unwrap_or_default(),
                        ticks,
                        wave_y_offset,
                        self.config,
                    );
                }
                ItemDrawingInfo::Marker(_) => {}
                ItemDrawingInfo::TimeLine(_) => {
                    let text_color = color.unwrap_or(
                        // Get background color and determine best text color
                        self.config
                            .theme
                            .get_best_text_color(self.get_background_color(
                                items,
                                source.focused_item,
                                drawing_info.vidx(),
                                item_count,
                            )),
                    );
                    let wave_y_offset = y_offset + self.config.layout.waveforms_gap;
                    ctx.draw_ticks(text_color, ticks, wave_y_offset, Align2::CENTER_TOP);
                }
                ItemDrawingInfo::Stream(_) => {}
                ItemDrawingInfo::Placeholder(_) => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_transaction_data(
        &self,
        source: &CanvasSource<'_>,
        draw_data: &CachedTransactionDrawData,
        ui: &mut Ui,
        msgs: &mut Vec<Message>,
        row_offset: f32,
        ctx: &mut DrawingContext,
        tile_id: crate::tiles::TileId,
    ) {
        let waves = source.document;
        let items = source.items;
        let draw_commands = &draw_data.draw_commands;
        let stream_to_displayed_txs = &draw_data.stream_to_displayed_txs;
        let inc_relation_tx_ids = &draw_data.inc_relation_tx_ids;
        let out_relation_tx_ids = &draw_data.out_relation_tx_ids;

        let mut inc_relation_starts = vec![];
        let mut out_relation_starts = vec![];
        let mut focused_transaction_start: Option<Pos2> = None;

        let ticks = self.get_ticks_for_viewport(waves, source.viewport, ctx.cfg);

        if !ticks.is_empty() && self.show_ticks {
            let stroke = Stroke::from(&self.config.theme.ticks.style);

            for (_, x, _) in &ticks {
                ctx.draw_tick_line(*x, &stroke);
            }
        }

        // Draws the surrounding border of the stream
        let border_stroke = Stroke::new(self.config.theme.linewidth, self.config.theme.foreground);

        let visible_top = -row_offset;
        let visible_bottom = ctx.cfg.canvas_size.y - row_offset;
        // Loop over all items to enable drawing relations to non-visible transactions
        for (item_count, drawing_info) in items.layout_cache.borrow().infos.iter().enumerate() {
            let is_visible = drawing_info.overlaps(visible_top, visible_bottom);
            let y_offset = drawing_info.top_at(row_offset);

            let displayed_item = items
                .items_tree
                .get_visible(drawing_info.vidx())
                .and_then(|node| items.displayed_items.get(&node.item_ref));
            let color = displayed_item
                .and_then(super::displayed_item::DisplayedItem::color)
                .and_then(|color| self.config.theme.get_color(color));
            let tx_color = color.unwrap_or(self.config.theme.transaction_default);

            match drawing_info {
                ItemDrawingInfo::Stream(stream) => {
                    if let Some(tx_refs) =
                        stream_to_displayed_txs.get(&stream.transaction_stream_ref)
                    {
                        for tx_ref in tx_refs {
                            if let Some(tx_draw_command) = draw_commands.get(tx_ref) {
                                let mut min = tx_draw_command.min;
                                let mut max = tx_draw_command.max;

                                min.x = min.x.max(0.);
                                max.x = max.x.min(ctx.cfg.canvas_size.x - 1.);

                                let min = (ctx.to_screen)(min.x, y_offset + min.y);
                                let max = (ctx.to_screen)(max.x, y_offset + max.y);

                                let start = Pos2::new(min.x, f32::midpoint(min.y, max.y));

                                let is_transaction_focused = source
                                    .focused_transaction
                                    .as_ref()
                                    .is_some_and(|t| t == tx_ref);

                                if inc_relation_tx_ids.contains(tx_ref) {
                                    inc_relation_starts.push(start);
                                } else if out_relation_tx_ids.contains(tx_ref) {
                                    out_relation_starts.push(start);
                                } else if is_transaction_focused {
                                    focused_transaction_start = Some(start);
                                }

                                // Skip rendering if the transaction is not visible
                                if !is_visible {
                                    continue;
                                }

                                let transaction_rect = Rect { min, max };
                                if (max.x - min.x) > 1.0 {
                                    let mut response =
                                        ui.allocate_rect(transaction_rect, Sense::click());

                                    response = handle_transaction_tooltip(
                                        response,
                                        waves,
                                        &tx_draw_command.gen_ref,
                                        tx_ref,
                                    );

                                    if response.clicked() {
                                        msgs.push(Message::FocusTransaction(
                                            Some(tx_ref.clone()),
                                            tile_id,
                                        ));
                                    }

                                    let tx_fill_color = if is_transaction_focused {
                                        // Complementary color for focused transaction
                                        Color32::from_rgb(
                                            255 - tx_color.r(),
                                            255 - tx_color.g(),
                                            255 - tx_color.b(),
                                        )
                                    } else {
                                        tx_color
                                    };

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
                                    let tx_fill_color = tx_color.gamma_multiply(1.2);

                                    let stroke = Stroke::new(1.5, tx_fill_color);
                                    ctx.painter.rect(
                                        transaction_rect,
                                        CornerRadius::ZERO,
                                        tx_fill_color,
                                        stroke,
                                        epaint::StrokeKind::Middle,
                                    );
                                }
                            }
                        }
                        if is_visible {
                            ctx.painter.hline(
                                0.0..=((ctx.to_screen)(ctx.cfg.canvas_size.x, 0.0).x),
                                (ctx.to_screen)(0.0, drawing_info.bottom_at(row_offset)).y,
                                border_stroke,
                            );
                        }
                    }
                }
                ItemDrawingInfo::TimeLine(_) => {
                    if !is_visible {
                        continue;
                    }
                    let text_color = color.unwrap_or(
                        // Get background color and determine best text color
                        self.config
                            .theme
                            .get_best_text_color(self.get_background_color(
                                items,
                                source.focused_item,
                                drawing_info.vidx(),
                                item_count,
                            )),
                    );
                    ctx.draw_ticks(text_color, &ticks, y_offset, Align2::CENTER_TOP);
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
            // color = self.config.theme.annotation_arrow.color
            // width = self.config.theme.annotation_arrow.width
            // });
            for start_pos in inc_relation_starts {
                self.draw_arrow(start_pos, focused_pos, ctx, &path_stroke);
            }

            for end_pos in out_relation_starts {
                self.draw_arrow(focused_pos, end_pos, ctx, &path_stroke);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_region(
        &self,
        ((old_x, prev_region), (new_x, _)): (&(f32, DrawnRegion), &(f32, DrawnRegion)),
        user_color: Color32,
        offset: f32,
        height_scaling_factor: f32,
        ctx: &mut DrawingContext,
        text_color: Color32,
        line_width: f32,
        brightness_shift: Option<f32>,
    ) {
        if let Some(prev_result) = &prev_region.inner {
            let color = apply_brightness_shift(
                prev_result.kind.color(user_color, ctx.theme),
                brightness_shift,
                ctx.theme.canvas_colors.background,
            );
            let transition_width = (new_x - old_x).min(ctx.theme.vector_transition_width);

            let trace_coords =
                |x, y| (ctx.to_screen)(x, y * ctx.cfg.line_height * height_scaling_factor + offset);

            let points = vec![
                trace_coords(*old_x, 0.5),
                trace_coords(old_x + transition_width * 0.5, 0.0),
                trace_coords(new_x - transition_width * 0.5, 0.0),
                trace_coords(*new_x, 0.5),
                trace_coords(new_x - transition_width * 0.5, 1.0),
                trace_coords(old_x + transition_width * 0.5, 1.0),
                trace_coords(*old_x, 0.5),
            ];

            if self.draw_vector_unknowns_as_line
                && matches!(prev_result.kind, ValueKind::HighImp | ValueKind::Undef)
            {
                let stroke = Stroke {
                    color,
                    width: line_width,
                };
                ctx.painter.add(PathShape::line(
                    vec![trace_coords(*old_x, 0.5), trace_coords(*new_x, 0.5)],
                    stroke,
                ));
                return;
            }

            if self.config.theme.wide_opacity != 0.0 {
                // For performance, it might be nice to draw both the background and line with this
                // call, but using convex_polygon on our polygons create artefacts on thin transitions.
                ctx.painter.add(PathShape::convex_polygon(
                    points.clone(),
                    color.gamma_multiply(self.config.theme.wide_opacity),
                    PathStroke::NONE,
                ));
            }
            match prev_region.trace_value {
                TraceValue::Normal => {
                    let stroke = Stroke {
                        color,
                        width: line_width,
                    };

                    ctx.painter.add(PathShape::line(points, stroke));
                }
                TraceValue::AllOnes => {
                    let stroke_thick = Stroke {
                        color,
                        width: self.config.theme.thick_linewidth,
                    };
                    let stroke = Stroke {
                        color,
                        width: self.config.theme.linewidth,
                    };
                    ctx.painter
                        .add(PathShape::line(points[0..4].to_vec(), stroke_thick));
                    ctx.painter
                        .add(PathShape::line(points[3..7].to_vec(), stroke));
                }
                TraceValue::AllZeros => {
                    let stroke_thick = Stroke {
                        color,
                        width: self.config.theme.linewidth,
                    };
                    ctx.painter
                        .add(PathShape::line(points[3..7].to_vec(), stroke_thick));
                }
                TraceValue::AllZerosThick => {
                    let stroke_thick = Stroke {
                        color,
                        width: self.config.theme.thick_linewidth,
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
    pub(crate) fn draw_bool_transition(
        &self,
        ((old_x, prev_region), (new_x, new_region)): (&(f32, DrawnRegion), &(f32, DrawnRegion)),
        force_anti_alias: bool,
        color: Color32,
        offset: f32,
        height_scaling_factor: f32,
        draw_clock_marker: bool,
        draw_background: bool,
        line_width: f32,
        brightness_shift: Option<f32>,
        ctx: &mut DrawingContext,
    ) {
        if let (Some(prev_result), Some(new_result)) = (&prev_region.inner, &new_region.inner) {
            let trace_coords =
                |x, y| (ctx.to_screen)(x, y * ctx.cfg.line_height * height_scaling_factor + offset);

            let bg_color = ctx.theme.canvas_colors.background;
            let (old_height, old_color, old_bg) = {
                let (h, c, bg) = prev_result.value.bool_drawing_spec(
                    color,
                    &self.config.theme,
                    prev_result.kind,
                );
                (h, apply_brightness_shift(c, brightness_shift, bg_color), bg)
            };
            let (new_height, _, _) =
                new_result
                    .value
                    .bool_drawing_spec(color, &self.config.theme, new_result.kind);

            if let (Some(old_bg), true) = (old_bg, draw_background) {
                ctx.painter.add(RectShape::new(
                    Rect {
                        min: (ctx.to_screen)(*old_x, offset),
                        max: (ctx.to_screen)(
                            *new_x,
                            offset
                                + ctx.cfg.line_height * height_scaling_factor
                                + ctx.theme.linewidth * 0.5,
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
                width: line_width,
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

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_event(
        &self,
        (x, prev_region): &(f32, DrawnRegion),
        color: Color32,
        offset: f32,
        height_scaling_factor: f32,
        line_width: f32,
        brightness_shift: Option<f32>,
        ctx: &mut DrawingContext,
    ) {
        let color =
            apply_brightness_shift(color, brightness_shift, ctx.theme.canvas_colors.background);
        if prev_region.inner.is_some() {
            let trace_coords =
                |x, y| (ctx.to_screen)(x, y * ctx.cfg.line_height * height_scaling_factor + offset);

            let stroke = Stroke {
                color,
                width: line_width,
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

    /// Draws a curvy arrow from `start` to `end`.
    pub(crate) fn draw_arrow(
        &self,
        start: Pos2,
        end: Pos2,
        ctx: &DrawingContext,
        stroke: &PathStroke,
    ) {
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
    pub(crate) fn draw_arrowheads(
        &self,
        vec_start: Pos2,
        vec_tip: Pos2,
        ctx: &DrawingContext,
        stroke: &PathStroke,
    ) {
        let head_length = ctx.theme.relation_arrow.head_length;

        let vec_x = vec_tip.x - vec_start.x;
        let vec_y = vec_tip.y - vec_start.y;

        let alpha = (PI / 180.) * ctx.theme.relation_arrow.head_angle;

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

    pub(crate) fn handle_canvas_context_menu(
        &self,
        response: &Response,
        waves: &CanvasSource<'_>,
        to_screen: RectTransform,
        ctx: &mut DrawingContext,
        msgs: &mut Vec<Message>,
    ) {
        let frame_size = response.rect.size();
        response.context_menu(|ui| {
            let offset = f32::from(ui.spacing().menu_margin.left);
            let top_left = to_screen.inverse().transform_rect(ui.min_rect()).left_top()
                - Pos2 {
                    x: offset,
                    y: offset,
                };

            let snap_pos = self.snap_to_edge(Some(top_left.to_pos2()), waves, frame_size.x);

            if let Some(time) = snap_pos {
                draw_vertical_line_at_time(
                    &time,
                    ctx,
                    &self.config.theme.cursor,
                    waves.viewport,
                    waves.time_range(),
                );
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
    pub(crate) fn snap_to_edge(
        &self,
        pointer_pos_canvas: Option<Pos2>,
        waves: &CanvasSource<'_>,
        frame_width: f32,
    ) -> Option<BigInt> {
        let pos = pointer_pos_canvas?;
        let viewport = waves.viewport;
        let range = waves.time_range();
        let timestamp = viewport.as_time_bigint(pos.x, frame_width, range);
        if let Some(utimestamp) = timestamp.to_biguint()
            && let Some(item_ref) = waves.items.item_ref_at_canvas_y(pos.y)
            && let Some(DisplayedItem::Variable(variable)) =
                &waves.items.displayed_items.get(&item_ref)
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
            let prev = viewport.pixel_from_time(prev_time, frame_width, range);
            let next = viewport.pixel_from_time(next_time, frame_width, range);
            if (prev - pos.x).abs() < (next - pos.x).abs() {
                if (prev - pos.x).abs() <= self.config.snap_distance {
                    return Some(prev_time.clone());
                }
            } else if (next - pos.x).abs() <= self.config.snap_distance {
                return Some(next_time.clone());
            }
        }
        Some(timestamp)
    }
}

#[cfg(test)]
mod view_cache_tests {
    use super::*;
    use crate::{StartupParams, wave_source::WaveSource};

    fn tile_id(state: &SystemState, index: usize) -> crate::tiles::TileId {
        state.user.workspace.layout().tile_order()[index]
    }
    /// The legacy "add viewport" gesture: a linked split of the target waveform.
    fn add_viewport(state: &mut SystemState) {
        let command = state
            .user
            .workspace
            .split_command(
                crate::tiles::TileTarget::Focused,
                crate::tiles::layout::Direction::Right,
                false,
            )
            .expect("a waveform tile to split");
        state.update(Message::Workspace(command)).unwrap();
    }
    fn views(state: &SystemState) -> Vec<&crate::tile_kinds::waveform::WaveformView> {
        state
            .user
            .workspace
            .layout()
            .tile_order()
            .into_iter()
            .filter_map(|id| {
                state
                    .user
                    .workspace
                    .waveform_resources(id)
                    .map(|(_, view)| view)
            })
            .collect()
    }
    async fn settle(state: &mut SystemState) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !(state.waves_fully_loaded() && state.batch_commands_completed()) {
            assert!(
                std::time::Instant::now() < deadline,
                "wave load did not finish"
            );
            state.handle_async_messages();
            state.handle_batch_commands();
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    }

    fn render(ctx: &egui::Context, state: &SystemState, right_width: f32) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 500.0))),
                ..Default::default()
            },
            |ui| {
                for (index, rect) in [
                    Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 400.0)),
                    Rect::from_min_size(Pos2::new(320.0, 0.0), Vec2::new(right_width, 400.0)),
                ]
                .into_iter()
                .enumerate()
                {
                    let mut pane = ui.new_child(
                        egui::UiBuilder::new()
                            .id_salt(("view", index))
                            .max_rect(rect),
                    );
                    pane.set_clip_rect(rect);
                    state.draw_items(&mut pane, &mut Vec::new(), tile_id(state, index));
                }
            },
        );
        output.textures_delta.clear();
    }

    async fn loaded_counter() -> SystemState {
        let mut state = SystemState::new_default_config()
            .unwrap()
            .with_params(StartupParams {
                waves: Some(WaveSource::File(
                    project_root::get_project_root()
                        .unwrap()
                        .join("examples/counter.vcd")
                        .try_into()
                        .unwrap(),
                )),
                ..Default::default()
            });
        settle(&mut state).await;
        state.update(Message::AddVariables(vec![
            crate::wave_container::VariableRef::from_hierarchy_string("tb.dut.counter"),
        ]));
        settle(&mut state).await;
        state
    }

    #[tokio::test]
    async fn adding_variables_without_a_waveform_creates_one_atomic_history_entry() {
        use crate::tiles::{commands::WorkspaceCommand, kind::TileKind, layout::Placement};
        let mut state = loaded_counter().await;
        let first = tile_id(&state, 0);
        state.update(Message::Workspace(WorkspaceCommand::CloseTile(first)));
        state.update(Message::Workspace(WorkspaceCommand::CreateTile {
            kind: "logs".into(),
            placement: Placement::Root,
            focus: true,
        }));
        let logs = state.user.workspace.layout().focused().unwrap();
        state.undo_stack.clear();
        state.redo_stack.clear();
        let variable = crate::wave_container::VariableRef::from_hierarchy_string("tb.dut.counter");
        let invalid = crate::wave_container::VariableRef::from_hierarchy_string("tb.dut.missing");
        assert!(
            state
                .update(Message::AddVariables(vec![
                    variable.clone(),
                    invalid.clone()
                ]))
                .is_none()
        );
        assert_eq!(state.user.workspace.layout().tile_order(), [logs]);
        assert!(state.undo_stack.is_empty());
        assert!(state.user.workspace.item_lists().is_empty());

        state.update(Message::AddVariables(vec![variable.clone()]));
        settle(&mut state).await;
        let created = state.user.workspace.layout().focused().unwrap();
        assert_ne!(created, logs);
        let list = state.user.workspace.tiles()[&created]
            .kind
            .waveform_list()
            .unwrap();
        assert_eq!(
            state.user.workspace.item_lists()[&list]
                .displayed_items
                .len(),
            1
        );
        assert_eq!(state.undo_stack.len(), 1);
        assert_eq!(state.undo_stack[0].label(), "Add variables");
        assert!(matches!(
            &state.user.workspace.tiles()[&logs].kind,
            TileKind::Logs(_)
        ));
        state.update(Message::Undo(1));
        assert_eq!(state.user.workspace.layout().tile_order(), [logs]);
        assert!(state.user.workspace.item_lists().is_empty());
        assert_eq!(state.redo_stack.len(), 1);
        state.update(Message::AddVariables(vec![invalid]));
        state.update(Message::AddVariables(vec![]));
        assert_eq!(state.redo_stack.len(), 1);
        state.update(Message::Redo(1));
        assert!(state.user.workspace.tiles().contains_key(&created));
        assert_eq!(
            state.user.workspace.item_lists()[&list]
                .displayed_items
                .len(),
            1
        );

        // Existing-list edits do not roll back unrelated shared marker navigation.
        state.update(Message::Workspace(WorkspaceCommand::FocusTile(created)));
        state
            .user
            .waves
            .as_mut()
            .unwrap()
            .markers
            .insert(7, 10.into());
        state.update(Message::AddVariables(vec![variable.clone()]));
        state
            .user
            .waves
            .as_mut()
            .unwrap()
            .markers
            .insert(7, 20.into());
        state.update(Message::Undo(1));
        assert_eq!(
            state.user.waves.as_ref().unwrap().markers[&7],
            BigInt::from(20)
        );
        assert_eq!(
            state.user.workspace.item_lists()[&list]
                .displayed_items
                .len(),
            1
        );

        state.update(Message::Workspace(WorkspaceCommand::CloseTile(created)));
        state.update(Message::Workspace(WorkspaceCommand::CloseTile(logs)));
        state.undo_stack.clear();
        state.update(Message::AddVariables(vec![variable]));
        assert_eq!(state.user.workspace.tiles().len(), 1);
        assert_eq!(state.undo_stack.len(), 1);
        state.update(Message::Undo(1));
        assert!(state.user.workspace.tiles().is_empty());
        assert!(state.user.workspace.item_lists().is_empty());
    }

    #[tokio::test]
    async fn framebuffer_render_keeps_preferred_width_when_source_has_fewer_pixels() {
        use crate::tile_kinds::frame_buffer::FrameBufferMessage;
        use crate::tiles::kind::{TileKind, TileMessage};
        let mut state = loaded_counter().await;
        state.user.waves.as_mut().unwrap().cursor = Some(10.into());
        state
            .update(Message::SetFrameBufferVariable(
                crate::wave_container::VariableRef::from_hierarchy_string("tb.dut.counter"),
            ))
            .unwrap();
        settle(&mut state).await;
        let id = state.framebuffer_target().unwrap();
        state
            .update(Message::ToTile(
                id,
                TileMessage::FrameBuffer(FrameBufferMessage::Width(1024)),
            ))
            .unwrap();
        let TileKind::FrameBuffer(tile) = &state.user.workspace.tiles()[&id].kind else {
            panic!()
        };
        let original = tile.state.clone();
        let mut cache = None;
        let (bits, _, _) = crate::frame_buffer::read_frame_buffer(
            state.user.waves.as_ref(),
            &original.content,
            &mut cache,
        )
        .unwrap();
        assert!(!bits.is_empty());
        assert!(bits.len() < original.settings.pixels_per_row);
        let ctx = egui::Context::default();
        for _ in 0..2 {
            let mut messages = vec![];
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(640.0, 480.0))),
                    ..Default::default()
                },
                |ui| {
                    tile.ui(ui, id, state.user.waves.as_ref(), &mut messages);
                },
            );
            output.textures_delta.clear();
            assert!(
                messages.is_empty(),
                "rendering must not enqueue a settings edit"
            );
            assert_eq!(tile.state, original);
        }
    }

    #[tokio::test]
    async fn canvas_generation_uses_explicit_document_list_and_viewport() {
        let mut state = loaded_counter().await;
        let document = state.user.waves.take().unwrap();
        let id = tile_id(&state, 0);
        let (items, view) = state.user.workspace.waveform_resources(id).unwrap();
        let waves = crate::wave_data::WaveformRead {
            document: &document,
            items,
            view,
            tile_id: id,
        };
        let cfg = DrawConfig::new(Vec2::new(400.0, 300.0), 20.0, 12.0);
        let generate = |items: &ItemList, viewport: &Viewport| {
            let mut messages = Vec::new();
            let Some(CachedDrawData::Waves(data)) =
                state.waveform_services().generate_draw_commands(
                    &CanvasSource {
                        tile_id: crate::tiles::TileId(1),
                        interaction: &Default::default(),
                        document: waves.document,
                        items,
                        viewport,
                        focused_item: None,
                        focused_transaction: &None,
                    },
                    &cfg,
                    &mut messages,
                )
            else {
                panic!("expected wave draw commands");
            };
            data
        };
        // Generation must not consult the currently installed waveform or its caches.
        assert!(state.user.waves.is_none());
        let viewport = views(&state)[0].viewport;
        let populated = generate(waves.items, &viewport);
        assert!(!populated.draw_commands.is_empty());
        let empty = generate(&ItemList::default(), &viewport);
        assert!(empty.draw_commands.is_empty());
        assert_eq!(populated.ticks, empty.ticks);

        let mut zoomed = viewport;
        zoomed.curr_left = crate::viewport::Relative(0.25);
        zoomed.curr_right = crate::viewport::Relative(0.5);
        let linked = generate(waves.items, &zoomed);
        assert_eq!(linked.draw_commands.len(), populated.draw_commands.len());
        assert_ne!(linked.ticks, populated.ticks);
        assert_eq!(generate(waves.items, &viewport).ticks, populated.ticks);
    }

    #[tokio::test]
    async fn waveform_bodies_keep_columns_caches_and_drag_input_local() {
        let mut state = loaded_counter().await;
        let document = state.user.waves.take().unwrap();
        let id = tile_id(&state, 0);
        let (items, view) = state.user.workspace.waveform_resources(id).unwrap();
        let waves = crate::wave_data::WaveformRead {
            document: &document,
            items,
            view,
            tile_id: id,
        };
        let mut caches = [WaveDrawCache::default(), WaveDrawCache::default()];
        let ctx = egui::Context::default();
        let mut frame = |events: Vec<egui::Event>| {
            let mut messages = Vec::new();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 500.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    for (index, cache) in caches.iter_mut().enumerate() {
                        let rect = Rect::from_min_size(
                            Pos2::new(index as f32 * 400.0, 0.0),
                            Vec2::new(380.0, 400.0),
                        );
                        let mut pane =
                            ui.new_child(egui::UiBuilder::new().id_salt(index).max_rect(rect));
                        pane.set_clip_rect(rect);
                        state.waveform_services().draw_waveform_body(
                            &CanvasView {
                                source: CanvasSource {
                                    tile_id: crate::tiles::TileId(index as u64 + 1),
                                    interaction: &Default::default(),
                                    document: waves.document,
                                    items: waves.items,
                                    viewport: &views(&state)[0].viewport,
                                    focused_item: None,
                                    focused_transaction: &None,
                                },
                                scroll_offset: index as f32 * 20.0,
                                selected_annotation: None,
                                annotation_menu: None,
                            },
                            cache,
                            crate::tile_kinds::waveform_body::WaveformColumns {
                                focus_ids: false,
                                names: Some(100.0),
                                values: (index == 0).then_some(100.0),
                            },
                            &mut pane,
                            &mut messages,
                        );
                    }
                },
            );
            output.textures_delta.clear();
            fn duplicate_id_warning(shape: &egui::Shape) -> bool {
                match shape {
                    egui::Shape::Text(text) => {
                        text.galley.text().contains("First use of")
                            || text.galley.text().contains("Second use of")
                    }
                    egui::Shape::Vec(shapes) => shapes.iter().any(duplicate_id_warning),
                    _ => false,
                }
            }
            assert!(
                !output
                    .shapes
                    .iter()
                    .any(|shape| duplicate_id_warning(&shape.shape)),
                "waveform panels must have distinct egui IDs"
            );
            messages
        };
        frame(Vec::new());
        let start = Pos2::new(650.0, 150.0);
        frame(vec![
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton {
                pos: start,
                button: PointerButton::Secondary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
        ]);
        let messages = frame(vec![egui::Event::PointerMoved(
            start + Vec2::new(30.0, 20.0),
        )]);
        let targets: Vec<_> = messages
            .iter()
            .filter_map(|message| match message {
                Message::CanvasScroll { tile_id, .. } => Some(*tile_id),
                _ => None,
            })
            .collect();
        assert_eq!(
            targets,
            [crate::tiles::TileId(2)],
            "only the canvas owning the drag should pan"
        );
        assert!(state.user.waves.is_none());
        let first = caches[0].rect.unwrap();
        let second = caches[1].rect.unwrap();
        assert_eq!(first.left(), 200.0);
        assert_eq!(first.right(), 380.0);
        assert_eq!(second.left(), 500.0);
        assert_eq!(second.right(), 780.0);
        assert_eq!(caches.map(|cache| cache.builds), [1, 1]);
    }

    #[tokio::test]
    async fn gesture_commands_keep_their_origin_when_focus_changes() {
        let mut state = loaded_counter().await;
        add_viewport(&mut state);
        let origin = Pos2::new(40.0, 30.0);
        state.update(Message::SetMouseGestureDragStart(
            Some(origin),
            Some(BigInt::from(17)),
            tile_id(&state, 0),
        ));
        state.update(Message::SetMouseGestureAnnotation(
            Some(crate::mousegestures::AnnotationKind::Rectangle),
            tile_id(&state, 0),
        ));
        state.update(Message::SetMeasureDragStart(
            Some(origin),
            tile_id(&state, 1),
        ));
        state.update(Message::Workspace(
            crate::tiles::commands::WorkspaceCommand::FocusTile(tile_id(&state, 1)),
        ));
        state.update(Message::SetMouseGestureDragStart(
            None,
            None,
            tile_id(&state, 0),
        ));
        {
            let views = &views(&state);
            assert!(views[0].interaction.gesture_start_location.is_none());
            assert!(views[0].interaction.measure_start_location.is_none());
            assert!(views[0].interaction.annotation_kind.is_some());
            assert_eq!(views[1].interaction.measure_start_location, Some(origin));
            assert!(views[1].interaction.annotation_kind.is_none());
            assert!(views[0].clone().interaction.annotation_kind.is_none());
        }
        let view = &mut state
            .user
            .waveform_edit_at(tile_id(&state, 1))
            .unwrap()
            .view;
        view.scroll_offset = 80.0;
        view.reset_runtime();
        assert!(view.interaction.measure_start_location.is_none());
        assert_eq!(view.scroll_offset, 80.0);
    }

    #[tokio::test]
    async fn linked_views_keep_independent_row_focus_through_insert_and_undo() {
        use crate::displayed_item_tree::VisibleItemIndex;
        let mut state = loaded_counter().await;
        state.update(Message::AddDivider(Some("one".into()), None));
        state.update(Message::AddDivider(Some("two".into()), None));
        add_viewport(&mut state);
        state.update(Message::Workspace(
            crate::tiles::commands::WorkspaceCommand::FocusTile(tile_id(&state, 0)),
        ));
        state.update(Message::FocusItem(VisibleItemIndex(2)));
        let first_focus = views(&state)[0].focused_item;
        state.update(Message::Workspace(
            crate::tiles::commands::WorkspaceCommand::FocusTile(tile_id(&state, 1)),
        ));
        state.update(Message::FocusItem(VisibleItemIndex(0)));
        state.update(Message::AddDivider(
            Some("inserted".into()),
            Some(VisibleItemIndex(0)),
        ));
        {
            let waves = state.user.waveform_read().unwrap();
            assert_eq!(views(&state)[0].focused_item, first_focus);
            assert_ne!(
                views(&state)[0].focused_index(waves.items),
                views(&state)[1].focused_index(waves.items)
            );
        }
        state.update(Message::Undo(1));
        let waves = state.user.waveform_read().unwrap();
        assert_eq!(views(&state)[0].focused_item, first_focus);
        assert_eq!(
            views(&state)[0].focused_index(waves.items),
            Some(VisibleItemIndex(2))
        );
    }

    #[tokio::test]
    async fn linked_views_keep_row_identity_through_move_and_undo() {
        use crate::displayed_item_tree::VisibleItemIndex;
        let mut state = loaded_counter().await;
        state.update(Message::AddDivider(Some("one".into()), None));
        state.update(Message::AddDivider(Some("two".into()), None));
        state.update(Message::FocusItem(VisibleItemIndex(2)));
        add_viewport(&mut state);
        state.update(Message::Workspace(
            crate::tiles::commands::WorkspaceCommand::FocusTile(tile_id(&state, 1)),
        ));
        state.update(Message::FocusItem(VisibleItemIndex(0)));
        let identities = views(&state)
            .iter()
            .map(|view| view.focused_item)
            .collect::<Vec<_>>();
        state.update(Message::MoveFocusedItem(crate::MoveDir::Down, 2));
        {
            let waves = state.user.waveform_read().unwrap();
            assert_eq!(
                views(&state)
                    .iter()
                    .map(|view| view.focused_item)
                    .collect::<Vec<_>>(),
                identities
            );
            assert_eq!(
                views(&state)[0].focused_index(waves.items),
                Some(VisibleItemIndex(1))
            );
            assert_eq!(
                views(&state)[1].focused_index(waves.items),
                Some(VisibleItemIndex(2))
            );
        }
        state.update(Message::Undo(1));
        let waves = state.user.waveform_read().unwrap();
        assert_eq!(
            views(&state)
                .iter()
                .map(|view| view.focused_item)
                .collect::<Vec<_>>(),
            identities
        );
        assert_eq!(
            views(&state)[0].focused_index(waves.items),
            Some(VisibleItemIndex(2))
        );
        assert_eq!(
            views(&state)[1].focused_index(waves.items),
            Some(VisibleItemIndex(0))
        );
    }

    #[tokio::test]
    async fn row_drop_uses_captured_items_after_focus_and_selection_change() {
        use crate::displayed_item_tree::{ItemIndex, TargetPosition, VisibleItemIndex};
        let mut state = loaded_counter().await;
        state.update(Message::AddDivider(Some("one".into()), None));
        state.update(Message::AddDivider(Some("two".into()), None));
        add_viewport(&mut state);
        let captured = state
            .user
            .waveform_read()
            .unwrap()
            .items
            .items_tree
            .get_visible(VisibleItemIndex(0))
            .unwrap()
            .item_ref;
        state.update(Message::Workspace(
            crate::tiles::commands::WorkspaceCommand::FocusTile(tile_id(&state, 1)),
        ));
        state.update(Message::FocusItem(VisibleItemIndex(1)));
        state.update(Message::SetItemSelected(VisibleItemIndex(1), true));
        let focus = views(&state)[1].focused_item;
        state.update(Message::MoveDraggedItems {
            tile_id: crate::tiles::TileId(1),
            items: vec![captured],
            position: TargetPosition {
                before: ItemIndex(3),
                level: 0,
            },
        });
        let waves = state.user.waveform_read().unwrap();
        assert_eq!(
            waves.items.items_tree.iter().last().unwrap().item_ref,
            captured
        );
        assert_eq!(views(&state)[1].focused_item, focus);
        assert_eq!(
            state.user.workspace.layout().focused(),
            Some(tile_id(&state, 1))
        );
        state.update(Message::Undo(1));
        assert_eq!(
            state
                .user
                .waveform_read()
                .unwrap()
                .items
                .items_tree
                .iter()
                .next()
                .unwrap()
                .item_ref,
            captured
        );
    }

    #[tokio::test]
    async fn navigation_validates_before_mutation_and_invalidates_only_its_view() {
        let mut state = loaded_counter().await;
        add_viewport(&mut state);
        let ctx = egui::Context::default();
        render(&ctx, &state, 300.0);
        let before = views(&state)[0].viewport;
        for command in [
            Message::CanvasZoom {
                delta: f32::NAN,
                mouse_ptr: None,
                tile_id: crate::tiles::TileId(1),
            },
            Message::CanvasZoom {
                delta: 0.0,
                mouse_ptr: None,
                tile_id: crate::tiles::TileId(1),
            },
            Message::ZoomToRange {
                start: 100.into(),
                end: 50.into(),
                tile_id: crate::tiles::TileId(1),
            },
            Message::GoToTime(Some(BigInt::from(1u8) << 4096), tile_id(&state, 0)),
            Message::GoToStart {
                tile_id: crate::tiles::TileId(u64::MAX),
            },
        ] {
            assert!(state.update(command).is_none());
            assert_eq!(views(&state)[0].viewport, before);
            assert!(
                views(&state)
                    .iter()
                    .all(|view| view.draw_cache.borrow().commands.is_some())
            );
        }
        state.update(Message::Workspace(
            crate::tiles::commands::WorkspaceCommand::FocusTile(tile_id(&state, 1)),
        ));
        // Switching input focus may invalidate the old renderer globally; repopulate both.
        render(&ctx, &state, 300.0);
        state.update(Message::CanvasZoom {
            delta: 0.5,
            mouse_ptr: None,
            tile_id: crate::tiles::TileId(1),
        });
        assert_ne!(views(&state)[0].viewport, before);
        assert!(views(&state)[0].draw_cache.borrow().commands.is_none());
        assert!(views(&state)[1].draw_cache.borrow().commands.is_some());
        assert_eq!(
            state.user.workspace.layout().focused(),
            Some(tile_id(&state, 1))
        );
    }

    #[tokio::test]
    async fn deleting_marker_rows_preserves_shared_time_and_explicit_deletion_is_undoable() {
        let mut state = loaded_counter().await;
        state.update(Message::AddMarker {
            time: 42.into(),
            name: Some("shared".into()),
            move_focus: false,
        });
        let (row, marker) = state
            .user
            .waveform_read()
            .unwrap()
            .items
            .displayed_items
            .iter()
            .find_map(|(id, item)| match item {
                DisplayedItem::Marker(marker) => Some((*id, marker.idx)),
                _ => None,
            })
            .unwrap();
        state.update(Message::RemoveItems(vec![row]));
        let waves = state.user.waveform_read().unwrap();
        assert_eq!(waves.document.markers.get(&marker), Some(&BigInt::from(42)));
        assert!(!waves.items.displayed_items.contains_key(&row));
        state.update(Message::Undo(1));
        assert!(
            state
                .user
                .waveform_read()
                .unwrap()
                .items
                .displayed_items
                .contains_key(&row)
        );
        state.update(Message::RemoveItems(vec![row]));
        state.update(Message::RemoveMarker(marker));
        assert!(
            !state
                .user
                .waveform_read()
                .unwrap()
                .document
                .markers
                .contains_key(&marker)
        );
        state.update(Message::Undo(1));
        let waves = state.user.waveform_read().unwrap();
        assert_eq!(waves.document.markers.get(&marker), Some(&BigInt::from(42)));
        assert!(!waves.items.displayed_items.contains_key(&row));
    }

    #[tokio::test]
    async fn invalid_insertions_preserve_content_identity_and_redo() {
        use crate::displayed_item_tree::VisibleItemIndex;
        let mut state = loaded_counter().await;
        state.update(Message::AddDivider(Some("valid".into()), None));
        state.update(Message::Undo(1));
        let undo = state.undo_stack.len();
        let redo = state.redo_stack.len();
        let before = {
            let waves = state.user.waveform_read().unwrap();
            ron::to_string(&crate::tiles::serde::ItemListFile::from(waves.items)).unwrap()
        };
        for message in [
            Message::AddDivider(None, Some(VisibleItemIndex(999))),
            Message::AddTimeLine(Some(VisibleItemIndex(999))),
        ] {
            assert!(state.update(message).is_none());
            assert_eq!(state.undo_stack.len(), undo);
            assert_eq!(state.redo_stack.len(), redo);
            let waves = state.user.waveform_read().unwrap();
            assert_eq!(
                ron::to_string(&crate::tiles::serde::ItemListFile::from(waves.items)).unwrap(),
                before
            );
        }
        state.update(Message::Redo(1));
        assert!(state.user.waveform_read().unwrap().items.displayed_items.values().any(|item| matches!(item, DisplayedItem::Divider(divider) if divider.name.as_deref() == Some("valid"))));
    }

    #[tokio::test]
    async fn transaction_focus_is_per_view_and_records_only_inspector_creation() {
        let mut state = SystemState::new_default_config()
            .unwrap()
            .with_params(StartupParams {
                waves: Some(WaveSource::File(
                    project_root::get_project_root()
                        .unwrap()
                        .join("examples/my_db.ftr")
                        .try_into()
                        .unwrap(),
                )),
                ..Default::default()
            });
        settle(&mut state).await;
        // Load transaction bodies without adding stream rows: inspector lookup must
        // depend on the shared document, not on displayed stream membership.
        let transactions = state
            .user
            .waves
            .as_mut()
            .unwrap()
            .inner
            .as_transactions_mut()
            .unwrap();
        let streams = transactions
            .get_streams()
            .into_iter()
            .map(|stream| stream.id)
            .collect::<Vec<_>>();
        for stream in streams {
            transactions.inner.load_stream_into_memory(stream).unwrap();
        }
        add_viewport(&mut state);
        let first = TransactionRef {
            id: ftr_parser::types::TransactionId(4),
        };
        let second = TransactionRef {
            id: ftr_parser::types::TransactionId(34),
        };
        let history = state.undo_stack.len();
        state.update(Message::FocusTransaction(
            Some(first.clone()),
            tile_id(&state, 0),
        ));
        state.update(Message::FocusTransaction(
            Some(second.clone()),
            tile_id(&state, 1),
        ));
        let details = *state
            .user
            .workspace
            .tiles()
            .iter()
            .find(|(_, entry)| entry.kind.kind_name() == "transaction_details")
            .unwrap()
            .0;
        let rendered_transaction = |state: &SystemState| {
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 600.0))),
                    ..Default::default()
                },
                |ui| {
                    crate::tiles::render::PaneRenderer::ui(
                        &crate::tiles::kind::ApplicationPanes::new(state, false),
                        details,
                        true,
                        ui,
                        &mut Vec::new(),
                    );
                },
            );
            output.textures_delta.clear();
            fn collect(shape: &egui::Shape, text: &mut Vec<String>) {
                match shape {
                    egui::Shape::Text(value) => text.push(value.galley.text().into()),
                    egui::Shape::Vec(shapes) => {
                        shapes.iter().for_each(|shape| collect(shape, text))
                    }
                    _ => {}
                }
            }
            let mut text = Vec::new();
            for shape in output.shapes {
                collect(&shape.shape, &mut text);
            }
            text.windows(2)
                .find(|pair| pair[0] == "Transaction ID")
                .map(|pair| pair[1].clone())
                .unwrap_or_else(|| panic!("transaction id missing from inspector: {text:?}"))
        };
        state
            .update(Message::Workspace(
                crate::tiles::commands::WorkspaceCommand::FocusTile(tile_id(&state, 0)),
            ))
            .unwrap();
        state
            .update(Message::Workspace(
                crate::tiles::commands::WorkspaceCommand::FocusTile(details),
            ))
            .unwrap();
        assert_eq!(rendered_transaction(&state), "4");
        state
            .update(Message::Workspace(
                crate::tiles::commands::WorkspaceCommand::FocusTile(tile_id(&state, 1)),
            ))
            .unwrap();
        state
            .update(Message::Workspace(
                crate::tiles::commands::WorkspaceCommand::FocusTile(details),
            ))
            .unwrap();
        assert_eq!(rendered_transaction(&state), "34");
        assert_eq!(state.undo_stack.len(), history + 1);
        assert_eq!(state.undo_stack.last().unwrap().label(), "Open tile");
        let focus = |state: &SystemState| {
            views(state)
                .iter()
                .map(|view| view.focused_transaction.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(focus(&state), [Some(first), Some(second.clone())]);
        state.update(Message::AddDivider(Some("edit".into()), None));
        state.update(Message::FocusTransaction(None, tile_id(&state, 0)));
        state.update(Message::Undo(1));
        assert_eq!(focus(&state), [None, Some(second)]);
        // A focused transaction need not have a displayed stream in this view.
        state.update(Message::Workspace(
            crate::tiles::commands::WorkspaceCommand::FocusTile(tile_id(&state, 1)),
        ));
        state.update(Message::MoveTransaction { next: true });
    }

    #[tokio::test]
    async fn annotation_selection_is_per_view_and_survives_content_undo() {
        let mut state = loaded_counter().await;
        add_viewport(&mut state);
        let ids = [
            egui::Id::new("first annotation"),
            egui::Id::new("second annotation"),
        ];
        for (index, id) in ids.iter().enumerate() {
            state.user.waveform_edit().unwrap().items.annotations.push(
                crate::annotation::Annotation::Rect(crate::rectangle::RectAnnotation::new(
                    *id,
                    BigInt::ZERO,
                    BigInt::from(10),
                    None,
                    None,
                    Rect::ZERO,
                    index as i32,
                )),
            );
        }
        let ids: Vec<_> = state
            .user
            .waveform_read()
            .unwrap()
            .items
            .annotations
            .iter()
            .map(crate::annotation::Annotatable::get_id)
            .collect();
        state.update(Message::AnnotationClicked(
            Some(ids[0]),
            None,
            Some(tile_id(&state, 0)),
            None,
            None,
        ));
        state.update(Message::UpdateAnnotationName(ids[0], "renamed".into()));
        state.update(Message::AnnotationClicked(
            Some(ids[1]),
            None,
            Some(tile_id(&state, 1)),
            None,
            None,
        ));
        state.update(Message::Undo(1));
        let selected = |state: &SystemState| {
            views(state)
                .iter()
                .map(|view| view.selected_annotation)
                .collect::<Vec<_>>()
        };
        assert_eq!(selected(&state), [Some(ids[0]), Some(ids[1])]);
        state.update(Message::RemoveAnnotation(ids[0]));
        assert_eq!(selected(&state), [None, Some(ids[1])]);
        state.update(Message::Undo(1));
        assert_eq!(selected(&state), [None, Some(ids[1])]);
        state.update(Message::Redo(1));
        assert_eq!(selected(&state), [None, Some(ids[1])]);
    }

    #[tokio::test]
    async fn queued_row_scroll_keeps_its_tile_and_closed_targets_do_not_redirect() {
        let mut state = loaded_counter().await;
        for row in 0..32 {
            state
                .update(Message::AddDivider(Some(format!("row {row}")), None))
                .unwrap();
        }
        let first = tile_id(&state, 0);
        add_viewport(&mut state);
        let second = tile_id(&state, 1);
        let ctx = egui::Context::default();
        render(&ctx, &state, 400.0);
        for id in [first, second] {
            state
                .update(Message::WaveformBodyMeasured {
                    tile_id: id,
                    height: 100.0,
                    scroll_offset: None,
                })
                .unwrap();
        }
        state
            .update(Message::Workspace(
                crate::tiles::commands::WorkspaceCommand::FocusTile(first),
            ))
            .unwrap();
        let queued = state.scroll_rows_message(true, usize::MAX).unwrap();
        state
            .update(Message::Workspace(
                crate::tiles::commands::WorkspaceCommand::FocusTile(second),
            ))
            .unwrap();
        state.update(queued).unwrap();
        let offset = state
            .user
            .waveform_read_at(first)
            .unwrap()
            .view
            .scroll_offset;
        assert!(offset > 0.0);
        assert_eq!(
            state
                .user
                .waveform_read_at(second)
                .unwrap()
                .view
                .scroll_offset,
            0.0
        );
        assert_eq!(state.user.workspace.layout().focused(), Some(second));
        let stale = state.scroll_rows_message(false, usize::MAX).unwrap();
        state
            .update(Message::Workspace(
                crate::tiles::commands::WorkspaceCommand::CloseTile(second),
            ))
            .unwrap();
        assert!(state.update(stale).is_none());
        assert_eq!(
            state
                .user
                .waveform_read_at(first)
                .unwrap()
                .view
                .scroll_offset,
            offset
        );
    }

    #[tokio::test]
    async fn body_measurements_keep_their_origin_when_focus_changes() {
        let mut state = loaded_counter().await;
        add_viewport(&mut state);
        for id in state.user.workspace.layout().tile_order() {
            state
                .update(Message::ToTile(
                    id,
                    crate::tiles::kind::TileMessage::Waveform(
                        crate::tile_kinds::waveform::WaveformMessage::LinkVerticalScroll(true),
                    ),
                ))
                .unwrap();
        }
        state.update(Message::Workspace(
            crate::tiles::commands::WorkspaceCommand::FocusTile(tile_id(&state, 1)),
        ));
        state.update(Message::WaveformBodyMeasured {
            tile_id: crate::tiles::TileId(1),
            height: 400.0,
            scroll_offset: Some(25.0),
        });
        state.update(Message::WaveformBodyMeasured {
            tile_id: crate::tiles::TileId(2),
            height: 200.0,
            scroll_offset: Some(80.0),
        });
        state.update(Message::WaveformBodyMeasured {
            tile_id: crate::tiles::TileId(100),
            height: 10.0,
            scroll_offset: Some(0.0),
        });
        state.update(Message::WaveformBodyMeasured {
            tile_id: crate::tiles::TileId(1),
            height: f32::NAN,
            scroll_offset: Some(f32::INFINITY),
        });
        assert_eq!(
            state.user.workspace.layout().focused(),
            Some(tile_id(&state, 1))
        );
        assert_eq!(views(&state)[0].viewport_height, 400.0);
        assert_eq!(views(&state)[1].viewport_height, 200.0);
        assert_eq!(views(&state)[0].scroll_offset, 80.0);
        assert_eq!(views(&state)[1].scroll_offset, 80.0);
    }

    #[tokio::test]
    async fn borrowed_marker_edits_share_time_but_keep_independent_rows_separate() {
        use crate::tiles::{
            commands::{SplitMode, WorkspaceCommand},
            layout::Direction,
        };
        let mut state = loaded_counter().await;
        let mut runtime = std::mem::take(&mut state.workspace_runtime);
        let mut workspace = std::mem::take(&mut state.user.workspace);
        let mut document = state.user.waves.take().unwrap();
        let first = workspace.layout().focused().unwrap();
        let row = workspace
            .waveform_edit(first, &mut document)
            .unwrap()
            .add_marker(&42.into(), Some("shared time".into()), false)
            .unwrap();
        let list_id = workspace.tiles()[&first].kind.waveform_list().unwrap();
        let DisplayedItem::Marker(marker) = &workspace.item_lists()[&list_id].displayed_items[&row]
        else {
            panic!()
        };
        let marker = marker.idx;
        workspace
            .apply_command(
                &mut runtime,
                WorkspaceCommand::SplitTile {
                    tile: first,
                    dir: Direction::Right,
                    mode: SplitMode::Independent,
                },
            )
            .unwrap();
        let second = workspace.layout().focused().unwrap();
        let copy_list = workspace.tiles()[&second].kind.waveform_list().unwrap();
        document.cursor = Some(88.into());
        workspace
            .waveform_edit(first, &mut document)
            .unwrap()
            .move_marker_to_cursor(marker)
            .unwrap();
        assert_eq!(document.markers[&marker], BigInt::from(88));
        workspace
            .waveform_edit(first, &mut document)
            .unwrap()
            .remove_displayed_items(&[row]);
        assert!(
            !workspace.item_lists()[&list_id]
                .displayed_items
                .contains_key(&row)
        );
        assert!(
            workspace.item_lists()[&copy_list]
                .displayed_items
                .contains_key(&row)
        );
        assert_eq!(document.markers[&marker], BigInt::from(88));
        assert_eq!(workspace.layout().focused(), Some(second));
    }

    #[tokio::test]
    async fn adding_variables_edits_the_native_list_and_only_the_target_focus() {
        use crate::tile_kinds::waveform::WaveformMessage;
        use crate::tiles::{
            commands::{SplitMode, WorkspaceCommand},
            kind::{TileKind, TileMessage},
            layout::{Direction, Placement},
        };
        let mut state = loaded_counter().await;
        let old = state.user.waveform_read().unwrap();
        let old_count = old.items.items_tree.len();
        let old_tile = old.tile_id;
        let variable = old
            .items
            .displayed_items
            .values()
            .find_map(|item| match item {
                DisplayedItem::Variable(variable) => Some(variable.variable_ref.clone()),
                _ => None,
            })
            .unwrap();
        state
            .update(Message::Workspace(WorkspaceCommand::CreateTile {
                kind: "waveform".into(),
                placement: Placement::TabAfter(old_tile),
                focus: true,
            }))
            .unwrap();
        let first = state.user.workspace.layout().focused().unwrap();
        let list_id = state.user.workspace.tiles()[&first]
            .kind
            .waveform_list()
            .unwrap();
        let position = state.user.workspace.item_lists()[&list_id].end_insert_position();
        state
            .update(Message::ToTile(
                first,
                TileMessage::Waveform(WaveformMessage::AddDivider {
                    name: None,
                    position,
                }),
            ))
            .unwrap();
        let divider = state.user.workspace.item_lists()[&list_id]
            .items_tree
            .iter()
            .next()
            .unwrap()
            .item_ref;
        state
            .update(Message::ToTile(
                first,
                TileMessage::Waveform(WaveformMessage::FocusItem(Some(divider))),
            ))
            .unwrap();
        state
            .update(Message::Workspace(WorkspaceCommand::SplitTile {
                tile: first,
                dir: Direction::Right,
                mode: SplitMode::Linked,
            }))
            .unwrap();
        let second = state.user.workspace.layout().focused().unwrap();
        state.update(Message::AddVariables(vec![variable])).unwrap();
        let list = &state.user.workspace.item_lists()[&list_id];
        assert_eq!(list.items_tree.len(), 2);
        let inserted = list.items_tree.iter().last().unwrap().item_ref;
        assert!(matches!(
            list.displayed_items[&inserted],
            DisplayedItem::Variable(_)
        ));
        let TileKind::Waveform(first_tile) = &state.user.workspace.tiles()[&first].kind else {
            panic!()
        };
        let TileKind::Waveform(second_tile) = &state.user.workspace.tiles()[&second].kind else {
            panic!()
        };
        assert_eq!(first_tile.view.focused_item, Some(divider));
        assert_eq!(second_tile.view.focused_item, Some(inserted));
        assert_eq!(
            state
                .user
                .waveform_read_at(old_tile)
                .unwrap()
                .items
                .items_tree
                .len(),
            old_count
        );
    }

    #[tokio::test]
    async fn legacy_ownership_moves_into_linked_tiles_with_independent_time_navigation() {
        let mut state = loaded_counter().await;
        add_viewport(&mut state);
        let order = state.user.workspace.layout().tile_order();
        let focused = state.user.workspace.layout().focused();
        let list_id = state.user.workspace.tiles()[&order[0]]
            .kind
            .waveform_list()
            .unwrap();
        let mut waves = crate::tiles::legacy::LegacyWaveformV0 {
            document: state.user.waves.take().unwrap(),
            items: state.user.workspace.item_lists()[&list_id].copy_content(),
            viewports: order
                .iter()
                .map(|id| {
                    let crate::tiles::kind::TileKind::Waveform(tile) =
                        state.user.workspace.tiles()[id].kind.clone()
                    else {
                        panic!()
                    };
                    tile.view
                })
                .collect(),
            annotation_list_visible: false,
            last_active_viewport_idx: order.iter().position(|id| Some(*id) == focused).unwrap(),
        };
        let item = waves.items.items_tree.iter().next().unwrap().item_ref;
        let row_address = waves.items.items_tree.iter().next().unwrap() as *const _ as usize;
        waves.viewports[0].focused_item = Some(item);
        waves.viewports[0].scroll_offset = 25.0;
        waves.viewports[1].scroll_offset = 80.0;
        waves.viewports[1].viewport.curr_left = crate::viewport::Relative(0.25);
        waves.last_active_viewport_idx = 1;
        waves.annotation_list_visible = true;
        let mut runtime = crate::tiles::runtime::WorkspaceRuntime::default();
        let migrated = waves.into_workspace(&mut runtime).unwrap();
        let workspace = &migrated.workspace;
        let order = workspace.layout().tile_order();
        assert_eq!(order.len(), 2);
        assert_eq!(workspace.layout().focused(), Some(order[1]));
        assert_eq!(workspace.item_lists().len(), 1);
        let list = workspace.item_lists().values().next().unwrap();
        assert_eq!(
            list.items_tree.iter().next().unwrap() as *const _ as usize,
            row_address
        );
        assert!(migrated.annotation_list_visible);
        assert!(migrated.document.inner.as_waves().is_some());
        for (index, id) in order.iter().enumerate() {
            let crate::tiles::kind::TileKind::Waveform(tile) = &workspace.tiles()[id].kind else {
                panic!()
            };
            assert_eq!(tile.show_name_column, index == 0);
            assert_eq!(tile.show_value_column, index == 0);
            assert!(tile.link_vertical_scroll);
            assert_eq!(tile.view.scroll_offset, 80.0);
            assert_eq!(
                tile.view.viewport.curr_left,
                crate::viewport::Relative([0.0, 0.25][index])
            );
            assert_eq!(tile.view.focused_item, (index == 0).then_some(item));
            assert_eq!(tile.view.viewport_height, 0.0);
        }
        crate::tiles::workspace::Workspace::from_file(workspace.to_file().unwrap()).unwrap();
    }

    #[tokio::test]
    async fn unlinked_vertical_offsets_round_trip_independently() {
        let mut state = loaded_counter().await;
        let first = tile_id(&state, 0);
        add_viewport(&mut state);
        let second = tile_id(&state, 1);
        for id in [first, second] {
            state
                .update(Message::ToTile(
                    id,
                    crate::tiles::kind::TileMessage::Waveform(
                        crate::tile_kinds::waveform::WaveformMessage::LinkVerticalScroll(false),
                    ),
                ))
                .unwrap();
        }
        state.update(Message::Workspace(
            crate::tiles::commands::WorkspaceCommand::FocusTile(first),
        ));
        state.update(Message::ToTile(
            first,
            crate::tiles::kind::TileMessage::Waveform(
                crate::tile_kinds::waveform::WaveformMessage::ScrollTo(25.0),
            ),
        ));
        state.update(Message::Workspace(
            crate::tiles::commands::WorkspaceCommand::FocusTile(second),
        ));
        state.update(Message::ToTile(
            second,
            crate::tiles::kind::TileMessage::Waveform(
                crate::tile_kinds::waveform::WaveformMessage::ScrollTo(80.0),
            ),
        ));
        let encoded = ron::to_string(&state.user.workspace).unwrap();
        let restored: crate::tiles::workspace::Workspace = ron::from_str(&encoded).unwrap();
        assert_eq!(restored.layout().focused(), Some(second));
        assert_eq!(
            restored.waveform_resources(first).unwrap().1.scroll_offset,
            25.0
        );
        assert_eq!(
            restored.waveform_resources(second).unwrap().1.scroll_offset,
            80.0
        );
    }

    #[tokio::test]
    async fn drawing_two_views_reuses_each_cache_and_resizes_only_one() {
        let mut state = loaded_counter().await;
        add_viewport(&mut state);
        let ctx = egui::Context::default();
        for _ in 0..3 {
            render(&ctx, &state, 400.0);
        }
        let builds = || {
            views(&state)
                .iter()
                .map(|view| view.draw_cache.borrow().builds)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            builds(),
            [1, 1],
            "each view should build once and then reuse its own cache"
        );
        render(&ctx, &state, 500.0);
        assert_eq!(
            builds(),
            [1, 2],
            "resizing the second view must not evict the first"
        );

        // A newly created view always carries a fresh cache of its own.
        add_viewport(&mut state);
        assert_eq!(views(&state)[2].draw_cache.borrow().builds, 0);
    }
}
