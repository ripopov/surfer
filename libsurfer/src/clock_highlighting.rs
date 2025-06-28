//! Drawing and handling of clock highlighting.
use derive_more::{Display, FromStr};
use egui::Ui;
use emath::{Pos2, Rect};
use enum_iterator::Sequence;
use epaint::Stroke;
use serde::{Deserialize, Serialize};

use crate::{config::SurferConfig, message::Message, view::DrawingContext};

#[derive(PartialEq, Copy, Clone, Debug, Deserialize, Display, FromStr, Sequence, Serialize)]
pub enum ClockHighlightType {
    /// Draw a line at every posedge of the clocks
    Line,

    /// Highlight every other cycle
    Cycle,

    /// No highlighting
    None,
}

pub fn draw_clock_edge_marks(
    clock_edges: &Vec<f32>,
    ctx: &mut DrawingContext,
    config: &SurferConfig,
    clock_highlight_type: ClockHighlightType,
) {
    match clock_highlight_type {
        ClockHighlightType::Line => {
            let stroke = Stroke {
                color: config.theme.clock_highlight_line.color,
                width: config.theme.clock_highlight_line.width,
            };

            for x in clock_edges {
                let Pos2 {
                    x: x_pos,
                    y: y_start,
                } = (ctx.to_screen)(*x, 0.);
                ctx.painter
                    .vline(x_pos, (y_start)..=(y_start + ctx.cfg.canvas_height), stroke);
            }
        }
        ClockHighlightType::Cycle => {
            let mut x_start = 0.0;
            let mut cycle = false;
            for x_tmp in clock_edges {
                if cycle {
                    let Pos2 {
                        x: x_end,
                        y: y_start,
                    } = (ctx.to_screen)(*x_tmp, 0.);
                    ctx.painter.rect_filled(
                        Rect {
                            min: (ctx.to_screen)(x_start, 0.),
                            max: Pos2 {
                                x: x_end,
                                y: ctx.cfg.canvas_height + y_start,
                            },
                        },
                        0.0,
                        config.theme.clock_highlight_cycle,
                    );
                }
                cycle = !cycle;
                x_start = *x_tmp;
            }
        }
        ClockHighlightType::None => (),
    }
}

pub fn clock_highlight_type_menu(
    ui: &mut Ui,
    msgs: &mut Vec<Message>,
    clock_highlight_type: ClockHighlightType,
) {
    for highlight_type in enum_iterator::all::<ClockHighlightType>() {
        ui.radio(
            highlight_type == clock_highlight_type,
            highlight_type.to_string(),
        )
        .clicked()
        .then(|| {
            ui.close_menu();
            msgs.push(Message::SetClockHighlightType(highlight_type));
        });
    }
}
