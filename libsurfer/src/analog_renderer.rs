use egui::{emath, Color32, Pos2, Stroke};
use epaint::PathShape;
use surfer_translation_types::ValueKind;

use crate::displayed_item::AnalogMode;
use crate::drawing_canvas::DrawingCommands;
use crate::translation::ValueKindExt;
use crate::view::DrawingContext;

/// Configuration for analog signal rendering
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

pub fn draw_analog(
    commands: &DrawingCommands,
    color: Color32,
    offset: f32,
    height_scaling_factor: f32,
    analog_mode: &AnalogMode,
    frame_width: f32,
    ctx: &mut DrawingContext,
) {
    if *analog_mode == AnalogMode::Off {
        return;
    }

    let config = AnalogRenderConfig::default();

    let (min_val, max_val) = match calculate_value_range(commands) {
        Some(range) => range,
        None => return,
    };

    // Render based on analog mode
    match analog_mode {
        AnalogMode::Step => {
            render_step_mode(
                commands,
                color,
                offset,
                height_scaling_factor,
                min_val,
                max_val,
                ctx,
                &config,
            );
        }
        AnalogMode::Interpolated => {
            render_interpolated_mode(
                commands,
                color,
                offset,
                height_scaling_factor,
                min_val,
                max_val,
                ctx,
                &config,
            );
        }
        AnalogMode::Off => return,
    }

    draw_amplitude_labels(
        color,
        offset,
        height_scaling_factor,
        min_val,
        max_val,
        frame_width,
        ctx,
        &config,
    );
}

fn create_stroke(color: Color32, config: &AnalogRenderConfig) -> Stroke {
    Stroke {
        color,
        width: 2.0 * config.line_width_multiplier,
    }
}

fn process_analog_points<F>(
    commands: &DrawingCommands,
    stroke: Stroke,
    value_range: f64,
    offset: f32,
    height_scaling_factor: f32,
    min_val: f64,
    ctx: &mut DrawingContext,
    mut point_processor: F,
) where
    F: FnMut(Pos2, Pos2, Option<(Pos2, f64)>, &mut DrawingContext, Stroke) -> Option<(Pos2, f64)>,
{
    let mut last_point: Option<(Pos2, f64)> = None;

    let trace_coords = |x, y_normalized: f32| {
        (ctx.to_screen)(
            x,
            (1.0 - y_normalized) * ctx.cfg.line_height * height_scaling_factor + offset,
        )
    };

    for ((old_x, prev_region), (new_x, _)) in
        commands.values.iter().zip(commands.values.iter().skip(1))
    {
        if let Some(translated_value) = &prev_region.inner {
            // Check if this is a special value kind (Z, X, etc.) or if it can be parsed as numeric
            let is_special_value = matches!(
                translated_value.kind,
                ValueKind::HighImp
                    | ValueKind::Undef
                    | ValueKind::DontCare
                    | ValueKind::Weak
                    | ValueKind::Warn
            );

            if is_special_value || parse_numeric_value(&translated_value.value).is_none() {
                // Draw a filled rectangle for Z/X/undefined regions
                let color = translated_value.kind.color(stroke.color, ctx.theme);
                let rect_min = (ctx.to_screen)(*old_x, offset);
                let rect_max =
                    (ctx.to_screen)(*new_x, offset + ctx.cfg.line_height * height_scaling_factor);

                ctx.painter.rect_filled(
                    egui::Rect::from_min_max(rect_min, rect_max),
                    0.0, // no corner radius
                    color,
                );

                // Reset last_point as we're not drawing a continuous line
                last_point = None;
            } else if let Some(numeric_value) = parse_numeric_value(&translated_value.value) {
                let normalized_value = if value_range.abs() > f64::EPSILON {
                    ((numeric_value - min_val) / value_range) as f32
                } else {
                    0.5
                };

                let start_point = trace_coords(*old_x, normalized_value);
                let end_point = trace_coords(*new_x, normalized_value);

                last_point = point_processor(start_point, end_point, last_point, ctx, stroke);
            }
        }
    }
}

fn render_step_mode(
    commands: &DrawingCommands,
    color: Color32,
    offset: f32,
    height_scaling_factor: f32,
    min_val: f64,
    max_val: f64,
    ctx: &mut DrawingContext,
    config: &AnalogRenderConfig,
) {
    let stroke = create_stroke(color, config);
    let value_range = max_val - min_val;

    process_analog_points(
        commands,
        stroke,
        value_range,
        offset,
        height_scaling_factor,
        min_val,
        ctx,
        |start_point, end_point, last_point, ctx, stroke| {
            if let Some((prev_point, _)) = last_point {
                if (prev_point.y - start_point.y).abs() > 1.0 {
                    // Draw vertical line for transition at the transition time
                    ctx.painter.add(PathShape::line(
                        vec![Pos2::new(start_point.x, prev_point.y), start_point],
                        stroke,
                    ));
                }
            }

            // Draw horizontal line for this value's duration
            ctx.painter
                .add(PathShape::line(vec![start_point, end_point], stroke));

            Some((end_point, 0.0))
        },
    );
}

fn render_interpolated_mode(
    commands: &DrawingCommands,
    color: Color32,
    offset: f32,
    height_scaling_factor: f32,
    min_val: f64,
    max_val: f64,
    ctx: &mut DrawingContext,
    config: &AnalogRenderConfig,
) {
    let stroke = create_stroke(color, config);
    let value_range = max_val - min_val;

    process_analog_points(
        commands,
        stroke,
        value_range,
        offset,
        height_scaling_factor,
        min_val,
        ctx,
        |start_point, _end_point, last_point, ctx, stroke| {
            if let Some((prev_point, _)) = last_point {
                ctx.painter
                    .add(PathShape::line(vec![prev_point, start_point], stroke));
            }

            Some((start_point, 0.0))
        },
    );
}

fn draw_amplitude_labels(
    color: Color32,
    offset: f32,
    height_scaling_factor: f32,
    min_val: f64,
    max_val: f64,
    frame_width: f32,
    ctx: &mut DrawingContext,
    config: &AnalogRenderConfig,
) {
    let trace_coords = |x, y_normalized: f32| {
        (ctx.to_screen)(
            x,
            (1.0 - y_normalized) * ctx.cfg.line_height * height_scaling_factor + offset,
        )
    };

    let text_size_multiplier = if height_scaling_factor <= config.text_size_multiplier_threshold {
        config.text_size_multipliers.0
    } else {
        config.text_size_multipliers.1
    };
    let text_size = ctx.cfg.text_size * text_size_multiplier;
    let text_color = color.gamma_multiply(config.label_alpha);
    let background_color = Color32::from_rgba_unmultiplied(0, 0, 0, config.background_alpha);

    let max_text = format!("max: {:.2}", max_val);
    let max_galley = ctx.painter.layout_no_wrap(
        max_text.clone(),
        egui::FontId::monospace(text_size),
        text_color,
    );

    let min_text = format!("min: {:.2}", min_val);
    let min_galley = ctx.painter.layout_no_wrap(
        min_text.clone(),
        egui::FontId::monospace(text_size),
        text_color,
    );

    let max_text_width = max_galley.size().x.max(min_galley.size().x);
    let label_x = frame_width - max_text_width - 5.0; // Account for text width + padding
    let (max_y_offset, min_y_offset) = (1.0, 0.0);
    let max_pos = trace_coords(label_x, max_y_offset);

    // Draw background rectangle for max label
    let max_bg_rect = egui::Rect::from_min_size(
        Pos2::new(max_pos.x - 2.0, max_pos.y - 2.0),
        egui::Vec2::new(max_galley.size().x + 4.0, max_galley.size().y + 4.0),
    );
    ctx.painter.rect_filled(max_bg_rect, 2.0, background_color);

    // Draw max label text
    ctx.painter.text(
        max_pos,
        emath::Align2::LEFT_TOP,
        max_text,
        egui::FontId::monospace(text_size),
        text_color,
    );

    // Min value label at adjusted bottom position
    let min_pos = trace_coords(label_x, min_y_offset);

    // Draw background rectangle for min label
    let min_bg_rect = egui::Rect::from_min_size(
        Pos2::new(min_pos.x - 2.0, min_pos.y - min_galley.size().y - 2.0),
        egui::Vec2::new(min_galley.size().x + 4.0, min_galley.size().y + 4.0),
    );
    ctx.painter.rect_filled(min_bg_rect, 2.0, background_color);

    // Draw min label text
    ctx.painter.text(
        min_pos,
        emath::Align2::LEFT_BOTTOM,
        min_text,
        egui::FontId::monospace(text_size),
        text_color,
    );
}

/// Parse numeric value from string representation
/// Only supports decimal parsing - translators should be switched to Unsigned/Signed/Floats when analog mode is enabled
pub fn parse_numeric_value(value: &str) -> Option<f64> {
    if value.is_empty() {
        return None;
    }

    value.parse::<f64>().ok()
}

/// Calculate range for currently visible data
pub fn calculate_value_range(commands: &DrawingCommands) -> Option<(f64, f64)> {
    let mut values = Vec::new();

    for (_, region) in &commands.values {
        if let Some(translated_value) = &region.inner {
            // Skip special value kinds (Z, X, etc.)
            let is_special_value = matches!(
                translated_value.kind,
                ValueKind::HighImp
                    | ValueKind::Undef
                    | ValueKind::DontCare
                    | ValueKind::Weak
                    | ValueKind::Warn
            );

            if !is_special_value {
                if let Some(numeric_value) = parse_numeric_value(&translated_value.value) {
                    values.push(numeric_value);
                }
            }
        }
    }

    if values.is_empty() {
        return None;
    }

    let min_val = values.iter().fold(f64::INFINITY, |a, &b| a.min(b));
    let max_val = values.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));

    // If min and max are the same, add some padding to avoid division by zero
    if (min_val - max_val).abs() < f64::EPSILON {
        Some((min_val - 0.5, max_val + 0.5))
    } else {
        Some((min_val, max_val))
    }
}
