//! Code related to the mouse gesture handling.
use derive_more::Display;
use egui::{Context, Painter, PointerButton, Response, RichText, Sense, Window};
use emath::{Align2, Pos2, Rect, RectTransform, Vec2};
use epaint::{FontId, Stroke};
use num::BigInt;
use serde::Deserialize;

use crate::arrow::{ArrowHeadMode, WavePoint};
use crate::config::{SurferConfig, SurferTheme};
use crate::graphics::{Anchor, GraphicsY};
use crate::time::TimeFormatter;
use crate::view::DrawingContext;
use crate::{Message, SystemState, wave_data::WaveData};

/// Geometric constant: tan(22.5°) used for gesture zone calculations
const TAN_22_5_DEGREES: f32 = 0.41421357;

/// Helper function to create a stroke with appropriate color and width based on mode
fn create_gesture_stroke(config: &SurferConfig, is_measure: bool) -> Stroke {
    let line_style = if is_measure {
        &config.theme.measure
    } else {
        &config.theme.gesture
    };
    Stroke::from(line_style)
}

/// The supported mouse gesture operations.
#[derive(Clone, PartialEq, Copy, Display, Debug, Deserialize)]
enum GestureKind {
    #[display("Zoom to fit")]
    ZoomToFit,
    #[display("Zoom in")]
    ZoomIn,
    #[display("Zoom out")]
    ZoomOut,
    #[display("Go to end")]
    GoToEnd,
    #[display("Go to start")]
    GoToStart,
    Cancel,
}

/// The supported mouse gesture zones.
#[derive(Clone, PartialEq, Copy, Debug, Deserialize)]
pub struct GestureZones {
    north: GestureKind,
    northeast: GestureKind,
    east: GestureKind,
    southeast: GestureKind,
    south: GestureKind,
    southwest: GestureKind,
    west: GestureKind,
    northwest: GestureKind,
}

// The supported annotations.
#[derive(Clone, PartialEq, Copy, Display, Debug, Deserialize)]
pub enum AnnotationKind {
    Rectangle,
    ArrowSingleHead,
    ArrowDoubleHead,
}

impl SystemState {
    //Adjusts y_value to not go without scope and whether it should snap to waves or not.
    #[allow(clippy::too_many_arguments)]
    fn clamp_y(
        &self,
        pos: Pos2,
        max_y: f32,
        snap_y: bool,
        waves: &WaveData,
        ctx: &mut DrawingContext<'_>,
        anchor: Anchor,
        y_offset: f32,
    ) -> Pos2 {
        let mut y = pos.y.clamp(waves.get_content_start(ctx), max_y);
        if snap_y {
            let local_y = y - y_offset;

            if let Some(snapped_y) = waves.item_ref_at_canvas_y(local_y).and_then(|item_ref| {
                let gy = GraphicsY {
                    item: item_ref,
                    anchor,
                };

                waves.get_item_y(&gy)
            }) {
                y = snapped_y + y_offset;
            }
        }

        Pos2 {
            x: pos.x,
            y: y.min(max_y),
        }
    }

    /// Draw the mouse gesture widget, i.e., the line(s) and text showing which gesture is being drawn.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_mouse_gesture_widget(
        &self,
        egui_ctx: &Context,
        waves: &WaveData,
        pointer_pos_canvas: Option<Pos2>,
        response: &Response,
        msgs: &mut Vec<Message>,
        ctx: &mut DrawingContext,
        viewport_idx: usize,
        y_offset: f32,
    ) {
        if let Some(mut start_location) = self.gesture_start_location {
            if self.annotation_kind == Some(AnnotationKind::Rectangle)
                && start_location.y
                    > (waves.get_content_height(ctx) + self.user.config.layout.waveforms_gap)
            {
                return;
            }
            //Attach position to canvas, so it doesn't follow screen movement.
            if let Some(time) = &self.gesture_start_time {
                let range = waves.time_range();
                let x_pixel = waves.viewports[viewport_idx].pixel_from_time(
                    time,
                    ctx.cfg.canvas_size.x,
                    range,
                );
                start_location.x = x_pixel;
            }
            let modifiers = egui_ctx.input(|i| i.modifiers);
            if response.dragged_by(PointerButton::Middle)
                || modifiers.command && response.dragged_by(PointerButton::Primary)
                || self.annotation_kind.is_some() && response.dragged_by(PointerButton::Primary)
            {
                self.start_dragging(
                    pointer_pos_canvas,
                    start_location,
                    ctx,
                    egui_ctx,
                    response,
                    waves,
                    viewport_idx,
                    y_offset,
                );
            }

            if response.drag_stopped_by(PointerButton::Middle)
                || modifiers.command && response.drag_stopped_by(PointerButton::Primary)
                || self.annotation_kind.is_some()
                    && response.drag_stopped_by(PointerButton::Primary)
            {
                let frame_width = response.rect.width();
                self.stop_dragging(
                    pointer_pos_canvas,
                    start_location,
                    msgs,
                    viewport_idx,
                    waves,
                    frame_width,
                    ctx,
                    egui_ctx,
                    y_offset,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn stop_dragging(
        &self,
        pointer_pos_canvas: Option<Pos2>,
        start_location: Pos2,
        msgs: &mut Vec<Message>,
        viewport_idx: usize,
        waves: &WaveData,
        frame_width: f32,
        ctx: &mut DrawingContext<'_>,
        ui: &Context,
        y_offset: f32,
    ) {
        let range = waves.time_range();
        let Some(end_location) = pointer_pos_canvas else {
            return;
        };
        let distance = end_location - start_location;
        if distance.length_sq() >= self.user.config.gesture.deadzone {
            match self.annotation_kind {
                Some(AnnotationKind::Rectangle) => {
                    self.create_rectangle(
                        end_location,
                        start_location,
                        msgs,
                        viewport_idx,
                        waves,
                        frame_width,
                        ctx,
                        ui,
                        y_offset,
                    );
                }
                Some(AnnotationKind::ArrowSingleHead | AnnotationKind::ArrowDoubleHead) => {
                    self.create_arrow(
                        end_location,
                        start_location,
                        msgs,
                        viewport_idx,
                        waves,
                        frame_width,
                        ctx,
                        y_offset,
                    );
                }
                _ => {
                    match gesture_type(self.user.config.gesture.mapping, distance) {
                        GestureKind::ZoomToFit => {
                            msgs.push(Message::ZoomToFit { viewport_idx });
                        }
                        GestureKind::ZoomIn => {
                            let (min_x, max_x) = if end_location.x < start_location.x {
                                (end_location.x, start_location.x)
                            } else {
                                (start_location.x, end_location.x)
                            };
                            msgs.push(Message::ZoomToRange {
                                // FIXME: No need to go via bigint here, this could all be relative
                                start: waves.viewports[viewport_idx].as_time_bigint(
                                    min_x,
                                    frame_width,
                                    range,
                                ),
                                end: waves.viewports[viewport_idx].as_time_bigint(
                                    max_x,
                                    frame_width,
                                    range,
                                ),
                                viewport_idx,
                            });
                        }
                        GestureKind::GoToStart => {
                            msgs.push(Message::GoToStart { viewport_idx });
                        }
                        GestureKind::GoToEnd => {
                            msgs.push(Message::GoToEnd { viewport_idx });
                        }
                        GestureKind::ZoomOut => {
                            msgs.push(Message::CanvasZoom {
                                mouse_ptr: None,
                                delta: 2.0,
                                viewport_idx,
                            });
                        }
                        GestureKind::Cancel => {}
                    }
                }
            }
        }
        msgs.push(Message::SetMouseGestureDragStart(None, None));
        msgs.push(Message::SetMouseGestureAnnotation(None));
    }

    #[allow(clippy::too_many_arguments)]
    fn start_dragging(
        &self,
        pointer_pos_canvas: Option<Pos2>,
        start_location: Pos2,
        ctx: &mut DrawingContext<'_>,
        ui: &Context,
        response: &Response,
        waves: &WaveData,
        viewport_idx: usize,
        y_offset: f32,
    ) {
        let Some(current_location) = pointer_pos_canvas else {
            return;
        };
        let distance = current_location - start_location;
        if distance.length_sq() >= self.user.config.gesture.deadzone {
            match self.annotation_kind {
                Some(AnnotationKind::Rectangle) => {
                    self.draw_gesture_rectangle(
                        start_location,
                        waves,
                        ui,
                        current_location,
                        ctx,
                        y_offset,
                    );
                }
                Some(AnnotationKind::ArrowSingleHead | AnnotationKind::ArrowDoubleHead) => {
                    self.draw_arrow_line(start_location, current_location, "Add arrow", true, ctx);
                }
                _ => match gesture_type(self.user.config.gesture.mapping, distance) {
                    GestureKind::ZoomToFit => self.draw_gesture_line(
                        start_location,
                        current_location,
                        "Zoom to fit",
                        true,
                        ctx,
                    ),
                    GestureKind::ZoomIn => self.draw_zoom_in_gesture(
                        start_location,
                        current_location,
                        response,
                        ctx,
                        waves,
                        viewport_idx,
                        false,
                    ),

                    GestureKind::GoToStart => self.draw_gesture_line(
                        start_location,
                        current_location,
                        "Go to start",
                        true,
                        ctx,
                    ),
                    GestureKind::GoToEnd => {
                        self.draw_gesture_line(
                            start_location,
                            current_location,
                            "Go to end",
                            true,
                            ctx,
                        );
                    }
                    GestureKind::ZoomOut => {
                        self.draw_gesture_line(
                            start_location,
                            current_location,
                            "Zoom out",
                            true,
                            ctx,
                        );
                    }
                    GestureKind::Cancel => {
                        self.draw_gesture_line(
                            start_location,
                            current_location,
                            "Cancel",
                            false,
                            ctx,
                        );
                    }
                },
            }
        } else if self.annotation_kind.is_none() {
            draw_gesture_help(
                &self.user.config,
                response,
                ctx.painter,
                Some(start_location),
                true,
            );
        }
    }

    fn draw_gesture_rectangle(
        &self,
        start_location: Pos2,
        waves: &WaveData,
        ui: &Context,
        current_location: Pos2,
        ctx: &mut DrawingContext,
        y_offset: f32,
    ) {
        let modifiers = ui.input(|i| i.modifiers);
        let max_y = waves.get_content_height(ctx);
        let current_anchor = {
            if current_location.y > start_location.y {
                Anchor::Bottom
            } else {
                Anchor::Top
            }
        };
        let start_anchor = {
            if start_location.y < current_location.y {
                Anchor::Top
            } else {
                Anchor::Bottom
            }
        };
        let end = self.clamp_y(
            current_location,
            max_y,
            !modifiers.shift,
            waves,
            ctx,
            current_anchor,
            y_offset,
        );
        let start = self.clamp_y(
            start_location,
            max_y,
            !modifiers.shift,
            waves,
            ctx,
            start_anchor,
            y_offset,
        );
        let color = self.user.config.theme.annotation_rectangle.color;
        let stroke = Stroke {
            color,
            width: self.user.config.theme.annotation_rectangle.width,
        };

        let start_pos = (ctx.to_screen)(start.x, start.y);
        let end_pos = (ctx.to_screen)(end.x, end.y);

        let temp_rect = emath::Rect::from_two_pos(start_pos, end_pos);

        ctx.painter
            .rect_stroke(temp_rect, 0.0, stroke, egui::StrokeKind::Middle);
    }

    #[allow(clippy::too_many_arguments)]
    fn create_rectangle(
        &self,
        end_location: Pos2,
        start_location: Pos2,
        msgs: &mut Vec<Message>,
        viewport_idx: usize,
        waves: &WaveData,
        frame_width: f32,
        ctx: &mut DrawingContext<'_>,
        ui: &Context,
        y_offset: f32,
    ) {
        let range = waves.time_range();
        let modifiers = ui.input(|i| i.modifiers);
        let max_y = waves.get_content_height(ctx);

        let end_anchor = if end_location.y > start_location.y {
            Anchor::Bottom
        } else {
            Anchor::Top
        };

        let start_anchor = if start_location.y < end_location.y {
            Anchor::Top
        } else {
            Anchor::Bottom
        };

        let end = self.clamp_y(
            end_location,
            max_y,
            !modifiers.shift,
            waves,
            ctx,
            end_anchor,
            y_offset,
        );

        let start = self.clamp_y(
            start_location,
            max_y,
            !modifiers.shift,
            waves,
            ctx,
            start_anchor,
            y_offset,
        );

        let rect = emath::Rect::from_two_pos(start, end);

        let viewport = &waves.viewports[viewport_idx];

        let t1 = viewport.as_time_bigint(start_location.x, frame_width, range);
        let t2 = viewport.as_time_bigint(end_location.x, frame_width, range);

        let (time_start, time_end) = (t1.clone().min(t2.clone()), t1.max(t2));

        let get_anchored_y = |y: f32, anchor: Anchor| {
            waves
                .item_ref_at_canvas_y(y)
                .map(|item| GraphicsY { item, anchor })
        };

        let get_percentual_y = |lookup_y: f32, scale_y: f32| {
            waves.item_ref_at_canvas_y(lookup_y).map(|item| {
                let p = waves.get_item_y_scale(item, scale_y);

                GraphicsY {
                    item,
                    anchor: Anchor::Percentual(p.unwrap_or(0.)),
                }
            })
        };

        let (wave_from, wave_to) = if modifiers.shift {
            let from =
                get_percentual_y(start.y.min(end.y) - y_offset, start.y.min(end.y) - y_offset);

            let to = get_percentual_y(
                end.y.max(start.y) - y_offset - self.user.config.layout.waveforms_gap * 2.,
                end.y.max(start.y) - y_offset,
            );

            (from, to)
        } else {
            let y_from = start.y.min(end.y);
            let y_to = start.y.max(end.y);

            let from = get_anchored_y(y_from - y_offset, Anchor::Top);

            let mut adjusted_y = y_to - y_offset;
            if y_to > waves.get_content_start(ctx) {
                adjusted_y -= self.user.config.layout.waveforms_gap * 2.0;
            }

            let to = get_anchored_y(adjusted_y, Anchor::Bottom);

            (from, to)
        };

        msgs.push(Message::RectangleAdded {
            time_at_start: time_start,
            time_at_end: time_end,
            wave_from,
            wave_to,
            rect,
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn create_arrow(
        &self,
        end_location: Pos2,
        start_location: Pos2,
        msgs: &mut Vec<Message>,
        viewport_idx: usize,
        waves: &WaveData,
        frame_width: f32,
        ctx: &mut DrawingContext<'_>,
        offset: f32,
    ) {
        let range = waves.time_range();
        let start_pos = (ctx.to_screen)(start_location.x, start_location.y);
        let end_pos = (ctx.to_screen)(end_location.x, end_location.y);

        let time_from: BigInt =
            waves.viewports[viewport_idx].as_time_bigint(start_location.x, frame_width, range);

        let snap_pos = Some(Pos2::new(end_location.x, end_location.y - offset));

        let time_to = self
            .snap_to_edge(snap_pos, waves, frame_width, viewport_idx)
            .unwrap_or_else(|| {
                waves.viewports[viewport_idx].as_time_bigint(end_location.x, frame_width, range)
            });

        let attached_item_to = waves.item_ref_at_canvas_y(end_location.y - offset);
        let attached_item_from = waves.item_ref_at_canvas_y(start_location.y - offset);

        let mut head_mode = ArrowHeadMode::End;

        if self.annotation_kind == Some(AnnotationKind::ArrowDoubleHead) {
            head_mode = ArrowHeadMode::Double;
        }

        let wave_point_from = WavePoint {
            time: time_from.clone(),
            attached_item: attached_item_from,
            screen_pos: start_pos,
        };

        let wave_point_to = WavePoint {
            time: time_to.clone(),
            attached_item: attached_item_to,
            screen_pos: end_pos,
        };

        if attached_item_to.is_some() {
            msgs.push(Message::ArrowAdded {
                wave_point_from,
                wave_point_to,
                head_mode,
            });
        }
    }

    /// Draw the line used by most mouse gestures.
    fn draw_gesture_line(
        &self,
        start: Pos2,
        end: Pos2,
        text: &str,
        active: bool,
        ctx: &mut DrawingContext,
    ) {
        let color = if active {
            self.user.config.theme.gesture.color
        } else {
            self.user.config.theme.gesture.color.gamma_multiply(0.3)
        };
        let stroke = Stroke {
            color,
            width: self.user.config.theme.gesture.width,
        };
        ctx.painter.line_segment(
            [
                (ctx.to_screen)(end.x, end.y),
                (ctx.to_screen)(start.x, start.y),
            ],
            stroke,
        );
        draw_gesture_text(
            ctx,
            (ctx.to_screen)(end.x, end.y),
            text,
            &self.user.config.theme,
        );
    }

    fn draw_arrow_line(
        &self,
        start: Pos2,
        end: Pos2,
        text: &str,
        active: bool,
        ctx: &mut DrawingContext,
    ) {
        let color = if active {
            self.user.config.theme.annotation_arrow.color
        } else {
            self.user.config.theme.gesture.color.gamma_multiply(0.3)
        };
        let stroke = Stroke {
            color,
            width: self.user.config.theme.gesture.width,
        };
        ctx.painter.line_segment(
            [
                (ctx.to_screen)(end.x, end.y),
                (ctx.to_screen)(start.x, start.y),
            ],
            stroke,
        );
        draw_gesture_text(
            ctx,
            (ctx.to_screen)(end.x, end.y),
            text,
            &self.user.config.theme,
        );
    }

    /// Draw the lines used for the zoom-in gesture.
    #[allow(clippy::too_many_arguments)]
    fn draw_zoom_in_gesture(
        &self,
        start_location: Pos2,
        current_location: Pos2,
        response: &Response,
        ctx: &mut DrawingContext<'_>,
        waves: &WaveData,
        viewport_idx: usize,
        measure: bool,
    ) {
        let stroke = create_gesture_stroke(&self.user.config, measure);
        let height = response.rect.height();
        let width = response.rect.width();
        let segments = [
            ((start_location.x, 0.0), (start_location.x, height)),
            ((current_location.x, 0.0), (current_location.x, height)),
            (
                (start_location.x, start_location.y),
                (current_location.x, start_location.y),
            ),
        ];
        for (start, end) in segments {
            ctx.painter.line_segment(
                [
                    (ctx.to_screen)(start.0, start.1),
                    (ctx.to_screen)(end.0, end.1),
                ],
                stroke,
            );
        }
        let (minx, maxx) = if measure || current_location.x > start_location.x {
            (start_location.x, current_location.x)
        } else {
            (current_location.x, start_location.x)
        };
        let range = waves.time_range();
        let start_time = waves.viewports[viewport_idx].as_time_bigint(minx, width, range);
        let end_time = waves.viewports[viewport_idx].as_time_bigint(maxx, width, range);
        let diff_time = &end_time - &start_time;
        let time_formatter = TimeFormatter::new(
            &waves.inner.metadata().timescale,
            &self.user.wanted_timeunit,
            &self.get_time_format(),
        );
        let start_time_str = time_formatter.format(&start_time);
        let end_time_str = time_formatter.format(&end_time);
        let diff_time_str = time_formatter.format(&diff_time);
        let text = if measure {
            format!("{start_time_str} to {end_time_str}\nΔ = {diff_time_str}")
        } else {
            format!("Zoom in: {diff_time_str}\n{start_time_str} to {end_time_str}")
        };
        draw_gesture_text(
            ctx,
            (ctx.to_screen)(current_location.x, current_location.y),
            text,
            &self.user.config.theme,
        );
    }

    /// Draw the mouse gesture help window.
    pub(crate) fn mouse_gesture_help(&self, ctx: &Context, msgs: &mut Vec<Message>) {
        let mut open = true;
        Window::new("Mouse gestures")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(
                        "Press middle mouse button (or ctrl+primary mouse button) and drag",
                    ));
                    ui.add_space(20.);
                    let (response, painter) = ui.allocate_painter(
                        Vec2 {
                            x: self.user.config.gesture.size,
                            y: self.user.config.gesture.size,
                        },
                        Sense::empty(),
                    );
                    draw_gesture_help(&self.user.config, &response, &painter, None, false);
                    ui.add_space(10.);
                    ui.separator();
                    if ui.button("Close").clicked() {
                        msgs.push(Message::SetGestureHelpVisible(false));
                    }
                });
            });
        if !open {
            msgs.push(Message::SetGestureHelpVisible(false));
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_measure_widget(
        &self,
        egui_ctx: &Context,
        waves: &WaveData,
        pointer_pos_item_space: Option<Pos2>,
        pointer_pos_canvas: Option<Pos2>,
        response: &Response,
        msgs: &mut Vec<Message>,
        ctx: &mut DrawingContext,
        viewport_idx: usize,
    ) {
        if let Some(start_location) = self.measure_start_location {
            let modifiers = egui_ctx.input(|i| i.modifiers);
            if !modifiers.command
                && response.dragged_by(PointerButton::Primary)
                && self.do_measure(&modifiers)
                && let Some(mut current_location) = pointer_pos_canvas
            {
                // Snap current X to nearest edge/time (same logic as cursor placement)
                let frame_width = response.rect.width();
                if let Some(snap_time) =
                    self.snap_to_edge(pointer_pos_item_space, waves, frame_width, viewport_idx)
                {
                    let x = waves.viewports[viewport_idx].pixel_from_time(
                        &snap_time,
                        frame_width,
                        waves.time_range(),
                    );
                    current_location.x = x;
                }

                self.draw_zoom_in_gesture(
                    start_location,
                    current_location,
                    response,
                    ctx,
                    waves,
                    viewport_idx,
                    true,
                );
            }
            if response.drag_stopped_by(PointerButton::Primary) {
                msgs.push(Message::SetMeasureDragStart(None));
            }
        }
    }
}

/// Draw the "compass" showing the boundaries for different gestures.
fn draw_gesture_help(
    config: &SurferConfig,
    response: &Response,
    painter: &Painter,
    midpoint: Option<Pos2>,
    draw_bg: bool,
) {
    let frame_size = response.rect.size();
    // Compute sizes and coordinates
    let (midx, midy, deltax, deltay) = if let Some(midpoint) = midpoint {
        let halfsize = config.gesture.size * 0.5;
        (midpoint.x, midpoint.y, halfsize, halfsize)
    } else {
        let halfwidth = frame_size.x * 0.5;
        let halfheight = frame_size.y * 0.5;
        (halfwidth, halfheight, halfwidth, halfheight)
    };

    let container_rect = Rect::from_min_size(Pos2::ZERO, frame_size);
    let to_screen = &|x, y| {
        RectTransform::from_to(container_rect, response.rect).transform_pos(Pos2::new(x, y))
    };
    let stroke = Stroke::from(&config.theme.gesture);
    let tan225deltax = TAN_22_5_DEGREES * deltax;
    let tan225deltay = TAN_22_5_DEGREES * deltay;
    let left = midx - deltax;
    let right = midx + deltax;
    let top = midy - deltay;
    let bottom = midy + deltay;
    // Draw background
    if draw_bg {
        let bg_radius = config.gesture.background_radius * deltax;
        painter.circle_filled(
            to_screen(midx, midy),
            bg_radius,
            config
                .theme
                .canvas_colors
                .background
                .gamma_multiply(config.gesture.background_gamma),
        );
    }
    // Draw lines
    let segments = [
        ((left, midy + tan225deltax), (right, midy - tan225deltax)),
        ((left, midy - tan225deltax), (right, midy + tan225deltax)),
        ((midx + tan225deltay, top), (midx - tan225deltay, bottom)),
        ((midx - tan225deltay, top), (midx + tan225deltay, bottom)),
    ];
    for (start, end) in segments {
        painter.line_segment(
            [to_screen(start.0, start.1), to_screen(end.0, end.1)],
            stroke,
        );
    }

    let halfwaytexty_upper = top + (deltay - tan225deltax) * 0.5;
    let halfwaytexty_lower = bottom - (deltay - tan225deltax) * 0.5;

    // Draw commands using a table-driven approach
    let directions = [
        (left, midy, Align2::LEFT_CENTER, config.gesture.mapping.west),
        (
            right,
            midy,
            Align2::RIGHT_CENTER,
            config.gesture.mapping.east,
        ),
        (
            left,
            halfwaytexty_upper,
            Align2::LEFT_CENTER,
            config.gesture.mapping.northwest,
        ),
        (
            right,
            halfwaytexty_upper,
            Align2::RIGHT_CENTER,
            config.gesture.mapping.northeast,
        ),
        (midx, top, Align2::CENTER_TOP, config.gesture.mapping.north),
        (
            left,
            halfwaytexty_lower,
            Align2::LEFT_CENTER,
            config.gesture.mapping.southwest,
        ),
        (
            right,
            halfwaytexty_lower,
            Align2::RIGHT_CENTER,
            config.gesture.mapping.southeast,
        ),
        (
            midx,
            bottom,
            Align2::CENTER_BOTTOM,
            config.gesture.mapping.south,
        ),
    ];

    for (x, y, align, text) in directions {
        painter.text(
            to_screen(x, y),
            align,
            text,
            FontId::default(),
            config.theme.foreground,
        );
    }
}

/// Determine which mouse gesture ([`GestureKind`]) is currently drawn.
fn gesture_type(zones: GestureZones, delta: Vec2) -> GestureKind {
    let tan225x = TAN_22_5_DEGREES * delta.x;
    let tan225y = TAN_22_5_DEGREES * delta.y;
    if delta.x < 0.0 {
        if delta.y.abs() < -tan225x {
            // West
            zones.west
        } else if delta.y < 0.0 && delta.x < tan225y {
            // North west
            zones.northwest
        } else if delta.y > 0.0 && delta.x < -tan225y {
            // South west
            zones.southwest
        } else if delta.y < 0.0 {
            // North
            zones.north
        } else {
            // South
            zones.south
        }
    } else if tan225x > delta.y.abs() {
        // East
        zones.east
    } else if delta.y < 0.0 && delta.x > -tan225y {
        // North east
        zones.northeast
    } else if delta.y > 0.0 && delta.x > tan225y {
        // South east
        zones.southeast
    } else if delta.y < 0.0 {
        // North
        zones.north
    } else {
        // South
        zones.south
    }
}

fn draw_gesture_text(
    ctx: &mut DrawingContext,
    pos: Pos2,
    text: impl ToString,
    theme: &SurferTheme,
) {
    // Translate away from the mouse cursor so the text isn't hidden by it
    let pos = pos + Vec2::new(10.0, -10.0);

    let galley = ctx
        .painter
        .layout_no_wrap(text.to_string(), FontId::default(), theme.foreground);

    ctx.painter.rect(
        galley.rect.translate(pos.to_vec2()).expand(3.0),
        2.0,
        theme.primary_ui_color.background,
        Stroke::default(),
        epaint::StrokeKind::Inside,
    );

    ctx.painter
        .galley(pos, galley, theme.primary_ui_color.foreground);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_zones() -> GestureZones {
        GestureZones {
            north: GestureKind::ZoomToFit,
            northeast: GestureKind::ZoomIn,
            east: GestureKind::GoToEnd,
            southeast: GestureKind::ZoomOut,
            south: GestureKind::Cancel,
            southwest: GestureKind::ZoomOut,
            west: GestureKind::GoToStart,
            northwest: GestureKind::ZoomIn,
        }
    }

    #[test]
    fn gesture_type_cardinal_directions() {
        let zones = default_zones();

        // Pure cardinal directions
        assert_eq!(
            gesture_type(zones, Vec2::new(100.0, 0.0)),
            GestureKind::GoToEnd
        ); // East
        assert_eq!(
            gesture_type(zones, Vec2::new(-100.0, 0.0)),
            GestureKind::GoToStart
        ); // West
        assert_eq!(
            gesture_type(zones, Vec2::new(0.0, -100.0)),
            GestureKind::ZoomToFit
        ); // North
        assert_eq!(
            gesture_type(zones, Vec2::new(0.0, 100.0)),
            GestureKind::Cancel
        ); // South
    }

    #[test]
    fn gesture_type_diagonal_directions() {
        let zones = default_zones();

        // 45-degree diagonals (should be in the diagonal zones)
        assert_eq!(
            gesture_type(zones, Vec2::new(100.0, -100.0)),
            GestureKind::ZoomIn
        ); // Northeast
        assert_eq!(
            gesture_type(zones, Vec2::new(100.0, 100.0)),
            GestureKind::ZoomOut
        ); // Southeast
        assert_eq!(
            gesture_type(zones, Vec2::new(-100.0, 100.0)),
            GestureKind::ZoomOut
        ); // Southwest
        assert_eq!(
            gesture_type(zones, Vec2::new(-100.0, -100.0)),
            GestureKind::ZoomIn
        ); // Northwest
    }

    #[test]
    fn gesture_type_boundary_zones() {
        let zones = default_zones();

        // Test vectors just inside the east zone boundary (tan(22.5°) ≈ 0.414)
        // For east: |y| < tan(22.5°) * x
        assert_eq!(
            gesture_type(zones, Vec2::new(100.0, 40.0)),
            GestureKind::GoToEnd
        ); // East
        assert_eq!(
            gesture_type(zones, Vec2::new(100.0, -40.0)),
            GestureKind::GoToEnd
        ); // East

        // Test vectors just outside the east zone boundary (should be southeast/northeast)
        assert_eq!(
            gesture_type(zones, Vec2::new(100.0, 50.0)),
            GestureKind::ZoomOut
        ); // Southeast
        assert_eq!(
            gesture_type(zones, Vec2::new(100.0, -50.0)),
            GestureKind::ZoomIn
        ); // Northeast
    }

    #[test]
    fn gesture_type_west_boundary_zones() {
        let zones = default_zones();

        // Test vectors just inside the west zone boundary
        assert_eq!(
            gesture_type(zones, Vec2::new(-100.0, 40.0)),
            GestureKind::GoToStart
        ); // West
        assert_eq!(
            gesture_type(zones, Vec2::new(-100.0, -40.0)),
            GestureKind::GoToStart
        ); // West

        // Test vectors just outside the west zone boundary
        assert_eq!(
            gesture_type(zones, Vec2::new(-100.0, 50.0)),
            GestureKind::ZoomOut
        ); // Southwest
        assert_eq!(
            gesture_type(zones, Vec2::new(-100.0, -50.0)),
            GestureKind::ZoomIn
        ); // Northwest
    }
}
