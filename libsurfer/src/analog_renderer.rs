//! Analog signal rendering for waveform visualization.
//!
//! This module provides:
//! - Command generation: Converting analog signal data into drawing commands
//! - Rendering: Drawing analog waveforms in step or interpolated style

use egui::{emath, Color32, Pos2, Stroke};
use epaint::PathShape;
use eyre::Result;
use num::{BigInt, ToPrimitive};
use std::collections::HashMap;
use surfer_translation_types::ValueKind;
use tracing::warn;

use crate::analog_signal_cache::AnalogSignalCache;
use crate::displayed_item::{AnalogSettings, DisplayedFieldRef, DisplayedItemRef, DisplayedVariable};
use crate::drawing_canvas::{AnalogDrawingCommands, DrawingCommands, VariableDrawCommands};
use crate::message::Message;
use crate::translation::{DynTranslator, TranslatorList, ValueKindExt};
use crate::view::DrawingContext;
use crate::viewport::Viewport;
use crate::wave_container::{QueryResult, VariableMeta, VariableRef, WaveContainer};
use crate::wave_data::WaveData;

// ============================================================================
// Types
// ============================================================================

/// A single drawing command for analog signal visualization.
#[derive(Debug)]
pub struct AnalogDrawingCommand {
    pub start_px: f32,
    pub kind: CommandKind,
}

/// The kind of drawing to perform at a pixel position.
#[derive(Debug)]
pub enum CommandKind {
    /// Single value spanning from start_px to end_px.
    Flat { value: f64, end_px: f32 },
    /// Multiple transitions compressed into one pixel (anti-aliasing).
    Range { min: f64, max: f64 },
}

/// Configuration for analog signal rendering.
#[derive(Debug, Clone)]
pub struct AnalogRenderConfig {
    pub line_width_multiplier: f32,
    pub text_size_multiplier_threshold: f32,
    pub text_size_multipliers: (f32, f32),
    pub label_alpha: f32,
    pub background_alpha: u8,
}

impl Default for AnalogRenderConfig {
    fn default() -> Self {
        Self {
            line_width_multiplier: 1.5,
            text_size_multiplier_threshold: 1.0,
            text_size_multipliers: (0.5, 1.0),
            label_alpha: 0.7,
            background_alpha: 200,
        }
    }
}

// ============================================================================
// Public API
// ============================================================================

/// Generate analog draw commands for a displayed variable.
///
/// Returns `None` if the variable cannot be rendered (missing data, cache not ready, etc.).
/// When the cache is not ready, returns a command to build it asynchronously.
pub(crate) fn variable_analog_draw_commands(
    displayed_variable: &DisplayedVariable,
    display_id: DisplayedItemRef,
    waves: &WaveData,
    translators: &TranslatorList,
    view_width: f32,
    viewport_idx: usize,
) -> Option<VariableDrawCommands> {
    let wave_container = waves.inner.as_waves()?;
    let meta = wave_container.variable_meta(&displayed_variable.variable_ref).ok()?;
    let displayed_field_ref: DisplayedFieldRef = display_id.into();
    let translator = waves.variable_translator(&displayed_field_ref, translators);
    let viewport = &waves.viewports[viewport_idx];
    let num_timestamps = waves.num_timestamps().unwrap_or(1.into());

    let signal_ref = wave_container.get_signal_ref(&displayed_variable.variable_ref).ok()?;
    let translator_name = translator.name();
    let cache_key = (signal_ref, translator_name.clone());

    let num_timestamps_u64 = num_timestamps.to_u64()?;
    let cache = match waves.analog_signal_caches.get(&cache_key) {
        Some(cache) if cache.num_timestamps == num_timestamps_u64 => cache,
        _ => {
            return Some(VariableDrawCommands {
                clock_edges: vec![],
                display_id,
                local_commands: HashMap::new(),
                local_msgs: vec![Message::BuildAnalogCache {
                    signal_ref: cache_key.0,
                    translator_name,
                    variable_ref: displayed_variable.variable_ref.clone(),
                }],
                used_cache_key: None,
            });
        }
    };

    let analog_commands = CommandBuilder::new(
        wave_container,
        cache,
        &displayed_variable.variable_ref,
        translator,
        &meta,
        viewport,
        &num_timestamps,
        view_width,
    )
    .build();

    let analog_commands = match analog_commands {
        Ok(cmds) => cmds,
        Err(e) => {
            warn!("Failed to build analog drawing commands: {e:?}");
            return None;
        }
    };

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

/// Render analog waveform from pre-computed drawing commands.
pub fn draw_analog(
    commands: &DrawingCommands,
    color: Color32,
    offset: f32,
    height_scaling_factor: f32,
    analog_settings: &AnalogSettings,
    frame_width: f32,
    ctx: &mut DrawingContext,
) {
    debug_assert!(analog_settings.enabled, "draw_analog called when disabled");

    let analog_commands = match commands {
        DrawingCommands::Analog(a) => a,
        DrawingCommands::Digital(_) => return,
    };

    if !analog_commands.viewport_min.is_finite() || !analog_commands.viewport_max.is_finite() {
        return;
    }

    let (min_val, max_val) = select_value_range(analog_commands, analog_settings);
    let config = AnalogRenderConfig::default();

    let renderer = Renderer::new(color, &config, min_val, max_val, analog_commands, offset, height_scaling_factor, ctx);

    match analog_settings.render_style {
        crate::displayed_item::AnalogRenderStyle::Step => renderer.render_step(ctx),
        crate::displayed_item::AnalogRenderStyle::Interpolated => renderer.render_interpolated(ctx),
    }

    draw_amplitude_labels(color, offset, height_scaling_factor, min_val, max_val, frame_width, ctx, &config);
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

// ============================================================================
// Command Building
// ============================================================================

/// Builds drawing commands by iterating through viewport pixels.
struct CommandBuilder<'a> {
    // Input references
    wave_container: &'a WaveContainer,
    cache: &'a AnalogSignalCache,
    variable: &'a VariableRef,
    translator: &'a DynTranslator,
    meta: &'a VariableMeta,
    viewport: &'a Viewport,
    num_timestamps: &'a BigInt,
    view_width: f32,

    // Pixel bounds
    min_valid_pixel: f32,
    max_valid_pixel: f32,

    // Output accumulator
    output: CommandOutput,
}

/// Accumulates commands and tracks bounds during building.
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
                    kind: CommandKind::Flat { value: v, end_px: px },
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
    #[allow(clippy::too_many_arguments)]
    fn new(
        wave_container: &'a WaveContainer,
        cache: &'a AnalogSignalCache,
        variable: &'a VariableRef,
        translator: &'a DynTranslator,
        meta: &'a VariableMeta,
        viewport: &'a Viewport,
        num_timestamps: &'a BigInt,
        view_width: f32,
    ) -> Self {
        let min_valid_pixel = viewport.pixel_from_time(&BigInt::from(0), view_width, num_timestamps);
        let max_valid_pixel = viewport.pixel_from_time(num_timestamps, view_width, num_timestamps);

        Self {
            wave_container,
            cache,
            variable,
            translator,
            meta,
            viewport,
            num_timestamps,
            view_width,
            min_valid_pixel,
            max_valid_pixel,
            output: CommandOutput::new(),
        }
    }

    fn build(mut self) -> Result<AnalogDrawingCommands> {
        let end_px = self.view_width.floor().max(0.0) + 1.0;

        let before_px = self.add_before_viewport_sample()?;
        self.iterate_pixels(0.0, end_px)?;
        self.add_after_viewport_sample(end_px)?;

        self.finalize(before_px)
    }

    // --- Coordinate conversion ---

    fn time_at_pixel(&self, px: f64) -> u64 {
        self.viewport
            .as_absolute_time(px, self.view_width, self.num_timestamps)
            .0
            .to_u64()
            .unwrap_or(0)
    }

    fn pixel_at_time(&self, time: u64) -> f32 {
        self.viewport.pixel_from_time(&BigInt::from(time), self.view_width, self.num_timestamps)
    }

    // --- Data access ---

    fn query(&self, time: u64) -> Result<Option<QueryResult>> {
        self.wave_container.query_variable(self.variable, &num::BigUint::from(time))
    }

    fn translate(&self, raw: &surfer_translation_types::VariableValue) -> f64 {
        crate::analog_signal_cache::translate_to_numeric(self.translator, self.meta, raw).unwrap_or(f64::NAN)
    }

    // --- Building phases ---

    fn add_before_viewport_sample(&mut self) -> Result<Option<f32>> {
        let query = match self.query(self.time_at_pixel(0.0))? {
            Some(q) => q,
            None => return Ok(None),
        };

        if let Some((time, raw)) = &query.current {
            let px = self.pixel_at_time(time.to_u64().unwrap_or(0));
            if px < 0.0 {
                let value = self.translate(raw);
                self.output.update_bounds(value);
                self.output.pending_flat = Some((px, value));
                return Ok(Some(px));
            }
        }
        Ok(None)
    }

    fn iterate_pixels(&mut self, start_px: f32, end_px: f32) -> Result<()> {
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

            let query = match self.query(t0)? {
                Some(q) => q,
                None => {
                    px += 1;
                    continue;
                }
            };

            let next_change = query.next.as_ref().and_then(|n| n.to_u64());
            let is_flat = next_change.is_none_or(|nc| nc >= t1);

            if is_flat {
                px = self.process_flat(px, end, &query, next_change, &mut next_query_time);
            } else {
                self.process_range(px, t0, t1);
                px += 1;
            }
        }
        Ok(())
    }

    fn process_flat(
        &mut self,
        px: u32,
        end: u32,
        query: &QueryResult,
        next_change: Option<u64>,
        next_query_time: &mut Option<u64>,
    ) -> u32 {
        if let Some((_, raw)) = &query.current {
            let value = self.translate(raw);
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

    fn add_after_viewport_sample(&mut self, end_px: f32) -> Result<()> {
        let query = match self.query(self.time_at_pixel(end_px as f64))? {
            Some(q) => q,
            None => return Ok(()),
        };

        let next_time = match query.next.as_ref().and_then(|n| n.to_u64()) {
            Some(t) => t,
            None => return Ok(()),
        };

        let after_px = self.pixel_at_time(next_time);
        if after_px <= end_px {
            return Ok(());
        }

        let after_query = match self.query(next_time)? {
            Some(q) => q,
            None => return Ok(()),
        };

        if let Some((_, raw)) = &after_query.current {
            let value = self.translate(raw);
            self.output.update_bounds(value);

            if let Some((start, v)) = self.output.pending_flat.take() {
                self.output.commands.push(AnalogDrawingCommand {
                    start_px: start,
                    kind: CommandKind::Flat { value: v, end_px: after_px },
                });
                self.output.pending_flat = Some((after_px, value));
            }
        }
        Ok(())
    }

    fn finalize(mut self, before_px: Option<f32>) -> Result<AnalogDrawingCommands> {
        // Flush remaining pending flat
        if let Some((start, v)) = self.output.pending_flat {
            self.output.commands.push(AnalogDrawingCommand {
                start_px: start,
                kind: CommandKind::Flat { value: v, end_px: self.max_valid_pixel },
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

        Ok(AnalogDrawingCommands {
            viewport_min: self.output.viewport_min,
            viewport_max: self.output.viewport_max,
            global_min: self.cache.global_min,
            global_max: self.cache.global_max,
            values: self.output.commands,
            min_valid_pixel: self.min_valid_pixel,
            max_valid_pixel: self.max_valid_pixel,
        })
    }
}

// ============================================================================
// Rendering
// ============================================================================

/// Pre-computed state for rendering analog waveforms.
struct Renderer<'a> {
    stroke: Stroke,
    value_range: f64,
    min_val: f64,
    min_valid_pixel: f32,
    max_valid_pixel: f32,
    offset: f32,
    height_scale: f32,
    line_height: f32,
    commands: &'a [AnalogDrawingCommand],
}

impl<'a> Renderer<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        color: Color32,
        config: &AnalogRenderConfig,
        min_val: f64,
        max_val: f64,
        analog_commands: &'a AnalogDrawingCommands,
        offset: f32,
        height_scale: f32,
        ctx: &DrawingContext,
    ) -> Self {
        Self {
            stroke: Stroke::new(ctx.theme.linewidth * config.line_width_multiplier, color),
            value_range: max_val - min_val,
            min_val,
            min_valid_pixel: analog_commands.min_valid_pixel,
            max_valid_pixel: analog_commands.max_valid_pixel,
            offset,
            height_scale,
            line_height: ctx.cfg.line_height,
            commands: &analog_commands.values,
        }
    }

    // --- Coordinate helpers ---

    fn normalize(&self, value: f64) -> f32 {
        if self.value_range.abs() > f64::EPSILON {
            ((value - self.min_val) / self.value_range) as f32
        } else {
            0.5
        }
    }

    fn to_screen(&self, x: f32, y_norm: f32, ctx: &DrawingContext) -> Pos2 {
        (ctx.to_screen)(x, (1.0 - y_norm) * self.line_height * self.height_scale + self.offset)
    }

    fn clamp_x(&self, x: f32) -> f32 {
        x.clamp(self.min_valid_pixel, self.max_valid_pixel)
    }

    fn is_visible(&self, x: f32) -> bool {
        x >= self.min_valid_pixel && x < self.max_valid_pixel
    }

    // --- Drawing primitives ---

    fn draw_line(&self, from: Pos2, to: Pos2, ctx: &mut DrawingContext) {
        ctx.painter.add(PathShape::line(vec![from, to], self.stroke));
    }

    fn draw_undefined(&self, start_x: f32, end_x: f32, ctx: &mut DrawingContext) {
        let color = ValueKind::Undef.color(self.stroke.color, ctx.theme);
        let min = (ctx.to_screen)(start_x, self.offset);
        let max = (ctx.to_screen)(end_x, self.offset + self.line_height * self.height_scale);
        ctx.painter.rect_filled(egui::Rect::from_min_max(min, max), 0.0, color);
    }

    // --- Rendering styles ---

    fn render_step(self, ctx: &mut DrawingContext) {
        let mut last: Option<Pos2> = None;

        for cmd in self.commands {
            match &cmd.kind {
                CommandKind::Flat { value, end_px } => {
                    last = self.render_step_flat(cmd.start_px, *end_px, *value, last, ctx);
                }
                CommandKind::Range { min, max } => {
                    last = self.render_step_range(cmd.start_px, *min, *max, last, ctx);
                }
            }
        }
    }

    fn render_step_flat(
        &self,
        start: f32,
        end: f32,
        value: f64,
        last: Option<Pos2>,
        ctx: &mut DrawingContext,
    ) -> Option<Pos2> {
        let start_x = self.clamp_x(start);
        let end_x = self.clamp_x(end);

        if start_x >= self.max_valid_pixel || end_x <= self.min_valid_pixel {
            return last;
        }

        if !value.is_finite() {
            self.draw_undefined(start_x, end_x, ctx);
            return None;
        }

        let norm = self.normalize(value);
        let p1 = self.to_screen(start_x, norm, ctx);
        let p2 = self.to_screen(end_x, norm, ctx);

        // Vertical transition from previous point
        if let Some(prev) = last {
            if (prev.y - p1.y).abs() > 1.0 {
                self.draw_line(Pos2::new(p1.x, prev.y), p1, ctx);
            }
        }

        // Horizontal segment
        self.draw_line(p1, p2, ctx);
        Some(p2)
    }

    fn render_step_range(
        &self,
        start: f32,
        min: f64,
        max: f64,
        last: Option<Pos2>,
        ctx: &mut DrawingContext,
    ) -> Option<Pos2> {
        let x = self.clamp_x(start);
        if !self.is_visible(x) {
            return last;
        }

        if !min.is_finite() || !max.is_finite() {
            self.draw_undefined(x, (x + 1.0).min(self.max_valid_pixel), ctx);
            return None;
        }

        let p_min = self.to_screen(x, self.normalize(min), ctx);
        let p_max = self.to_screen(x, self.normalize(max), ctx);

        // Choose which end to connect to previous point
        let (connect, other) = match last {
            Some(prev) if (prev.y - p_min.y).abs() < (prev.y - p_max.y).abs() => (p_min, p_max),
            _ => (p_max, p_min),
        };

        // Draw connection from previous point
        if let Some(prev) = last {
            let mid = Pos2::new(connect.x, prev.y);
            self.draw_line(prev, mid, ctx);
            if (mid.y - connect.y).abs() > 1.0 {
                self.draw_line(mid, connect, ctx);
            }
        }

        // Draw vertical range line
        if (other.y - connect.y).abs() > 1.0 {
            self.draw_line(connect, other, ctx);
        }

        Some(other)
    }

    fn render_interpolated(self, ctx: &mut DrawingContext) {
        let mut last: Option<(Pos2, f64)> = None;
        let mut started = false;

        for (i, cmd) in self.commands.iter().enumerate() {
            match &cmd.kind {
                CommandKind::Flat { value, end_px } => {
                    let next_cmd = self.commands.get(i + 1);
                    let result = self.render_interp_flat(cmd.start_px, *end_px, *value, last, started, next_cmd, ctx);
                    last = result.0;
                    started = result.1;
                }
                CommandKind::Range { min, max } => {
                    let result = self.render_interp_range(cmd.start_px, *min, *max, last, started, ctx);
                    last = result.0;
                    started = result.1;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_interp_flat(
        &self,
        start: f32,
        end: f32,
        value: f64,
        last: Option<(Pos2, f64)>,
        started: bool,
        next_cmd: Option<&AnalogDrawingCommand>,
        ctx: &mut DrawingContext,
    ) -> (Option<(Pos2, f64)>, bool) {
        let start_x = self.clamp_x(start);
        let end_x = self.clamp_x(end);

        if start_x >= self.max_valid_pixel || end_x <= self.min_valid_pixel {
            return (last, started);
        }

        if !value.is_finite() {
            self.draw_undefined(start_x, end_x, ctx);
            return (None, true);
        }

        let norm = self.normalize(value);
        let current = self.to_screen(start_x, norm, ctx);

        // Connect from previous point
        if let Some((prev, prev_val)) = last {
            if start < self.min_valid_pixel && prev.x < start_x {
                // Interpolate at edge
                let edge = self.to_screen(self.min_valid_pixel, self.normalize(prev_val), ctx);
                self.draw_line(edge, current, ctx);
            } else {
                self.draw_line(prev, current, ctx);
            }
        } else if !started {
            // First point - draw from left edge
            let edge = self.to_screen(self.min_valid_pixel.max(0.0), norm, ctx);
            if (edge.x - current.x).abs() > 0.5 {
                self.draw_line(edge, current, ctx);
            }
        }

        // Handle trailing segment before undefined
        if let Some(next) = next_cmd {
            if let CommandKind::Flat { value: next_val, .. } = &next.kind {
                if next_val.is_nan() {
                    let target = self.clamp_x(next.start_px).min(self.max_valid_pixel);
                    if (current.x - target).abs() > 0.5 {
                        self.draw_line(current, self.to_screen(target, norm, ctx), ctx);
                    }
                }
            }
        } else if end_x > start_x {
            // Last command - extend to end
            let endpoint = self.to_screen(end_x.min(self.max_valid_pixel), norm, ctx);
            if (current.x - endpoint.x).abs() > 0.5 {
                self.draw_line(current, endpoint, ctx);
            }
        }

        (Some((current, value)), true)
    }

    fn render_interp_range(
        &self,
        start: f32,
        min: f64,
        max: f64,
        last: Option<(Pos2, f64)>,
        started: bool,
        ctx: &mut DrawingContext,
    ) -> (Option<(Pos2, f64)>, bool) {
        let x = self.clamp_x(start);
        if !self.is_visible(x) {
            return (last, started);
        }

        if !min.is_finite() || !max.is_finite() {
            self.draw_undefined(x, (x + 1.0).min(self.max_valid_pixel), ctx);
            return (None, true);
        }

        let min_norm = self.normalize(min);
        let max_norm = self.normalize(max);
        let p_min = self.to_screen(x, min_norm, ctx);
        let p_max = self.to_screen(x, max_norm, ctx);

        let (first, second, second_val) = if let Some((prev, _)) = last {
            // Determine direction based on previous position
            let prev_norm = (prev.y - self.offset) / (self.line_height * self.height_scale);
            let prev_norm = 1.0 - prev_norm;

            let go_max_first = prev_norm < min_norm || (prev_norm <= max_norm && (prev_norm - max_norm).abs() < (prev_norm - min_norm).abs());

            if go_max_first {
                self.draw_line(prev, p_max, ctx);
                (p_max, p_min, min)
            } else {
                self.draw_line(prev, p_min, ctx);
                (p_min, p_max, max)
            }
        } else if !started {
            // First point - start from edge
            let edge = self.to_screen(self.min_valid_pixel, max_norm, ctx);
            self.draw_line(edge, p_max, ctx);
            (p_max, p_min, min)
        } else {
            (p_max, p_min, min)
        };

        // Draw the range line
        self.draw_line(first, second, ctx);
        (Some((second, second_val)), true)
    }
}

// ============================================================================
// Labels
// ============================================================================

#[allow(clippy::too_many_arguments)]
fn draw_amplitude_labels(
    color: Color32,
    offset: f32,
    height_scale: f32,
    min_val: f64,
    max_val: f64,
    frame_width: f32,
    ctx: &mut DrawingContext,
    config: &AnalogRenderConfig,
) {
    let to_screen = |x, y_norm: f32| {
        (ctx.to_screen)(x, (1.0 - y_norm) * ctx.cfg.line_height * height_scale + offset)
    };

    let text_size = ctx.cfg.text_size
        * if height_scale <= config.text_size_multiplier_threshold {
            config.text_size_multipliers.0
        } else {
            config.text_size_multipliers.1
        };

    let text_color = color.gamma_multiply(config.label_alpha);
    let bg_color = Color32::from_rgba_unmultiplied(0, 0, 0, config.background_alpha);
    let font = egui::FontId::monospace(text_size);

    let max_text = format!("max: {max_val:.2}");
    let min_text = format!("min: {min_val:.2}");

    let max_galley = ctx.painter.layout_no_wrap(max_text.clone(), font.clone(), text_color);
    let min_galley = ctx.painter.layout_no_wrap(min_text.clone(), font.clone(), text_color);

    let label_x = frame_width - max_galley.size().x.max(min_galley.size().x) - 5.0;

    // Max label (top)
    let max_pos = to_screen(label_x, 1.0);
    let max_rect = egui::Rect::from_min_size(
        Pos2::new(max_pos.x - 2.0, max_pos.y - 2.0),
        egui::Vec2::new(max_galley.size().x + 4.0, max_galley.size().y + 4.0),
    );
    ctx.painter.rect_filled(max_rect, 2.0, bg_color);
    ctx.painter.text(max_pos, emath::Align2::LEFT_TOP, max_text, font.clone(), text_color);

    // Min label (bottom)
    let min_pos = to_screen(label_x, 0.0);
    let min_rect = egui::Rect::from_min_size(
        Pos2::new(min_pos.x - 2.0, min_pos.y - min_galley.size().y - 2.0),
        egui::Vec2::new(min_galley.size().x + 4.0, min_galley.size().y + 4.0),
    );
    ctx.painter.rect_filled(min_rect, 2.0, bg_color);
    ctx.painter.text(min_pos, emath::Align2::LEFT_BOTTOM, min_text, font, text_color);
}
