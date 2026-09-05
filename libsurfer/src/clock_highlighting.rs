//! Drawing and handling of clock highlighting.
use derive_more::{Display, FromStr};
use ecolor::Color32;
use egui::Ui;
use emath::{Pos2, Rect};
use enum_iterator::Sequence;
use epaint::{CornerRadius, Shape, Stroke};
use serde::{Deserialize, Serialize};

use crate::{config::SurferConfig, message::Message, view::DrawingContext};

/// Base period, in screen pixels, used to interleave dashed clock-edge lines.
///
/// For coincident edges from N clocks, each clock gets a dash length of
/// `CLOCK_LINE_BASE_UNIT / N` and a corresponding phase offset so all dashes tile
/// the same period without overlap.
const CLOCK_LINE_BASE_UNIT: f32 = 30.0;

/// Cached clock-highlight payload for the active highlight strategy.
///
/// The payload stores data in the shape most efficient for the selected
/// `ClockHighlightType` and includes `active_clock_count` metadata to avoid
/// recomputing active-clock state while drawing.
pub(crate) enum ClockHighlightData {
    Line {
        clock_edges: Vec<(f32, Vec<usize>)>,
        active_clock_count: usize,
    },
    Cycle {
        clock_edges_by_clock: Vec<Vec<f32>>,
        active_clock_count: usize,
    },
    None,
}

impl ClockHighlightData {
    /// Returns whether this cached highlight payload represents any active clocks.
    ///
    /// The value is derived from `active_clock_count` metadata captured during draw-command
    /// generation so rendering can skip additional scans.
    pub(crate) fn has_edges(&self) -> bool {
        match self {
            Self::Line {
                active_clock_count, ..
            }
            | Self::Cycle {
                active_clock_count, ..
            } => *active_clock_count > 0,
            Self::None => false,
        }
    }
}

/// Selects a stable highlight color for a clock index.
///
/// If only one clock is active, the base fallback color is always used.
/// For multi-clock rendering, the color cycle is: fallback, then `color_list` entries.
fn clock_highlight_color(
    clock_idx: usize,
    single_active_clock: bool,
    fallback_color: Color32,
    color_list: &[Color32],
) -> Color32 {
    if single_active_clock {
        fallback_color
    } else {
        match clock_idx % (color_list.len() + 1) {
            0 => fallback_color,
            color_idx => color_list[color_idx - 1],
        }
    }
}

#[derive(PartialEq, Copy, Clone, Debug, Deserialize, Display, FromStr, Sequence, Serialize)]
pub enum ClockHighlightType {
    /// Draw a line at every posedge of the clocks
    Line,

    /// Highlight every other cycle
    Cycle,

    /// No highlighting
    None,
}

/// Draws clock highlight marks for the currently selected highlight mode.
///
/// `Line` mode draws vertical lines (or interleaved dashes for coincident edges), while
/// `Cycle` mode paints alternating cycle spans for each active clock.
pub(crate) fn draw_clock_edge_marks(
    clock_edges: &ClockHighlightData,
    ctx: &mut DrawingContext,
    config: &SurferConfig,
) {
    match clock_edges {
        ClockHighlightData::Line {
            clock_edges,
            active_clock_count,
        } => {
            let y_start = (ctx.to_screen)(0., 0.).y;
            let y_end = y_start + ctx.cfg.canvas_size.y;
            let stroke_width = config.theme.clock_highlight_line.width;
            let fallback_color = config.theme.clock_highlight_line.color;
            let color_list = &config.theme.clock_highlight_line_colors;
            let single_active_clock = *active_clock_count <= 1;

            for (x, clock_indices) in clock_edges {
                let x_pos = (ctx.to_screen)(*x, 0.).x;

                if clock_indices.len() == 1 {
                    let clock_idx = clock_indices[0];
                    let stroke_color = clock_highlight_color(
                        clock_idx,
                        single_active_clock,
                        fallback_color,
                        color_list,
                    );
                    let stroke = Stroke::new(stroke_width, stroke_color);
                    ctx.painter.vline(x_pos, y_start..=y_end, stroke);
                    continue;
                }

                let dash_length = CLOCK_LINE_BASE_UNIT / clock_indices.len() as f32;
                let gap_length = CLOCK_LINE_BASE_UNIT - dash_length;

                let line = [Pos2::new(x_pos, y_start), Pos2::new(x_pos, y_end)];
                for (phase_idx, clock_idx) in clock_indices.iter().copied().enumerate() {
                    let stroke_color = clock_highlight_color(
                        clock_idx,
                        single_active_clock,
                        fallback_color,
                        color_list,
                    );
                    let stroke = Stroke::new(stroke_width, stroke_color);
                    let offset = phase_idx as f32 * dash_length;

                    ctx.painter.add(Shape::dashed_line_with_offset(
                        &line,
                        stroke,
                        &[dash_length],
                        &[gap_length],
                        offset,
                    ));
                }
            }
        }
        ClockHighlightData::Cycle {
            clock_edges_by_clock,
            active_clock_count,
        } => {
            // Process clock edges in pairs: every other cycle gets highlighted
            let fallback_fill_color = config.theme.clock_highlight_cycle;
            let color_list = &config.theme.clock_highlight_cycle_colors;
            let single_active_clock = *active_clock_count <= 1;

            for (clock_idx, clock_edges) in clock_edges_by_clock.iter().enumerate() {
                let fill_color = clock_highlight_color(
                    clock_idx,
                    single_active_clock,
                    fallback_fill_color,
                    color_list,
                );
                let fill_color = if single_active_clock {
                    fill_color
                } else {
                    egui::Color32::from_rgba_unmultiplied(
                        fill_color.r(),
                        fill_color.g(),
                        fill_color.b(),
                        128,
                    )
                };

                for chunk in clock_edges.chunks(2) {
                    if let [x_start, x_end] = chunk {
                        let Pos2 {
                            x: x_end_screen,
                            y: y_start,
                        } = (ctx.to_screen)(*x_end, 0.);
                        ctx.painter.rect_filled(
                            Rect {
                                min: (ctx.to_screen)(*x_start, 0.),
                                max: Pos2 {
                                    x: x_end_screen,
                                    y: ctx.cfg.canvas_size.y + y_start,
                                },
                            },
                            CornerRadius::ZERO,
                            fill_color,
                        );
                    }
                }
            }
        }
        ClockHighlightData::None => (),
    }
}

/// Renders the UI radio options for selecting the clock highlight type.
pub(crate) fn clock_highlight_type_menu(
    ui: &mut Ui,
    msgs: &mut Vec<Message>,
    clock_highlight_type: ClockHighlightType,
) {
    for highlight_type in enum_iterator::all::<ClockHighlightType>() {
        if ui
            .radio(
                highlight_type == clock_highlight_type,
                highlight_type.to_string(),
            )
            .clicked()
        {
            msgs.push(Message::SetClockHighlightType(highlight_type));
        }
    }
}

/// Groups sparse per-clock edge lists into time-ordered `(time, clock_indices)` tuples.
///
/// This structure is used by `Line` mode to interleave coincident edges at the same time.
fn group_clock_edges_by_time(
    clock_edges_by_clock: Vec<(usize, Vec<f32>)>,
) -> Vec<(f32, Vec<usize>)> {
    let mut flattened = clock_edges_by_clock
        .into_iter()
        .flat_map(|(clock_idx, edges)| edges.into_iter().map(move |x| (x, clock_idx)))
        .collect::<Vec<_>>();
    flattened.sort_by(|(x1, _), (x2, _)| x1.total_cmp(x2));

    let mut grouped = Vec::<(f32, Vec<usize>)>::new();
    for (x, clock_idx) in flattened {
        if let Some((last_x, clock_indices)) = grouped.last_mut()
            && *last_x == x
        {
            clock_indices.push(clock_idx);
        } else {
            grouped.push((x, vec![clock_idx]));
        }
    }
    grouped
}

/// Converts sparse `(clock_idx, edges)` pairs into a dense vector indexed by clock index.
///
/// Missing indices are represented by empty edge lists.
fn dense_clock_edges_by_clock(clock_edges_by_clock: Vec<(usize, Vec<f32>)>) -> Vec<Vec<f32>> {
    let max_clock_idx = clock_edges_by_clock
        .iter()
        .map(|(clock_idx, _)| *clock_idx)
        .max()
        .map_or(0, |max_idx| max_idx + 1);
    let mut dense_clock_edges = vec![Vec::<f32>::new(); max_clock_idx];

    for (clock_idx, edges) in clock_edges_by_clock {
        dense_clock_edges[clock_idx] = edges;
    }

    dense_clock_edges
}

impl crate::tile_kinds::waveform_services::WaveformReadServices<'_> {
    /// Builds cached clock-highlight data for the active highlight mode.
    ///
    /// The input uses sparse clock indices so color assignment remains stable relative to
    /// original clock order, and `active_clock_count` is stored for fast rendering decisions.
    pub(crate) fn get_clock_hightlight_data(
        &self,
        clock_edges_by_clock: Vec<(usize, Vec<f32>)>,
    ) -> ClockHighlightData {
        let active_clock_count = clock_edges_by_clock.len();

        match self.clock_highlight_type {
            ClockHighlightType::Line => ClockHighlightData::Line {
                clock_edges: group_clock_edges_by_time(clock_edges_by_clock),
                active_clock_count,
            },
            ClockHighlightType::Cycle => ClockHighlightData::Cycle {
                clock_edges_by_clock: dense_clock_edges_by_clock(clock_edges_by_clock),
                active_clock_count,
            },
            ClockHighlightType::None => ClockHighlightData::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_highlight_data_has_edges_uses_active_clock_count() {
        let line = ClockHighlightData::Line {
            clock_edges: vec![(1.0, vec![0])],
            active_clock_count: 1,
        };
        assert!(line.has_edges());

        let line_inactive = ClockHighlightData::Line {
            clock_edges: vec![(1.0, vec![0])],
            active_clock_count: 0,
        };
        assert!(!line_inactive.has_edges());

        let cycle = ClockHighlightData::Cycle {
            clock_edges_by_clock: vec![vec![2.0]],
            active_clock_count: 1,
        };
        assert!(cycle.has_edges());

        let cycle_inactive = ClockHighlightData::Cycle {
            clock_edges_by_clock: vec![vec![2.0]],
            active_clock_count: 0,
        };
        assert!(!cycle_inactive.has_edges());

        assert!(!ClockHighlightData::None.has_edges());
    }

    #[test]
    fn clock_highlight_color_uses_fallback_for_single_clock() {
        let fallback = egui::Color32::from_rgb(1, 2, 3);
        let list = [egui::Color32::from_rgb(10, 20, 30)];

        assert_eq!(clock_highlight_color(0, true, fallback, &list), fallback);
        assert_eq!(clock_highlight_color(5, true, fallback, &list), fallback);
    }

    #[test]
    fn clock_highlight_color_cycles_fallback_and_list_for_multi_clock() {
        let fallback = egui::Color32::from_rgb(1, 2, 3);
        let c1 = egui::Color32::from_rgb(10, 20, 30);
        let c2 = egui::Color32::from_rgb(40, 50, 60);
        let list = [c1, c2];

        assert_eq!(clock_highlight_color(0, false, fallback, &list), fallback);
        assert_eq!(clock_highlight_color(1, false, fallback, &list), c1);
        assert_eq!(clock_highlight_color(2, false, fallback, &list), c2);
        assert_eq!(clock_highlight_color(3, false, fallback, &list), fallback);
    }

    #[test]
    fn group_clock_edges_by_time_groups_and_sorts() {
        let grouped = group_clock_edges_by_time(vec![
            (2, vec![7.0, 1.0]),
            (0, vec![4.0]),
            (1, vec![1.0, 7.0]),
        ]);

        assert_eq!(
            grouped,
            vec![(1.0, vec![2, 1]), (4.0, vec![0]), (7.0, vec![2, 1])]
        );
    }

    #[test]
    fn dense_clock_edges_by_clock_keeps_sparse_indices() {
        let dense = dense_clock_edges_by_clock(vec![(2, vec![3.0]), (0, vec![1.0, 2.0])]);

        assert_eq!(dense.len(), 3);
        assert_eq!(dense[0], vec![1.0, 2.0]);
        assert_eq!(dense[1], Vec::<f32>::new());
        assert_eq!(dense[2], vec![3.0]);
    }
}
