use color_eyre::eyre::WrapErr;
use color_eyre::Result;
use ecolor::Color32;
use egui::{PointerButton, Response, Sense, Ui};
use egui_extras::{Column, TableBuilder};
use emath::{Align2, Pos2, Rect, RectTransform, Vec2};
use epaint::{
    CornerRadiusF32, CubicBezierShape, FontId, PathShape, PathStroke, RectShape, Shape, Stroke,
};
use ftr_parser::types::{Transaction, TxGenerator};
use itertools::Itertools;
use log::{error, warn};
use num::bigint::{ToBigInt, ToBigUint};
use num::{BigInt, BigUint, ToPrimitive};
use rayon::prelude::{IntoParallelRefIterator, ParallelBridge, ParallelIterator};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::f64::consts::PI;
use surfer_translation_types::{
    NumericTranslator, SubFieldFlatTranslationResult, TranslatedValue, TranslationResult,
    ValueKind, ValueRepr, VariableInfo, VariableType, VariableValue,
};

use crate::analog::{AnalogDisplayMode, AnalogRangeMode};
use crate::clock_highlighting::draw_clock_edge;
use crate::config::SurferTheme;
use crate::data_container::DataContainer;
use crate::displayed_item::{DisplayedFieldRef, DisplayedItemRef, DisplayedVariable};
use crate::displayed_item_tree::VisibleItemIndex;
use crate::transaction_container::{TransactionRef, TransactionStreamRef};
use crate::translation::{
    AnyTranslator, DynNumericTranslator, TranslationResultExt, TranslatorList, ValueKindExt,
    VariableInfoExt,
};
use crate::view::{DrawConfig, DrawingContext, ItemDrawingInfo};
use crate::viewport::Viewport;
use crate::wave_container::{
    QueryResult, VariableMeta, VariableRef, VariableRefExt, WaveContainer,
};
use crate::wave_data::WaveData;
use crate::CachedDrawData::TransactionDrawData;
use crate::{
    displayed_item::DisplayedItem, CachedDrawData, CachedTransactionDrawData, CachedWaveDrawData,
    Message, State,
};

pub struct DigitalWaveformDrawRegion {
    value: TranslatedValue,
    /// True if a transition should be drawn even if there is no change in the value
    /// between the previous and next pixels. Only used by the bool drawing logic to
    /// draw draw a vertical line and prevent apparent aliasing
    force_anti_alias: bool,
}

pub struct AnalogPlotDrawData {
    mode: AnalogDisplayMode,
    normalization_range: (f64, f64),
    values: Vec<(f32, f32)>,
}

pub enum DrawingGeometry {
    DigitalWaveform {
        clock: bool,
        values: Vec<(f32, DigitalWaveformDrawRegion)>,
    },

    // For displaying more complex types like numbers or strings.
    InterpretedWaveform {
        values: Vec<(f32, TranslatedValue)>,
    },
    AnalogPlot(AnalogPlotDrawData),
}

pub struct TxDrawingCommands {
    min: Pos2,
    max: Pos2,
    gen_ref: TransactionStreamRef, // makes it easier to later access the actual Transaction object
}

struct VariableDrawCommands {
    clock_edges: Vec<f32>,
    display_id: DisplayedItemRef,
    geometries: HashMap<Vec<String>, DrawingGeometry>,
    local_msgs: Vec<Message>,
}

fn sample_variable_for_drawing(
    waves: &WaveContainer,
    var_ref: &VariableRef,
    timestamps: &[(i32, num::BigUint)],
) -> Result<Vec<(i32, VariableValue, bool)>> {
    let mut samples = vec![];

    // Iterate over all the time stamps to draw on
    let mut next_change = timestamps.first().map(|t| t.1.clone()).unwrap_or_default();
    for (i, (pixel, time)) in timestamps.iter().enumerate() {
        let is_last = i == timestamps.len() - 1;

        if *time < next_change && !is_last {
            continue;
        }

        let prev_time = if i == 0 {
            BigUint::ZERO
        } else {
            timestamps.get(i - 1).map(|t| t.1.clone()).unwrap()
        };

        let query_result = waves.query_variable(var_ref, time)?;

        match query_result {
            None => return Ok(vec![]),
            Some(QueryResult { current, next }) => {
                next_change = match next {
                    Some(timestamp) => timestamp.clone(),
                    None => timestamps.last().map(|t| t.1.clone()).unwrap_or_default(),
                };

                let (change_time, val) = match current {
                    Some((change_time, val)) => (change_time, val),
                    None => continue,
                };

                samples.push((*pixel, val, prev_time < change_time));
            }
        }
    }

    return Ok(samples);
}

fn variable_draw_commands(
    displayed_variable: &DisplayedVariable,
    display_id: DisplayedItemRef,
    timestamps: &[(i32, num::BigUint)],
    waves: &WaveData,
    translators: &TranslatorList,
) -> Option<VariableDrawCommands> {
    let waves_container = waves.inner.as_waves().unwrap();

    let mut clock_edges = vec![];
    let mut local_msgs = vec![];

    let meta = match waves
        .inner
        .as_waves()
        .unwrap()
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
    let translator = waves.variable_translator(&displayed_field_ref, translators);
    // we need to get the variable info here to get the correct info for aliases
    let info = translator.variable_info(&meta).unwrap();

    let mut geometries: HashMap<Vec<_>, _> = HashMap::new();

    let mut prev_values = HashMap::new();

    let samples = match sample_variable_for_drawing(
        waves_container,
        &displayed_variable.variable_ref,
        timestamps,
    ) {
        Ok(s) => s,
        Err(e) => {
            error!("Variable query error {e:#?}");
            return None;
        }
    };

    for (subfield, subinfo) in VariableInfo::flatten(&info) {
        let geometry = match subinfo {
            VariableInfo::Bool => DrawingGeometry::DigitalWaveform {
                clock: false,
                values: vec![],
            },
            VariableInfo::Clock => DrawingGeometry::DigitalWaveform {
                clock: true,
                values: vec![],
            },
            _ => DrawingGeometry::InterpretedWaveform { values: vec![] },
        };

        geometries.insert(subfield, geometry);
    }

    for (i, (pixel, val, changed_between_last_pixel)) in samples.iter().enumerate() {
        let is_first_timestep = i == 0;
        let is_last_sample = i == samples.len() - 1;

        let translation_result = match translator.translate(&meta, &val) {
            Ok(result) => result,
            Err(e) => {
                error!(
                    "{translator_name} for {variable_name} failed. Disabling:",
                    translator_name = translator.name(),
                    variable_name = displayed_variable.variable_ref.full_path_string()
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

        for SubFieldFlatTranslationResult { names, value } in fields {
            let prev = prev_values.get(&names);

            // If the value changed between this and the previous pixel, we want to
            // draw a transition even if the translated value didn't change.  We
            // only want to do this for root variables, because resolving when a
            // sub-field change is tricky without more information from the
            // translators
            let anti_alias = *changed_between_last_pixel
                && names.is_empty()
                && waves.inner.as_waves().unwrap().wants_anti_aliasing();
            let new_value = prev != Some(&value);

            // This is not the value we drew last time
            if new_value || is_last_sample || anti_alias {
                prev_values
                    .entry(names.clone())
                    .or_insert(value.clone())
                    .clone_from(&value);

                match geometries.get_mut(&names.clone()).unwrap() {
                    DrawingGeometry::DigitalWaveform { clock, values } => {
                        if let Some(trval) = value.as_ref() {
                            values.push((
                                *pixel as f32,
                                DigitalWaveformDrawRegion {
                                    value: trval.clone(),
                                    force_anti_alias: anti_alias && !new_value,
                                },
                            ));

                            if *clock {
                                if !is_last_sample && !is_first_timestep {
                                    clock_edges.push(*pixel as f32);
                                }
                            }
                        }
                    }

                    DrawingGeometry::InterpretedWaveform { values } => {
                        values.push((*pixel as f32, value.unwrap()));
                    }

                    _ => {}
                }
            }
        }
    }
    Some(VariableDrawCommands {
        clock_edges,
        display_id,
        geometries,
        local_msgs,
    })
}

fn variable_analog_draw_command(
    displayed_variable: &DisplayedVariable,
    display_id: DisplayedItemRef,
    timestamps: &[(i32, num::BigUint)],
    waves: &WaveData,
    translators: &TranslatorList,
) -> Option<VariableDrawCommands> {
    let waves_container = waves.inner.as_waves().unwrap();

    let meta = match waves_container
        .variable_meta(&displayed_variable.variable_ref)
        .context("failed to get variable meta")
    {
        Ok(meta) => meta,
        Err(e) => {
            warn!("{e:#?}");
            return None;
        }
    };

    let samples = match sample_variable_for_drawing(
        waves_container,
        &displayed_variable.variable_ref,
        timestamps,
    ) {
        Ok(s) => s,
        Err(e) => {
            error!("Variable query error {e:#?}");
            return None;
        }
    };

    let translator = match translators.get_translator(displayed_variable.format.as_ref().unwrap()) {
        AnyTranslator::Numeric(translator) => translator,
        _ => {
            error!("Failed to get numeric translator. Disabling analog mode TODO");
            return None;
        }
    };

    let value_range = match displayed_variable.analog_view.range {
        AnalogRangeMode::TypeRange => translator.variable_range(&meta),
    };

    let mut values = vec![];
    for (pixel, val, _) in samples {
        let translation_result = match translator.translate(&meta, &val) {
            Ok(result) => result,
            Err(e) => {
                error!(
                    "{translator_name} for {variable_name} failed. Disabling:",
                    translator_name = translator.name(),
                    variable_name = displayed_variable.variable_ref.full_path_string()
                );
                error!("{e:#}");
                //local_msgs.push(Message::ResetVariableFormat(displayed_field_ref));
                return None;
            }
        };

        let (analog_val, _) = translation_result.as_f64();

        values.push((pixel as f32, analog_val as f32));
    }

    let commands = HashMap::from([(
        vec![],
        DrawingGeometry::AnalogPlot(AnalogPlotDrawData {
            mode: displayed_variable.analog_view.mode,
            normalization_range: value_range,
            values,
        }),
    )]);

    return Some(VariableDrawCommands {
        clock_edges: vec![],
        display_id,
        geometries: commands,
        local_msgs: vec![],
    });
}

impl State {
    pub fn invalidate_draw_commands(&mut self) {
        if let Some(waves) = &self.waves {
            for viewport in 0..waves.viewports.len() {
                self.sys.draw_data.borrow_mut()[viewport] = None;
            }
        }
    }

    pub fn generate_draw_commands(
        &self,
        cfg: &DrawConfig,
        frame_width: f32,
        msgs: &mut Vec<Message>,
        viewport_idx: usize,
    ) {
        #[cfg(feature = "performance_plot")]
        self.sys.timing.borrow_mut().start("Generate draw commands");
        if let Some(waves) = &self.waves {
            let draw_data = match waves.inner {
                DataContainer::Waves(_) => {
                    self.generate_wave_draw_commands(waves, cfg, frame_width, msgs, viewport_idx)
                }
                DataContainer::Transactions(_) => self.generate_transaction_draw_commands(
                    waves,
                    cfg,
                    frame_width,
                    msgs,
                    viewport_idx,
                ),
                DataContainer::Empty => None,
            };
            self.sys.draw_data.borrow_mut()[viewport_idx] = draw_data;
        }
        #[cfg(feature = "performance_plot")]
        self.sys.timing.borrow_mut().end("Generate draw commands");
    }

    fn generate_wave_draw_commands(
        &self,
        waves: &WaveData,
        cfg: &DrawConfig,
        frame_width: f32,
        msgs: &mut Vec<Message>,
        viewport_idx: usize,
    ) -> Option<CachedDrawData> {
        let mut draw_commands = HashMap::new();

        let num_timestamps = waves.num_timestamps().unwrap_or(1.into());
        let max_time = num_timestamps.to_f64().unwrap_or(f64::MAX);
        let mut clock_edges = vec![];
        // Compute which timestamp to draw in each pixel. We'll draw from -transition_width to
        // width + transition_width in order to draw initial transitions outside the screen
        let mut timestamps = (-cfg.max_transition_width
            ..(frame_width as i32 + cfg.max_transition_width))
            .par_bridge()
            .filter_map(|x| {
                let time = waves.viewports[viewport_idx]
                    .as_absolute_time(x as f64 + 0.5, frame_width, &num_timestamps)
                    .0;
                if time < 0. || time > max_time {
                    None
                } else {
                    Some((x, time.to_biguint().unwrap_or_default()))
                }
            })
            .collect::<Vec<_>>();
        timestamps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));

        let translators = &self.sys.translators;
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
                if displayed_variable.analog_view.enabled {
                    variable_analog_draw_command(
                        displayed_variable,
                        id,
                        &timestamps,
                        waves,
                        translators,
                    )
                } else {
                    variable_draw_commands(displayed_variable, id, &timestamps, waves, translators)
                }
            })
            .collect::<Vec<_>>();

        for VariableDrawCommands {
            clock_edges: mut new_clock_edges,
            display_id,
            geometries,
            mut local_msgs,
        } in commands
        {
            msgs.append(&mut local_msgs);
            for (field, val) in geometries {
                draw_commands.insert(
                    DisplayedFieldRef {
                        item: display_id,
                        field,
                    },
                    val,
                );
            }
            clock_edges.append(&mut new_clock_edges);
        }
        let ticks = waves.get_ticks(
            &waves.viewports[viewport_idx],
            &waves.inner.metadata().timescale,
            frame_width,
            cfg.text_size,
            &self.wanted_timeunit,
            &self.get_time_format(),
            &self.config,
        );

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
        frame_width: f32,
        msgs: &mut Vec<Message>,
        viewport_idx: usize,
    ) -> Option<CachedDrawData> {
        let mut draw_commands = HashMap::new();
        let mut stream_to_displayed_txs = HashMap::new();
        let mut inc_relation_tx_ids = vec![];
        let mut out_relation_tx_ids = vec![];

        let (focused_tx_ref, old_focused_tx) = &waves.focused_transaction;
        let mut new_focused_tx: Option<&Transaction> = None;

        let viewport = waves.viewports[viewport_idx];
        let num_timestamps = waves.num_timestamps().unwrap_or(1.into());

        let displayed_streams = waves
            .items_tree
            .iter_visible()
            .map(|node| node.item_ref)
            .collect::<Vec<_>>()
            .par_iter()
            .map(|id| waves.displayed_items.get(id))
            .filter_map(|item| match item {
                Some(DisplayedItem::Stream(stream_ref)) => Some(stream_ref),
                _ => None,
            })
            .collect::<Vec<_>>();

        let first_visible_timestamp = viewport
            .curr_left
            .absolute(&num_timestamps)
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

            for gen in &generators {
                // find first visible transaction
                let first_visible_transaction_index = match gen
                    .transactions
                    .binary_search_by_key(&first_visible_timestamp, |tx| tx.get_end_time())
                {
                    Ok(i) => i,
                    Err(i) => i,
                }
                .saturating_sub(1);
                let transactions = gen
                    .transactions
                    .iter()
                    .skip(first_visible_transaction_index);

                let mut last_px = f32::NAN;

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

                    if let Some(focused_tx_ref) = focused_tx_ref {
                        if curr_tx_id == focused_tx_ref.id {
                            new_focused_tx = Some(tx);
                        }
                    }

                    let min_px = viewport.pixel_from_time(
                        &start_time.to_bigint().unwrap(),
                        frame_width - 1.,
                        &num_timestamps,
                    );
                    let max_px = viewport.pixel_from_time(
                        &end_time.to_bigint().unwrap(),
                        frame_width - 1.,
                        &num_timestamps,
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
                                gen.id,
                                gen.name.clone(),
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
            if old_focused_tx.is_none() || Some(focused_tx) != old_focused_tx.as_ref() {
                msgs.push(Message::FocusTransaction(
                    focused_tx_ref.clone(),
                    Some(focused_tx.clone()),
                ));
            }
        }

        Some(TransactionDrawData(CachedTransactionDrawData {
            draw_commands,
            stream_to_displayed_txs,
            inc_relation_tx_ids,
            out_relation_tx_ids,
        }))
    }

    pub fn draw_items(
        &mut self,
        egui_ctx: &egui::Context,
        msgs: &mut Vec<Message>,
        ui: &mut Ui,
        viewport_idx: usize,
    ) {
        let Some(waves) = &self.waves else { return };

        let (response, mut painter) =
            ui.allocate_painter(ui.available_size(), Sense::click_and_drag());

        let cfg = match waves.inner {
            DataContainer::Waves(_) => DrawConfig::new(
                response.rect.size().y,
                self.config.layout.waveforms_line_height,
                self.config.layout.waveforms_text_size,
            ),
            DataContainer::Transactions(_) => DrawConfig::new(
                response.rect.size().y,
                self.config.layout.transactions_line_height,
                self.config.layout.waveforms_text_size,
            ),
            DataContainer::Empty => return,
        };
        // the draw commands have been invalidated, recompute
        if self.sys.draw_data.borrow()[viewport_idx].is_none()
            || Some(response.rect) != *self.sys.last_canvas_rect.borrow()
        {
            self.generate_draw_commands(&cfg, response.rect.width(), msgs, viewport_idx);
            *self.sys.last_canvas_rect.borrow_mut() = Some(response.rect);
        }

        let Some(waves) = &self.waves else { return };
        if response.rect.size().x < 1. {
            return;
        }
        let container_rect = Rect::from_min_size(Pos2::ZERO, response.rect.size());
        let to_screen = RectTransform::from_to(container_rect, response.rect);
        let frame_width = response.rect.width();
        let pointer_pos_global = ui.input(|i| i.pointer.interact_pos());
        let pointer_pos_canvas = pointer_pos_global.map(|p| {
            to_screen
                .inverse()
                .transform_pos(if self.show_default_timeline() {
                    Pos2 {
                        x: p.x,
                        y: p.y - ui.text_style_height(&egui::TextStyle::Body),
                    }
                } else {
                    p
                })
        });
        let num_timestamps = waves.num_timestamps().unwrap_or(1.into());

        if ui.ui_contains_pointer() {
            let pointer_pos = pointer_pos_global.unwrap();
            let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
            let mouse_ptr_pos = to_screen.inverse().transform_pos(pointer_pos);
            if scroll_delta != Vec2::ZERO {
                msgs.push(Message::CanvasScroll {
                    delta: ui.input(|i| i.smooth_scroll_delta),
                    viewport_idx,
                });
            }

            if ui.input(egui::InputState::zoom_delta) != 1. {
                let mouse_ptr = Some(waves.viewports[viewport_idx].as_time_bigint(
                    mouse_ptr_pos.x,
                    frame_width,
                    &num_timestamps,
                ));

                msgs.push(Message::CanvasZoom {
                    mouse_ptr,
                    delta: ui.input(egui::InputState::zoom_delta),
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
                    viewport_idx: 0,
                });
            }
        });

        let modifiers = egui_ctx.input(|i| i.modifiers);
        if !modifiers.command
            && (response.dragged_by(PointerButton::Primary)
                || response.clicked_by(PointerButton::Primary))
        {
            if let Some(snap_point) =
                self.snap_to_edge(pointer_pos_canvas, waves, frame_width, viewport_idx)
            {
                msgs.push(Message::CursorSet(snap_point));
            }
        }
        painter.rect_filled(
            response.rect,
            CornerRadiusF32::ZERO,
            self.config.theme.canvas_colors.background,
        );

        // Check for mouse gesture starting
        if response.drag_started_by(PointerButton::Middle)
            || modifiers.command && response.drag_started_by(PointerButton::Primary)
        {
            msgs.push(Message::SetDragStart(
                response
                    .interact_pointer_pos()
                    .map(|p| to_screen.inverse().transform_pos(p)),
            ));
        }

        let mut ctx = DrawingContext {
            painter: &mut painter,
            cfg: &cfg,
            to_screen: &|x, y| {
                // As of 0.29 Egui draws things in the middle of pixels which means that if our
                // line width is odd, we'll get very fuzzy lines. To compensate for this, we'll
                // offset the coordinates by 0.5 if we have an odd line width.
                // Relevant issues: https://github.com/emilk/egui/issues/1322
                //                  https://github.com/emilk/egui/pull/4943
                let offset = if (self.config.theme.linewidth as i32) % 2 == 1 {
                    Vec2::new(0.5, 0.5)
                } else {
                    Vec2::ZERO
                };
                to_screen.transform_pos(Pos2::new(x, y) + offset)
            },
            theme: &self.config.theme,
        };

        let gap = ui.spacing().item_spacing.y * 0.5;
        // We draw in absolute coords, but the variable offset in the y
        // direction is also in absolute coordinates, so we need to
        // compensate for that
        let y_zero = to_screen.transform_pos(Pos2::ZERO).y;
        for (vidx, drawing_info) in waves.drawing_infos.iter().enumerate() {
            // Get background color
            let background_color =
                &self.get_background_color(waves, drawing_info, VisibleItemIndex(vidx));

            self.draw_background(
                drawing_info,
                y_zero,
                &ctx,
                gap,
                frame_width,
                background_color,
            );
        }

        #[cfg(feature = "performance_plot")]
        self.sys.timing.borrow_mut().start("Wave drawing");

        match &self.sys.draw_data.borrow()[viewport_idx] {
            Some(CachedDrawData::WaveDrawData(draw_data)) => {
                self.draw_wave_data(waves, draw_data, &mut ctx);
            }
            Some(CachedDrawData::TransactionDrawData(draw_data)) => {
                self.draw_transaction_data(
                    waves,
                    draw_data,
                    viewport_idx,
                    frame_width,
                    &cfg,
                    ui,
                    msgs,
                    &mut ctx,
                );
            }
            None => {}
        }
        #[cfg(feature = "performance_plot")]
        self.sys.timing.borrow_mut().end("Wave drawing");

        waves.draw_graphics(
            &mut ctx,
            response.rect.size(),
            &waves.viewports[viewport_idx],
            &self.config.theme,
        );

        waves.draw_cursor(
            &self.config.theme,
            &mut ctx,
            response.rect.size(),
            &waves.viewports[viewport_idx],
        );

        waves.draw_markers(
            &self.config.theme,
            &mut ctx,
            response.rect.size(),
            &waves.viewports[viewport_idx],
        );

        self.draw_marker_boxes(
            waves,
            &mut ctx,
            response.rect.size().x,
            gap,
            &waves.viewports[viewport_idx],
            y_zero,
        );

        if self.show_default_timeline() {
            let rect = Rect {
                min: Pos2 { x: 0.0, y: y_zero },
                max: Pos2 {
                    x: response.rect.max.x,
                    y: y_zero + ui.text_style_height(&egui::TextStyle::Body),
                },
            };
            ctx.painter
                .rect_filled(rect, 0.0, self.config.theme.canvas_colors.background);
            self.draw_default_timeline(waves, &ctx, viewport_idx, frame_width, &cfg);
        }

        self.draw_mouse_gesture_widget(
            egui_ctx,
            waves,
            pointer_pos_canvas,
            &response,
            msgs,
            &mut ctx,
            viewport_idx,
        );
        self.handle_canvas_context_menu(response, waves, to_screen, &mut ctx, msgs, viewport_idx);
    }

    fn draw_wave_data(
        &self,
        waves: &WaveData,
        draw_data: &CachedWaveDrawData,
        ctx: &mut DrawingContext,
    ) {
        let clock_edges = &draw_data.clock_edges;
        let draw_commands = &draw_data.draw_commands;
        let draw_clock_edges = match clock_edges.as_slice() {
            [] => false,
            [_single] => true,
            [first, second, ..] => second - first > 20.,
        };
        let draw_clock_rising_marker = draw_clock_edges && self.config.theme.clock_rising_marker;
        let ticks = &draw_data.ticks;
        if !ticks.is_empty() && self.show_ticks() {
            let stroke = Stroke {
                color: self.config.theme.ticks.style.color,
                width: self.config.theme.ticks.style.width,
            };

            for (_, x) in ticks {
                waves.draw_tick_line(*x, ctx, &stroke);
            }
        }

        if draw_clock_edges {
            let mut last_edge = 0.0;
            let mut cycle = false;
            for current_edge in clock_edges {
                draw_clock_edge(last_edge, *current_edge, cycle, ctx, &self.config);
                cycle = !cycle;
                last_edge = *current_edge;
            }
        }
        let zero_y = (ctx.to_screen)(0., 0.).y;
        for (vidx, drawing_info) in waves.drawing_infos.iter().enumerate() {
            let vidx = VisibleItemIndex(vidx);
            // We draw in absolute coords, but the variable offset in the y
            // direction is also in absolute coordinates, so we need to
            // compensate for that
            let y_offset = drawing_info.top() - zero_y;

            let displayed_item = waves
                .items_tree
                .get_visible(drawing_info.item_list_idx())
                .and_then(|node| waves.displayed_items.get(&node.item_ref));
            let color = displayed_item
                .and_then(super::displayed_item::DisplayedItem::color)
                .and_then(|color| self.config.theme.get_color(color));

            match drawing_info {
                ItemDrawingInfo::Variable(variable_info) => {
                    if let Some(command) = draw_commands.get(&variable_info.displayed_field_ref) {
                        // Get background color and determine best text color
                        let background_color = self.get_background_color(waves, drawing_info, vidx);
                        let text_color = self.config.theme.get_best_text_color(&background_color);
                        let height_scaling_factor = displayed_item
                            .map(super::displayed_item::DisplayedItem::height_scaling_factor)
                            .unwrap();

                        let color = *color.unwrap_or_else(|| {
                            if let Some(DisplayedItem::Variable(variable)) = displayed_item {
                                waves
                                    .inner
                                    .as_waves()
                                    .unwrap()
                                    .variable_meta(&variable.variable_ref)
                                    .ok()
                                    .and_then(|meta| meta.variable_type)
                                    .and_then(|var_type| {
                                        if var_type == VariableType::VCDParameter {
                                            Some(&self.config.theme.variable_parameter)
                                        } else {
                                            None
                                        }
                                    })
                                    .unwrap_or(&self.config.theme.variable_default)
                            } else {
                                &self.config.theme.variable_default
                            }
                        });
                        match command {
                            DrawingGeometry::AnalogPlot(data) => {
                                self.draw_analog_plot(
                                    data,
                                    color,
                                    y_offset,
                                    height_scaling_factor,
                                    ctx,
                                );
                            }
                            DrawingGeometry::DigitalWaveform { clock, values } => {
                                self.draw_digital_waveform(
                                    values,
                                    color,
                                    y_offset,
                                    height_scaling_factor,
                                    *clock && draw_clock_rising_marker,
                                    ctx,
                                );
                            }
                            DrawingGeometry::InterpretedWaveform { values } => {
                                self.draw_value_waveform(
                                    &values,
                                    color,
                                    y_offset,
                                    height_scaling_factor,
                                    ctx,
                                    *text_color,
                                );
                            }
                        }
                    }
                }
                ItemDrawingInfo::Divider(_) => {}
                ItemDrawingInfo::Marker(_) => {}
                ItemDrawingInfo::TimeLine(_) => {
                    let text_color = color.unwrap_or(
                        // Get background color and determine best text color
                        self.config
                            .theme
                            .get_best_text_color(&self.get_background_color(
                                waves,
                                drawing_info,
                                vidx,
                            )),
                    );
                    waves.draw_ticks(
                        Some(text_color),
                        ticks,
                        ctx,
                        y_offset,
                        Align2::CENTER_TOP,
                        &self.config,
                    );
                }
                ItemDrawingInfo::Stream(_) => {}
                ItemDrawingInfo::Group(_) => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_transaction_data(
        &self,
        waves: &WaveData,
        draw_data: &CachedTransactionDrawData,
        viewport_idx: usize,
        frame_width: f32,
        cfg: &DrawConfig,
        ui: &mut Ui,
        msgs: &mut Vec<Message>,
        ctx: &mut DrawingContext,
    ) {
        let draw_commands = &draw_data.draw_commands;
        let stream_to_displayed_txs = &draw_data.stream_to_displayed_txs;
        let inc_relation_tx_ids = &draw_data.inc_relation_tx_ids;
        let out_relation_tx_ids = &draw_data.out_relation_tx_ids;

        let mut inc_relation_starts = vec![];
        let mut out_relation_starts = vec![];
        let mut focused_transaction_start: Option<Pos2> = None;

        let ticks = &waves.get_ticks(
            &waves.viewports[viewport_idx],
            &waves.inner.metadata().timescale,
            frame_width,
            cfg.text_size,
            &self.wanted_timeunit,
            &self.get_time_format(),
            &self.config,
        );

        if !ticks.is_empty() && self.show_ticks() {
            let stroke = Stroke {
                color: self.config.theme.ticks.style.color,
                width: self.config.theme.ticks.style.width,
            };

            for (_, x) in ticks {
                waves.draw_tick_line(*x, ctx, &stroke);
            }
        }

        let zero_y = (ctx.to_screen)(0., 0.).y;
        for (vidx, drawing_info) in waves.drawing_infos.iter().enumerate() {
            let y_offset = drawing_info.top() - zero_y;

            let displayed_item = waves
                .items_tree
                .get_visible(drawing_info.item_list_idx())
                .and_then(|node| waves.displayed_items.get(&node.item_ref));
            let color = displayed_item
                .and_then(super::displayed_item::DisplayedItem::color)
                .and_then(|color| self.config.theme.get_color(color));
            // Draws the surrounding border of the stream
            let border_stroke =
                Stroke::new(self.config.theme.linewidth, self.config.theme.foreground);

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
                                max.x = max.x.min(frame_width - 1.);

                                let min = (ctx.to_screen)(min.x, y_offset + min.y);
                                let max = (ctx.to_screen)(max.x, y_offset + max.y);

                                let start = Pos2::new(min.x, (min.y + max.y) / 2.);

                                let is_transaction_focused = waves
                                    .focused_transaction
                                    .0
                                    .as_ref()
                                    .is_some_and(|t| t == tx_ref);

                                if inc_relation_tx_ids.contains(tx_ref) {
                                    inc_relation_starts.push(start);
                                } else if out_relation_tx_ids.contains(tx_ref) {
                                    out_relation_starts.push(start);
                                } else if is_transaction_focused {
                                    focused_transaction_start = Some(start);
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
                                            None,
                                        ));
                                    }

                                    let tx_fill_color = if is_transaction_focused {
                                        let c =
                                            color.unwrap_or(&self.config.theme.transaction_default);
                                        Color32::from_rgb(255 - c.r(), 255 - c.g(), 255 - c.b())
                                    } else {
                                        *color.unwrap_or(&self.config.theme.transaction_default)
                                    };

                                    let stroke =
                                        Stroke::new(1.5, tx_fill_color.gamma_multiply(1.2));
                                    ctx.painter.rect(
                                        transaction_rect,
                                        CornerRadiusF32::same(5.0),
                                        tx_fill_color,
                                        stroke,
                                        egui::StrokeKind::Middle,
                                    );
                                } else {
                                    let tx_fill_color = color
                                        .unwrap_or(&self.config.theme.transaction_default)
                                        .gamma_multiply(1.2);

                                    let stroke = Stroke::new(1.5, tx_fill_color);
                                    ctx.painter.rect(
                                        transaction_rect,
                                        CornerRadiusF32::ZERO,
                                        tx_fill_color,
                                        stroke,
                                        egui::StrokeKind::Middle,
                                    );
                                }
                            }
                        }
                        ctx.painter.hline(
                            0.0..=((ctx.to_screen)(frame_width, 0.0).x),
                            drawing_info.bottom(),
                            border_stroke,
                        );
                    }
                }
                ItemDrawingInfo::TimeLine(_) => {
                    let text_color = color.unwrap_or(
                        // Get background color and determine best text color
                        self.config
                            .theme
                            .get_best_text_color(&self.get_background_color(
                                waves,
                                drawing_info,
                                VisibleItemIndex(vidx),
                            )),
                    );
                    waves.draw_ticks(
                        Some(text_color),
                        ticks,
                        ctx,
                        y_offset,
                        Align2::CENTER_TOP,
                        &self.config,
                    );
                }
                ItemDrawingInfo::Variable(_) => {}
                ItemDrawingInfo::Divider(_) => {}
                ItemDrawingInfo::Marker(_) => {}
                ItemDrawingInfo::Group(_) => {}
            }
        }

        // Draws the relations of the focused transaction
        if let Some(focused_pos) = focused_transaction_start {
            let arrow_color = self.config.theme.relation_arrow;
            for start_pos in inc_relation_starts {
                self.draw_arrow(start_pos, focused_pos, 25., arrow_color, ctx);
            }

            for end_pos in out_relation_starts {
                self.draw_arrow(focused_pos, end_pos, 25., arrow_color, ctx);
            }
        }
    }

    fn draw_value_waveform(
        &self,
        values: &Vec<(f32, TranslatedValue)>,
        user_color: Color32,
        offset: f32,
        height_scaling_factor: f32,
        ctx: &mut DrawingContext,
        text_color: Color32,
    ) {
        for ((old_x, prev_value), (new_x, _)) in values.iter().zip(values.iter().skip(1)) {
            let stroke = Stroke {
                color: prev_value.kind.color(user_color, ctx.theme),
                width: self.config.theme.linewidth,
            };

            let transition_width = (new_x - old_x).min(6.);

            let trace_coords =
                |x, y| (ctx.to_screen)(x, y * ctx.cfg.line_height * height_scaling_factor + offset);

            ctx.painter.add(PathShape::line(
                vec![
                    trace_coords(*old_x, 0.5),
                    trace_coords(old_x + transition_width / 2., 1.0),
                    trace_coords(new_x - transition_width / 2., 1.0),
                    trace_coords(*new_x, 0.5),
                    trace_coords(new_x - transition_width / 2., 0.0),
                    trace_coords(old_x + transition_width / 2., 0.0),
                    trace_coords(*old_x, 0.5),
                ],
                stroke,
            ));

            let text_size = ctx.cfg.text_size;
            let char_width = text_size * (20. / 31.);

            let text_area = (new_x - old_x) - transition_width;
            let num_chars = (text_area / char_width).floor() as usize;
            let fits_text = num_chars >= 1;

            let text = &prev_value.value;

            if fits_text {
                let content = if text.len() > num_chars {
                    text.chars()
                        .take(num_chars - 1)
                        .chain(['…'])
                        .collect::<String>()
                } else {
                    text.to_string()
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

    fn draw_analog_plot(
        &self,
        data: &AnalogPlotDrawData,
        color: Color32,
        offset: f32,
        height_scaling_factor: f32,
        ctx: &mut DrawingContext,
    ) {
        for (old, new) in data.values.iter().zip(data.values.iter().skip(1)) {
            let trace_coords = |x, y| {
                (ctx.to_screen)(
                    x,
                    y as f32 * ctx.cfg.line_height * height_scaling_factor + offset,
                )
            };

            let (min, max) = data.normalization_range;
            let normalize = |y: f64| {
                if max == min {
                    0.0
                } else {
                    (y - min) / (max - min)
                }
            };

            let old_height = normalize(old.1 as f64);
            let new_height = normalize(new.1 as f64);

            let line_points = match data.mode {
                AnalogDisplayMode::LinearInterpolation => vec![
                    trace_coords(old.0, 1. - old_height),
                    trace_coords(new.0, 1. - new_height),
                ],
                AnalogDisplayMode::Step => vec![
                    trace_coords(old.0, 1. - old_height),
                    trace_coords(new.0, 1. - old_height),
                    trace_coords(new.0, 1. - new_height),
                ],
            };

            let stroke = Stroke {
                color: color,
                width: self.config.theme.linewidth,
            };

            ctx.painter.add(PathShape::line(line_points, stroke));
        }
    }

    fn bool_drawing_spec(
        str: &str,
        user_color: Color32,
        theme: &SurferTheme,
        value_kind: ValueKind,
    ) -> (f32, Color32, Option<Color32>) {
        let color = value_kind.color(user_color, theme);
        let (height, background) = match (value_kind, str) {
            (ValueKind::HighImp, _) => (0.5, None),
            (ValueKind::Undef, _) => (0.5, None),
            (ValueKind::DontCare, _) => (0.5, None),
            (ValueKind::Warn, _) => (0.5, None),
            (ValueKind::Custom(_), _) => (0.5, None),
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
        };
        (height, color, background)
    }

    fn draw_digital_waveform(
        &self,
        values: &Vec<(f32, DigitalWaveformDrawRegion)>,
        color: Color32,
        offset: f32,
        height_scaling_factor: f32,
        draw_clock_marker: bool,
        ctx: &mut DrawingContext,
    ) {
        for ((old_x, prev_region), (new_x, new_region)) in values.iter().zip(values.iter().skip(1))
        {
            let trace_coords =
                |x, y| (ctx.to_screen)(x, y * ctx.cfg.line_height * height_scaling_factor + offset);

            let (old_height, old_color, old_bg) = Self::bool_drawing_spec(
                &prev_region.value.value,
                color,
                &self.config.theme,
                prev_region.value.kind,
            );
            let (new_height, _, _) = Self::bool_drawing_spec(
                &new_region.value.value,
                color,
                &self.config.theme,
                new_region.value.kind,
            );

            if let Some(old_bg) = old_bg {
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
                    CornerRadiusF32::ZERO,
                    old_bg,
                    Stroke::NONE,
                    egui::StrokeKind::Middle,
                ));
            }

            let stroke = Stroke {
                color: old_color,
                width: self.config.theme.linewidth,
            };

            if new_region.force_anti_alias {
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

    /// Draws a curvy arrow from `start` to `end`.
    fn draw_arrow(
        &self,
        start: Pos2,
        end: Pos2,
        arrowhead_angle: f64,
        color: Color32,
        ctx: &DrawingContext,
    ) {
        let mut anchor1 = Pos2::default();
        let mut anchor2 = Pos2::default();

        let x_diff = (end.x - start.x).max(100.);

        anchor1.x = start.x + (2. / 5.) * x_diff;
        anchor1.y = start.y;

        anchor2.x = end.x - (2. / 5.) * x_diff;
        anchor2.y = end.y;

        let stroke = PathStroke::new(1.3, color);

        ctx.painter.add(Shape::CubicBezier(CubicBezierShape {
            points: [start, anchor1, anchor2, end],
            closed: false,
            fill: Default::default(),
            stroke,
        }));

        let stroke = Stroke::new(1.3, color);
        self.draw_arrowheads(anchor2, end, arrowhead_angle, ctx, stroke);
    }

    /// Draws arrowheads for the vector going from `vec_start` to `vec_tip`.
    /// The `angle` has to be in degrees.
    fn draw_arrowheads(
        &self,
        vec_start: Pos2,
        vec_tip: Pos2,
        angle: f64, // given in degrees
        ctx: &DrawingContext,
        stroke: Stroke,
    ) {
        // FIXME: should probably make scale a function parameter
        let scale = 8.;

        let vec_x = (vec_tip.x - vec_start.x) as f64;
        let vec_y = (vec_tip.y - vec_start.y) as f64;

        let alpha = 2. * PI / 360. * angle;

        // calculate the points of the new vector, which forms an angle of the given degrees with the given vector
        let vec_angled_x = vec_x * alpha.cos() + vec_y * alpha.sin();
        let vec_angled_y = -vec_x * alpha.sin() + vec_y * alpha.cos();

        // scale the new vector to be ´scale´ long
        let vec_angled_x =
            (1. / (vec_angled_y - vec_angled_x).powi(2).sqrt()) * vec_angled_x * scale;
        let vec_angled_y =
            (1. / (vec_angled_y - vec_angled_x).powi(2).sqrt()) * vec_angled_y * scale;

        let arrowhead_left_x = vec_tip.x - vec_angled_x as f32;
        let arrowhead_left_y = vec_tip.y - vec_angled_y as f32;

        let arrowhead_right_x = vec_tip.x + vec_angled_y as f32;
        let arrowhead_right_y = vec_tip.y - vec_angled_x as f32;

        ctx.painter.add(Shape::line_segment(
            [vec_tip, Pos2::new(arrowhead_left_x, arrowhead_left_y)],
            stroke,
        ));

        ctx.painter.add(Shape::line_segment(
            [vec_tip, Pos2::new(arrowhead_right_x, arrowhead_right_y)],
            stroke,
        ));
    }

    fn handle_canvas_context_menu(
        &self,
        response: Response,
        waves: &WaveData,
        to_screen: RectTransform,
        ctx: &mut DrawingContext,
        msgs: &mut Vec<Message>,
        viewport_idx: usize,
    ) {
        let size = response.rect.size();
        response.context_menu(|ui| {
            let offset = ui.spacing().menu_margin.left as f32;
            let top_left = to_screen.inverse().transform_rect(ui.min_rect()).left_top()
                - Pos2 {
                    x: offset,
                    y: offset,
                };

            let snap_pos = self.snap_to_edge(Some(top_left.to_pos2()), waves, size.x, viewport_idx);

            if let Some(time) = snap_pos {
                self.draw_line(&time, ctx, size, &waves.viewports[viewport_idx], waves);
                ui.menu_button("Set marker", |ui| {
                    macro_rules! close_menu {
                        () => {{
                            ui.close_menu();
                        }};
                    }

                    for id in waves.markers.keys().sorted() {
                        ui.button(format!("{id}")).clicked().then(|| {
                            msgs.push(Message::SetMarker {
                                id: *id,
                                time: time.clone(),
                            });
                            close_menu!();
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
                            close_menu!();
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
    fn snap_to_edge(
        &self,
        pointer_pos_canvas: Option<Pos2>,
        waves: &WaveData,
        frame_width: f32,
        viewport_idx: usize,
    ) -> Option<BigInt> {
        let pos = pointer_pos_canvas?;
        let viewport = &waves.viewports[viewport_idx];
        let num_timestamps = waves.num_timestamps().unwrap_or(1.into());
        let timestamp = viewport.as_time_bigint(pos.x, frame_width, &num_timestamps);
        if let Some(utimestamp) = timestamp.to_biguint() {
            if let Some(vidx) = waves.get_item_at_y(pos.y) {
                if let Some(node) = waves.items_tree.get_visible(vidx) {
                    if let DisplayedItem::Variable(variable) =
                        &waves.displayed_items[&node.item_ref]
                    {
                        if let Ok(Some(res)) = waves
                            .inner
                            .as_waves()
                            .unwrap()
                            .query_variable(&variable.variable_ref, &utimestamp)
                        {
                            let prev_time = if let Some(v) = res.current {
                                v.0.to_bigint().unwrap()
                            } else {
                                BigInt::ZERO
                            };
                            let next_time = &res.next.unwrap_or_default().to_bigint().unwrap();
                            let prev =
                                viewport.pixel_from_time(&prev_time, frame_width, &num_timestamps);
                            let next =
                                viewport.pixel_from_time(next_time, frame_width, &num_timestamps);
                            if (prev - pos.x).abs() < (next - pos.x).abs() {
                                if (prev - pos.x).abs() <= self.config.snap_distance {
                                    return Some(prev_time.clone());
                                }
                            } else if (next - pos.x).abs() <= self.config.snap_distance {
                                return Some(next_time.clone());
                            }
                        }
                    }
                }
            }
        }
        Some(timestamp)
    }

    pub fn draw_line(
        &self,
        time: &BigInt,
        ctx: &mut DrawingContext,
        size: Vec2,
        viewport: &Viewport,
        waves: &WaveData,
    ) {
        let x = viewport.pixel_from_time(time, size.x, &waves.num_timestamps().unwrap_or(1.into()));

        let stroke = Stroke {
            color: self.config.theme.cursor.color,
            width: self.config.theme.cursor.width,
        };
        ctx.painter.line_segment(
            [
                (ctx.to_screen)(x + 0.5, -0.5),
                (ctx.to_screen)(x + 0.5, size.y),
            ],
            stroke,
        );
    }
}

impl WaveData {}

fn handle_transaction_tooltip(
    response: Response,
    waves: &WaveData,
    gen_ref: &TransactionStreamRef,
    tx_ref: &TransactionRef,
) -> Response {
    response
        .on_hover_ui(|ui| {
            let tx = waves
                .inner
                .as_transactions()
                .unwrap()
                .get_generator(gen_ref.gen_id.unwrap())
                .unwrap()
                .transactions
                .iter()
                .find(|transaction| transaction.get_tx_id() == tx_ref.id)
                .unwrap();

            ui.set_max_width(ui.spacing().tooltip_width);
            ui.add(egui::Label::new(transaction_tooltip_text(waves, tx)));
        })
        .on_hover_ui(|ui| {
            // Seemingly a bit redundant to determine tx twice, but since the
            // alternative is to do it every frame for every transaction, this
            // is most likely still a better approach.
            // Feel free to use some Rust magic to only do it once though...
            let tx = waves
                .inner
                .as_transactions()
                .unwrap()
                .get_generator(gen_ref.gen_id.unwrap())
                .unwrap()
                .transactions
                .iter()
                .find(|transaction| transaction.get_tx_id() == tx_ref.id)
                .unwrap();

            transaction_tooltip_table(ui, tx)
        })
}

fn transaction_tooltip_text(waves: &WaveData, tx: &Transaction) -> String {
    let time_scale = waves.inner.as_transactions().unwrap().inner.time_scale;

    format!(
        "tx#{}: {}{} - {}{}\nType: {}",
        tx.event.tx_id,
        tx.event.start_time,
        time_scale,
        tx.event.end_time,
        time_scale,
        waves
            .inner
            .as_transactions()
            .unwrap()
            .get_generator(tx.get_gen_id())
            .unwrap()
            .name
            .clone(),
    )
}

fn transaction_tooltip_table(ui: &mut Ui, tx: &Transaction) {
    TableBuilder::new(ui)
        .column(Column::exact(80.))
        .column(Column::exact(80.))
        .header(20.0, |mut header| {
            header.col(|ui| {
                ui.heading("Attribute");
            });
            header.col(|ui| {
                ui.heading("Value");
            });
        })
        .body(|body| {
            let total_rows = tx.attributes.len();
            let attributes = &tx.attributes;
            body.rows(15., total_rows, |mut row| {
                let attribute = attributes.get(row.index()).unwrap();
                row.col(|ui| {
                    ui.label(attribute.name.clone());
                });
                row.col(|ui| {
                    ui.label(attribute.value());
                });
            });
        });
}
