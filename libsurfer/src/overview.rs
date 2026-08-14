use crate::message::Message;
use crate::view::{DrawConfig, DrawingContext};
use crate::viewport::Viewport;
use crate::{
    SystemState,
    wave_data::{TimeRange, WaveData},
};
use egui::{Frame, Panel, PointerButton, Sense, Ui};
use emath::{Align2, Pos2, Rect, RectTransform};
use epaint::CornerRadius;

impl SystemState {
    pub(crate) fn add_overview_panel(
        &self,
        ui: &mut Ui,
        waves: &WaveData,
        msgs: &mut Vec<Message>,
    ) {
        Panel::bottom("overview")
            .frame(Frame {
                fill: self.user.config.theme.primary_ui_color.background,
                ..Default::default()
            })
            .show(ui, |ui| {
                self.draw_overview(ui, waves, msgs);
            });
    }

    fn draw_overview(&self, ui: &mut Ui, waves: &WaveData, msgs: &mut Vec<Message>) {
        let (response, mut painter) = ui.allocate_painter(ui.available_size(), Sense::drag());
        let frame_size = response.rect.size();
        let cfg = DrawConfig::new(
            frame_size,
            self.user.config.layout.waveforms_line_height,
            self.user.config.layout.waveforms_text_size,
        );
        let container_rect = Rect::from_min_size(Pos2::ZERO, frame_size);
        let to_screen = RectTransform::from_to(container_rect, response.rect);

        let mut ctx = DrawingContext {
            painter: &mut painter,
            cfg: &cfg,
            to_screen: &|x, y| to_screen.transform_pos(Pos2::new(x, y)),
            theme: &self.user.config.theme,
        };

        let range = waves.time_range();
        let viewport_all = waves.viewport_all();
        let base_fill_color = self.user.config.theme.canvas_colors.foreground;

        // Draw rectangles for each viewport
        waves
            .viewports
            .iter()
            .enumerate()
            .map(|(idx, viewport)| (idx, get_viewport_rect(&ctx, range, &viewport_all, viewport)))
            .for_each(|(idx, rect)| {
                let gamma = if idx == waves.last_active_viewport_idx {
                    0.6
                } else {
                    0.3
                };
                ctx.painter.rect_filled(
                    rect,
                    CornerRadius::ZERO,
                    base_fill_color.gamma_multiply(gamma),
                );
            });

        // Draw cursor
        waves.draw_cursor(&self.user.config.theme, &mut ctx, &viewport_all);

        // Draw ticks
        let mut ticks = self.get_ticks_for_viewport(waves, &viewport_all, &cfg);

        if ticks.len() >= 2 {
            // Remove first and last tick
            ticks.pop();
            ticks.remove(0);
            // Draw ticks
            waves.draw_ticks(
                self.user.config.theme.foreground,
                &ticks,
                &ctx,
                frame_size.y * 0.5,
                Align2::CENTER_CENTER,
            );
        }

        // Draw markers
        waves.draw_markers(&self.user.config.theme, &mut ctx, &viewport_all);
        waves.draw_marker_number_boxes(&mut ctx, &self.user.config.theme, &viewport_all);

        // Handle dragging of the primary viewport
        response.dragged_by(PointerButton::Primary).then(|| {
            let pointer_pos_global = ui.input(|i| i.pointer.interact_pos());
            let pos = pointer_pos_global.map(|p| to_screen.inverse().transform_pos(p));
            if let Some(pos) = pos {
                let timestamp = viewport_all.as_time_bigint(pos.x, frame_size.x, range);
                msgs.push(Message::GoToTime(Some(timestamp), 0));
            }
        });
    }
}

fn get_viewport_rect(
    ctx: &DrawingContext<'_>,
    range: &TimeRange,
    viewport_all: &Viewport,
    viewport: &Viewport,
) -> Rect {
    let minx = viewport_all.pixel_from_absolute_time(
        viewport.curr_left.absolute(range),
        ctx.cfg.canvas_size.x,
        range,
    );
    let maxx = viewport_all.pixel_from_absolute_time(
        viewport.curr_right.absolute(range),
        ctx.cfg.canvas_size.x,
        range,
    );
    let mut min = (ctx.to_screen)(minx, 0.);
    let mut max = (ctx.to_screen)(maxx, ctx.cfg.canvas_size.y);

    if max.x < min.x {
        std::mem::swap(&mut min.x, &mut max.x);
    }

    if max.x - min.x < 1.0 {
        let center_x = min.x.midpoint(max.x);
        min.x = center_x - 0.5;
        max.x = center_x + 0.5;
    }

    Rect::from_min_max(min, max)
}
