//! Analog signal rendering: command generation and waveform drawing.

use egui::{emath, Color32, Pos2, Stroke};
use epaint::PathShape;
use num::{BigInt, ToPrimitive};
use std::collections::HashMap;
use surfer_translation_types::ValueKind;

use crate::analog_signal_cache::{AnalogSignalCache, CacheQueryResult};
use crate::displayed_item::{
    AnalogSettings, DisplayedFieldRef, DisplayedItemRef, DisplayedVariable,
};
use crate::drawing_canvas::{AnalogDrawingCommands, DrawingCommands, VariableDrawCommands};
use crate::message::Message;
use crate::translation::{TranslatorList, ValueKindExt};
use crate::view::DrawingContext;
use crate::viewport::Viewport;
use crate::wave_data::WaveData;

pub struct AnalogDrawingCommand {
    pub start_px: f32,
    pub kind: CommandKind,
}

pub enum CommandKind {
    /// Constant value from start_px to end_px.
    Flat { value: f64, end_px: f32 },
    /// Multiple transitions in one pixel (anti-aliased).
    Range { min: f64, max: f64 },
}

/// Generate draw commands for a displayed analog variable.
/// Returns `None` if unrenderable, or a cache-build command if cache not ready.
pub(crate) fn variable_analog_draw_commands(
    displayed_variable: &DisplayedVariable,
    display_id: DisplayedItemRef,
    waves: &WaveData,
    translators: &TranslatorList,
    view_width: f32,
    viewport_idx: usize,
) -> Option<VariableDrawCommands> {
    let wave_container = waves.inner.as_waves()?;
    let displayed_field_ref: DisplayedFieldRef = display_id.into();
    let translator = waves.variable_translator(&displayed_field_ref, translators);
    let viewport = &waves.viewports[viewport_idx];
    let num_timestamps = waves.num_timestamps().unwrap_or(1.into());

    let signal_id = wave_container
        .signal_id(&displayed_variable.variable_ref)
        .ok()?;
    let translator_name = translator.name();
    let cache_key = (signal_id, translator_name.clone());

    let num_timestamps_u64 = num_timestamps.to_u64()?;
    let cache = match waves.analog_signal_caches.get(&cache_key) {
        Some(cache) if cache.num_timestamps == num_timestamps_u64 => cache,
        _ => {
            return Some(VariableDrawCommands {
                clock_edges: vec![],
                display_id,
                local_commands: HashMap::new(),
                local_msgs: vec![Message::BuildAnalogCache {
                    cache_key,
                    variable_ref: displayed_variable.variable_ref.clone(),
                }],
                used_cache_key: None,
            });
        }
    };

    let analog_commands = CommandBuilder::new(
        cache,
        viewport,
        &num_timestamps,
        view_width,
        displayed_variable.analog_settings,
    )
    .build();

    let mut local_commands = HashMap::new();
    local_commands.insert(vec![], DrawingCommands::Analog(analog_commands));

    Some(VariableDrawCommands {
        clock_edges: vec![],
        display_id,
        local_commands,
        local_msgs: vec![],
        used_cache_key: Some(cache_key),
    })
}

/// Render analog waveform from pre-computed commands.
pub fn draw_analog(
    analog_commands: &AnalogDrawingCommands,
    color: Color32,
    offset: f32,
    height_scaling_factor: f32,
    frame_width: f32,
    ctx: &mut DrawingContext,
) {
    let analog_settings = &analog_commands.analog_settings;
    debug_assert!(analog_settings.enabled, "draw_analog called when disabled");

    if !analog_commands.viewport_min.is_finite() || !analog_commands.viewport_max.is_finite() {
        return;
    }

    let (min_val, max_val) = select_value_range(analog_commands, analog_settings);

    let render_ctx = RenderContext::new(
        color,
        min_val,
        max_val,
        analog_commands,
        offset,
        height_scaling_factor,
        ctx,
    );

    // Use the appropriate strategy based on settings
    match analog_settings.render_style {
        crate::displayed_item::AnalogRenderStyle::Step => {
            let mut strategy = StepStrategy::default();
            render_with_strategy(&analog_commands.values, &render_ctx, &mut strategy, ctx);
        }
        crate::displayed_item::AnalogRenderStyle::Interpolated => {
            let mut strategy = InterpolatedStrategy::default();
            render_with_strategy(&analog_commands.values, &render_ctx, &mut strategy, ctx);
        }
    }

    draw_amplitude_labels(&render_ctx, frame_width, ctx);
}

fn select_value_range(cmds: &AnalogDrawingCommands, settings: &AnalogSettings) -> (f64, f64) {
    let (min, max) = match settings.y_axis_scale {
        crate::displayed_item::AnalogYAxisScale::Viewport => (cmds.viewport_min, cmds.viewport_max),
        crate::displayed_item::AnalogYAxisScale::Global => (cmds.global_min, cmds.global_max),
    };
    // Avoid division by zero
    if (min - max).abs() < f64::EPSILON {
        (min - 0.5, max + 0.5)
    } else {
        (min, max)
    }
}

/// Builds drawing commands by iterating viewport pixels.
struct CommandBuilder<'a> {
    cache: &'a AnalogSignalCache,
    viewport: &'a Viewport,
    num_timestamps: &'a BigInt,
    view_width: f32,
    min_valid_pixel: f32,
    max_valid_pixel: f32,
    output: CommandOutput,
    analog_settings: AnalogSettings,
}

/// Accumulates commands and tracks value bounds.
struct CommandOutput {
    commands: Vec<AnalogDrawingCommand>,
    pending_flat: Option<(f32, f64)>,
    viewport_min: f64,
    viewport_max: f64,
}

impl CommandOutput {
    fn new() -> Self {
        Self {
            commands: Vec::new(),
            pending_flat: None,
            viewport_min: f64::INFINITY,
            viewport_max: f64::NEG_INFINITY,
        }
    }

    fn update_bounds(&mut self, value: f64) {
        if value.is_finite() {
            self.viewport_min = self.viewport_min.min(value);
            self.viewport_max = self.viewport_max.max(value);
        }
    }

    fn emit_flat(&mut self, px: f32, value: f64) {
        match self.pending_flat {
            Some((_, v)) if values_equal(v, value) => {}
            Some((start, v)) => {
                self.commands.push(AnalogDrawingCommand {
                    start_px: start,
                    kind: CommandKind::Flat {
                        value: v,
                        end_px: px,
                    },
                });
                self.pending_flat = Some((px, value));
            }
            None => self.pending_flat = Some((px, value)),
        }
    }

    fn flush_flat(&mut self, end_px: f32) {
        if let Some((start, v)) = self.pending_flat.take() {
            self.commands.push(AnalogDrawingCommand {
                start_px: start,
                kind: CommandKind::Flat { value: v, end_px },
            });
        }
    }

    fn emit_range(&mut self, px: f32, min: f64, max: f64) {
        self.flush_flat(px);
        self.commands.push(AnalogDrawingCommand {
            start_px: px,
            kind: CommandKind::Range { min, max },
        });
    }
}

fn values_equal(a: f64, b: f64) -> bool {
    (a.is_nan() && b.is_nan()) || (a - b).abs() < f64::EPSILON
}

impl<'a> CommandBuilder<'a> {
    fn new(
        cache: &'a AnalogSignalCache,
        viewport: &'a Viewport,
        num_timestamps: &'a BigInt,
        view_width: f32,
        analog_settings: AnalogSettings,
    ) -> Self {
        let min_valid_pixel =
            viewport.pixel_from_time(&BigInt::from(0), view_width, num_timestamps);
        let max_valid_pixel = viewport.pixel_from_time(num_timestamps, view_width, num_timestamps);

        Self {
            cache,
            viewport,
            num_timestamps,
            view_width,
            min_valid_pixel,
            max_valid_pixel,
            output: CommandOutput::new(),
            analog_settings,
        }
    }

    fn build(mut self) -> AnalogDrawingCommands {
        let end_px = self.view_width.floor().max(0.0) + 1.0;

        let before_px = self.add_before_viewport_sample();
        self.iterate_pixels(0.0, end_px);
        self.add_after_viewport_sample(end_px);

        self.finalize(before_px)
    }

    fn time_at_pixel(&self, px: f64) -> u64 {
        self.viewport
            .as_absolute_time(px, self.view_width, self.num_timestamps)
            .0
            .to_u64()
            .unwrap_or(0)
    }

    fn pixel_at_time(&self, time: u64) -> f32 {
        self.viewport
            .pixel_from_time(&BigInt::from(time), self.view_width, self.num_timestamps)
    }

    fn query(&self, time: u64) -> CacheQueryResult {
        self.cache.query_at_time(time)
    }

    fn add_before_viewport_sample(&mut self) -> Option<f32> {
        let query = self.query(self.time_at_pixel(0.0));

        if let Some((time, value)) = query.current {
            let px = self.pixel_at_time(time);
            if px < 0.0 {
                self.output.update_bounds(value);
                self.output.pending_flat = Some((px, value));
                return Some(px);
            }
        }
        None
    }

    fn iterate_pixels(&mut self, start_px: f32, end_px: f32) {
        let mut px = start_px as u32;
        let end = end_px as u32;
        let mut next_query_time: Option<u64> = None;

        while px < end {
            let t0 = next_query_time.unwrap_or_else(|| self.time_at_pixel(px as f64));
            let t1 = self.time_at_pixel(px as f64 + 1.0);
            next_query_time = None;

            if t0 == t1 {
                px += 1;
                continue;
            }

            let query = self.query(t0);
            let next_change = query.next;
            let is_flat = next_change.is_none_or(|nc| nc >= t1);

            if is_flat {
                px = self.process_flat(px, end, &query, next_change, &mut next_query_time);
            } else {
                self.process_range(px, t0, t1);
                px += 1;
            }
        }
    }

    fn process_flat(
        &mut self,
        px: u32,
        end: u32,
        query: &CacheQueryResult,
        next_change: Option<u64>,
        next_query_time: &mut Option<u64>,
    ) -> u32 {
        if let Some((_, value)) = query.current {
            self.output.update_bounds(value);
            self.output.emit_flat(px as f32, value);
        }

        // Skip ahead to next transition
        if let Some(next) = next_change {
            let next_px = self.pixel_at_time(next);
            if next_px.is_finite() {
                let jump = next_px.floor().max(0.0) as u32;
                if jump > px {
                    *next_query_time = Some(next);
                    return jump.min(end);
                }
            }
            (px + 1).min(end)
        } else {
            end
        }
    }

    fn process_range(&mut self, px: u32, t0: u64, t1: u64) {
        if let Some((min, max)) = self.cache.query_time_range(t0, t1.saturating_sub(1)) {
            self.output.update_bounds(min);
            self.output.update_bounds(max);
            self.output.emit_range(px as f32, min, max);
        }
    }

    fn add_after_viewport_sample(&mut self, end_px: f32) {
        let query = self.query(self.time_at_pixel(end_px as f64));

        let Some(next_time) = query.next else {
            return;
        };

        let after_px = self.pixel_at_time(next_time);
        if after_px <= end_px {
            return;
        }

        let after_query = self.query(next_time);

        if let Some((_, value)) = after_query.current {
            self.output.update_bounds(value);

            if let Some((start, v)) = self.output.pending_flat.take() {
                self.output.commands.push(AnalogDrawingCommand {
                    start_px: start,
                    kind: CommandKind::Flat {
                        value: v,
                        end_px: after_px,
                    },
                });
                self.output.pending_flat = Some((after_px, value));
            }
        }
    }

    fn finalize(mut self, before_px: Option<f32>) -> AnalogDrawingCommands {
        // Flush remaining pending flat
        if let Some((start, v)) = self.output.pending_flat {
            self.output.commands.push(AnalogDrawingCommand {
                start_px: start,
                kind: CommandKind::Flat {
                    value: v,
                    end_px: self.max_valid_pixel,
                },
            });
        }

        // Extend first command to include before-viewport sample
        if let Some(before) = before_px {
            if let Some(first) = self.output.commands.first_mut() {
                first.start_px = first.start_px.min(before);
            }
        }

        // Ensure valid bounds
        if !self.output.viewport_min.is_finite() || !self.output.viewport_max.is_finite() {
            self.output.viewport_min = 0.0;
            self.output.viewport_max = 1.0;
        }

        AnalogDrawingCommands {
            viewport_min: self.output.viewport_min,
            viewport_max: self.output.viewport_max,
            global_min: self.cache.global_min,
            global_max: self.cache.global_max,
            values: self.output.commands,
            min_valid_pixel: self.min_valid_pixel,
            max_valid_pixel: self.max_valid_pixel,
            analog_settings: self.analog_settings,
        }
    }
}

/// Rendering strategy for analog waveforms.
pub trait RenderStrategy {
    /// Reset state after encountering undefined values.
    fn reset_state(&mut self);

    /// Render a flat segment with a finite value. Relies on egui for clamping.
    fn render_flat_defined(
        &mut self,
        ctx: &mut DrawingContext,
        render_ctx: &RenderContext,
        start_x: f32,
        end_x: f32,
        value: f64,
        next: Option<&AnalogDrawingCommand>,
    );

    /// Render a range segment with finite min/max. Relies on egui for clamping.
    fn render_range_defined(
        &mut self,
        ctx: &mut DrawingContext,
        render_ctx: &RenderContext,
        x: f32,
        min: f64,
        max: f64,
        next: Option<&AnalogDrawingCommand>,
    );

    /// Render a flat segment with undefined value handling.
    fn render_flat(
        &mut self,
        ctx: &mut DrawingContext,
        render_ctx: &RenderContext,
        start_x: f32,
        end_x: f32,
        value: f64,
        next: Option<&AnalogDrawingCommand>,
    ) {
        if !value.is_finite() {
            render_ctx.draw_undefined(start_x, end_x, ctx);
            self.reset_state();
            return;
        }

        self.render_flat_defined(ctx, render_ctx, start_x, end_x, value, next);
    }

    /// Render a range segment with undefined value handling.
    fn render_range(
        &mut self,
        ctx: &mut DrawingContext,
        render_ctx: &RenderContext,
        x: f32,
        min: f64,
        max: f64,
        next: Option<&AnalogDrawingCommand>,
    ) {
        if !min.is_finite() || !max.is_finite() {
            render_ctx.draw_undefined(x, x + 1.0, ctx);
            self.reset_state();
            return;
        }

        self.render_range_defined(ctx, render_ctx, x, min, max, next);
    }
}

/// Shared rendering context.
pub struct RenderContext {
    pub stroke: Stroke,
    pub min_val: f64,
    pub max_val: f64,
    /// Pixel position of timestamp 0 (start of signal data).
    pub min_valid_pixel: f32,
    /// Pixel position of last timestamp (end of signal data).
    pub max_valid_pixel: f32,
    pub offset: f32,
    pub height_scale: f32,
    pub line_height: f32,
}

impl RenderContext {
    fn new(
        color: Color32,
        min_val: f64,
        max_val: f64,
        analog_commands: &AnalogDrawingCommands,
        offset: f32,
        height_scale: f32,
        ctx: &DrawingContext,
    ) -> Self {
        Self {
            stroke: Stroke::new(ctx.theme.linewidth, color),
            min_val,
            max_val,
            min_valid_pixel: analog_commands.min_valid_pixel,
            max_valid_pixel: analog_commands.max_valid_pixel,
            offset,
            height_scale,
            line_height: ctx.cfg.line_height,
        }
    }

    /// Normalize value to [0, 1].
    pub fn normalize(&self, value: f64) -> f32 {
        let range = self.max_val - self.min_val;
        if range.abs() > f64::EPSILON {
            ((value - self.min_val) / range) as f32
        } else {
            0.5
        }
    }

    /// Convert normalized coordinates to screen position.
    pub fn to_screen(&self, x: f32, y_norm: f32, ctx: &DrawingContext) -> Pos2 {
        (ctx.to_screen)(
            x,
            (1.0 - y_norm) * self.line_height * self.height_scale + self.offset,
        )
    }

    pub fn draw_line(&self, from: Pos2, to: Pos2, ctx: &mut DrawingContext) {
        ctx.painter
            .add(PathShape::line(vec![from, to], self.stroke));
    }

    pub fn draw_undefined(&self, start_x: f32, end_x: f32, ctx: &mut DrawingContext) {
        let color = ValueKind::Undef.color(self.stroke.color, ctx.theme);
        let min = (ctx.to_screen)(start_x, self.offset);
        let max = (ctx.to_screen)(end_x, self.offset + self.line_height * self.height_scale);
        ctx.painter
            .rect_filled(egui::Rect::from_min_max(min, max), 0.0, color);
    }
}

/// Step-style rendering: horizontal segments with vertical transitions.
#[derive(Default)]
pub struct StepStrategy {
    last_point: Option<Pos2>,
}


impl RenderStrategy for StepStrategy {
    fn reset_state(&mut self) {
        self.last_point = None;
    }

    fn render_flat_defined(
        &mut self,
        ctx: &mut DrawingContext,
        render_ctx: &RenderContext,
        start_x: f32,
        end_x: f32,
        value: f64,
        _next: Option<&AnalogDrawingCommand>,
    ) {
        let norm = render_ctx.normalize(value);
        let p1 = render_ctx.to_screen(start_x, norm, ctx);
        let p2 = render_ctx.to_screen(end_x, norm, ctx);

        if let Some(prev) = self.last_point {
            render_ctx.draw_line(Pos2::new(p1.x, prev.y), p1, ctx);
        }

        render_ctx.draw_line(p1, p2, ctx);
        self.last_point = Some(p2);
    }

    fn render_range_defined(
        &mut self,
        ctx: &mut DrawingContext,
        render_ctx: &RenderContext,
        x: f32,
        min: f64,
        max: f64,
        _next: Option<&AnalogDrawingCommand>,
    ) {
        let p_min = render_ctx.to_screen(x, render_ctx.normalize(min), ctx);
        let p_max = render_ctx.to_screen(x, render_ctx.normalize(max), ctx);

        let (connect, other) = match self.last_point {
            Some(prev) if (prev.y - p_min.y).abs() < (prev.y - p_max.y).abs() => (p_min, p_max),
            _ => (p_max, p_min),
        };

        if let Some(prev) = self.last_point {
            let mid = Pos2::new(connect.x, prev.y);
            render_ctx.draw_line(prev, mid, ctx);
            render_ctx.draw_line(mid, connect, ctx);
        }

        render_ctx.draw_line(connect, other, ctx);

        self.last_point = Some(other);
    }
}

/// Interpolated rendering: diagonal lines connecting consecutive values.
#[derive(Default)]
pub struct InterpolatedStrategy {
    last: Option<(Pos2, f64)>,
    started: bool,
}


impl RenderStrategy for InterpolatedStrategy {
    fn reset_state(&mut self) {
        self.last = None;
        self.started = true;
    }

    fn render_flat_defined(
        &mut self,
        ctx: &mut DrawingContext,
        render_ctx: &RenderContext,
        start_x: f32,
        end_x: f32,
        value: f64,
        next: Option<&AnalogDrawingCommand>,
    ) {
        let norm = render_ctx.normalize(value);
        let current = render_ctx.to_screen(start_x, norm, ctx);

        if let Some((prev, _)) = self.last {
            render_ctx.draw_line(prev, current, ctx);
        } else if !self.started {
            let edge = render_ctx.to_screen(render_ctx.min_valid_pixel.max(0.0), norm, ctx);
            render_ctx.draw_line(edge, current, ctx);
        }

        if let Some(next_cmd) = next {
            if let CommandKind::Flat {
                value: next_val, ..
            } = &next_cmd.kind
            {
                if next_val.is_nan() {
                    render_ctx.draw_line(
                        current,
                        render_ctx.to_screen(next_cmd.start_px, norm, ctx),
                        ctx,
                    );
                }
            }
        } else if end_x > start_x {
            let endpoint = render_ctx.to_screen(end_x, norm, ctx);
            render_ctx.draw_line(current, endpoint, ctx);
        }

        self.last = Some((current, value));
        self.started = true;
    }

    fn render_range_defined(
        &mut self,
        ctx: &mut DrawingContext,
        render_ctx: &RenderContext,
        x: f32,
        min: f64,
        max: f64,
        _next: Option<&AnalogDrawingCommand>,
    ) {
        let min_norm = render_ctx.normalize(min);
        let max_norm = render_ctx.normalize(max);
        let p_min = render_ctx.to_screen(x, min_norm, ctx);
        let p_max = render_ctx.to_screen(x, max_norm, ctx);

        let (first, second, second_val) = if let Some((prev, _)) = self.last {
            let prev_norm =
                1.0 - (prev.y - render_ctx.offset) / (render_ctx.line_height * render_ctx.height_scale);

            let go_max_first = prev_norm < min_norm
                || (prev_norm <= max_norm
                    && (prev_norm - max_norm).abs() < (prev_norm - min_norm).abs());

            if go_max_first {
                render_ctx.draw_line(prev, p_max, ctx);
                (p_max, p_min, min)
            } else {
                render_ctx.draw_line(prev, p_min, ctx);
                (p_min, p_max, max)
            }
        } else if !self.started {
            let edge = render_ctx.to_screen(render_ctx.min_valid_pixel, max_norm, ctx);
            render_ctx.draw_line(edge, p_max, ctx);
            (p_max, p_min, min)
        } else {
            (p_max, p_min, min)
        };

        render_ctx.draw_line(first, second, ctx);
        self.last = Some((second, second_val));
        self.started = true;
    }
}

/// Render commands using the given strategy.
fn render_with_strategy<S: RenderStrategy>(
    commands: &[AnalogDrawingCommand],
    render_ctx: &RenderContext,
    strategy: &mut S,
    ctx: &mut DrawingContext,
) {
    for (i, cmd) in commands.iter().enumerate() {
        let next = commands.get(i + 1);
        match &cmd.kind {
            CommandKind::Flat { value, end_px } => {
                strategy.render_flat(ctx, render_ctx, cmd.start_px, *end_px, *value, next);
            }
            CommandKind::Range { min, max } => {
                strategy.render_range(ctx, render_ctx, cmd.start_px, *min, *max, next);
            }
        }
    }
}

fn draw_amplitude_labels(
    render_ctx: &RenderContext,
    frame_width: f32,
    ctx: &mut DrawingContext,
) {
    const SPLIT_LABEL_HEIGHT_THRESHOLD: f32 = 2.0;
    const LABEL_ALPHA: f32 = 0.7;
    const BACKGROUND_ALPHA: u8 = 200;

    let text_size = ctx.cfg.text_size;

    let text_color = render_ctx.stroke.color.gamma_multiply(LABEL_ALPHA);
    let bg_color = Color32::from_rgba_unmultiplied(0, 0, 0, BACKGROUND_ALPHA);
    let font = egui::FontId::monospace(text_size);

    if render_ctx.height_scale < SPLIT_LABEL_HEIGHT_THRESHOLD {
        let combined_text = format!("[{:.2}, {:.2}]", render_ctx.min_val, render_ctx.max_val);
        let galley = ctx
            .painter
            .layout_no_wrap(combined_text.clone(), font.clone(), text_color);

        let label_x = frame_width - galley.size().x - 5.0;
        let label_pos = render_ctx.to_screen(label_x, 0.5, ctx);

        let rect = egui::Rect::from_min_size(
            Pos2::new(label_pos.x - 2.0, label_pos.y - galley.size().y / 2.0 - 2.0),
            egui::Vec2::new(galley.size().x + 4.0, galley.size().y + 4.0),
        );
        ctx.painter.rect_filled(rect, 2.0, bg_color);
        ctx.painter.text(
            Pos2::new(label_pos.x, label_pos.y - galley.size().y / 2.0),
            emath::Align2::LEFT_TOP,
            combined_text,
            font,
            text_color,
        );
    } else {
        let max_text = format!("{:.2}", render_ctx.max_val);
        let min_text = format!("{:.2}", render_ctx.min_val);

        let max_galley = ctx
            .painter
            .layout_no_wrap(max_text.clone(), font.clone(), text_color);
        let min_galley = ctx
            .painter
            .layout_no_wrap(min_text.clone(), font.clone(), text_color);

        let label_x = frame_width - max_galley.size().x.max(min_galley.size().x) - 5.0;

        let max_pos = render_ctx.to_screen(label_x, 1.0, ctx);
        let max_rect = egui::Rect::from_min_size(
            Pos2::new(max_pos.x - 2.0, max_pos.y - 2.0),
            egui::Vec2::new(max_galley.size().x + 4.0, max_galley.size().y + 4.0),
        );
        ctx.painter.rect_filled(max_rect, 2.0, bg_color);
        ctx.painter.text(
            max_pos,
            emath::Align2::LEFT_TOP,
            max_text,
            font.clone(),
            text_color,
        );

        let min_pos = render_ctx.to_screen(label_x, 0.0, ctx);
        let min_rect = egui::Rect::from_min_size(
            Pos2::new(min_pos.x - 2.0, min_pos.y - min_galley.size().y - 2.0),
            egui::Vec2::new(min_galley.size().x + 4.0, min_galley.size().y + 4.0),
        );
        ctx.painter.rect_filled(min_rect, 2.0, bg_color);
        ctx.painter.text(
            min_pos,
            emath::Align2::LEFT_BOTTOM,
            min_text,
            font,
            text_color,
        );
    }
}
