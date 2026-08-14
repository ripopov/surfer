use ecolor::Color32;
use emath::{Align, Align2, Vec2};
use epaint::{CubicBezierShape, FontId, Shape, Stroke};
use num::BigInt;
use serde::{Deserialize, Serialize};

use crate::{
    config::SurferTheme, displayed_item::DisplayedItemRef, view::DrawingContext,
    viewport::Viewport, wave_data::WaveData,
};

#[derive(Serialize, Deserialize, Debug)]
pub enum Direction {
    North,
    East,
    South,
    West,
}

impl Direction {
    #[must_use]
    pub fn as_vector(&self) -> Vec2 {
        match self {
            Direction::North => Vec2::new(0., -1.),
            Direction::East => Vec2::new(-1., 0.),
            Direction::South => Vec2::new(0., 1.),
            Direction::West => Vec2::new(1., 0.),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Anchor {
    Top,
    Center,
    Bottom,
    Percentual(f32),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GraphicsY {
    pub item: DisplayedItemRef,
    pub anchor: Anchor,
}

/// A point used to place graphics.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GrPoint {
    /// Timestamp at which to place the graphic
    pub x: BigInt,
    pub y: GraphicsY,
}

#[derive(Serialize, Deserialize, PartialEq, PartialOrd, Eq, Ord, Hash, Debug)]
pub struct GraphicId(pub usize);

#[derive(Serialize, Deserialize, Debug)]
pub enum Graphic {
    TextArrow {
        from: (GrPoint, Direction),
        to: (GrPoint, Direction),
        text: String,
    },
    Text {
        pos: (GrPoint, Direction),
        text: String,
    },
    Rectangle {
        from: GrPoint,
        to: GrPoint,
    },
}

impl WaveData {
    // FIXME: This function should probably not be here, we should instead update ItemDrawingInfo to
    // have this info
    #[must_use]
    pub fn get_item_y(&self, y: &GraphicsY) -> Option<f32> {
        self.items_tree
            .iter_visible()
            .zip(&self.drawing_infos)
            .find(|(node, _info)| node.item_ref == y.item)
            .map(|(_, info)| info.get_y_from_anchor(&y.anchor))
            .map(|point| point - self.top_item_draw_offset)
    }

    /// Returns the y-value of an item given a percentual value
    #[must_use]
    pub fn get_item_y_scale(&self, item: DisplayedItemRef, y: f32) -> Option<f32> {
        let y = y + self.top_item_draw_offset;
        self.items_tree
            .iter_visible()
            .zip(&self.drawing_infos)
            .find(|(node, _info)| node.item_ref == item)
            .map(|(_, info)| (y - info.top()) / (info.height()))
    }

    pub(crate) fn draw_graphics(
        &self,
        ctx: &mut DrawingContext,
        viewport: &Viewport,
        theme: &SurferTheme,
    ) {
        let color = theme.variable_dontcare;
        let range = self.time_range();
        for g in self.graphics.values() {
            match g {
                Graphic::TextArrow {
                    from: (from_point, from_dir),
                    to: (to_point, to_dir),
                    text,
                } => {
                    let from_x =
                        viewport.pixel_from_time(&from_point.x, ctx.cfg.canvas_size.x, range);
                    let from_y = self.get_item_y(&from_point.y);

                    let to_x = viewport.pixel_from_time(&to_point.x, ctx.cfg.canvas_size.x, range);
                    let to_y = self.get_item_y(&to_point.y);

                    if let (Some(from_y), Some(to_y)) = (from_y, to_y) {
                        let from_dir = from_dir.as_vector() * 30.;
                        let to_dir_vec = to_dir.as_vector() * 30.;
                        let shape = Shape::CubicBezier(CubicBezierShape {
                            points: [
                                (ctx.to_screen)(from_x, from_y),
                                (ctx.to_screen)(from_x + from_dir.x, from_y + from_dir.y),
                                (ctx.to_screen)(to_x + to_dir_vec.x, to_y + to_dir_vec.y),
                                (ctx.to_screen)(to_x, to_y),
                            ],
                            closed: false,
                            fill: Color32::TRANSPARENT,
                            stroke: Stroke { width: 3., color }.into(),
                        });
                        ctx.painter.add(shape);

                        ctx.painter.text(
                            (ctx.to_screen)(to_x, to_y),
                            match to_dir {
                                Direction::North => Align2([Align::Center, Align::TOP]),
                                Direction::East => Align2([Align::LEFT, Align::Center]),
                                Direction::South => Align2([Align::Center, Align::BOTTOM]),
                                Direction::West => Align2([Align::RIGHT, Align::Center]),
                            },
                            text,
                            FontId::monospace(15.),
                            color,
                        );
                    }
                }
                Graphic::Text {
                    pos: (pos, dir),
                    text,
                } => {
                    let to_x = viewport.pixel_from_time(&pos.x, ctx.cfg.canvas_size.x, range);
                    let to_y = self.get_item_y(&pos.y);
                    if let Some(to_y) = to_y {
                        ctx.painter.text(
                            (ctx.to_screen)(to_x, to_y),
                            match dir {
                                Direction::North => Align2([Align::Center, Align::TOP]),
                                Direction::East => Align2([Align::LEFT, Align::Center]),
                                Direction::South => Align2([Align::Center, Align::BOTTOM]),
                                Direction::West => Align2([Align::RIGHT, Align::Center]),
                            },
                            text,
                            FontId::monospace(15.),
                            color,
                        );
                    }
                }
                Graphic::Rectangle {
                    from: from_point,
                    to: to_point,
                } => {
                    let from_x =
                        viewport.pixel_from_time(&from_point.x, ctx.cfg.canvas_size.x, range);
                    let from_y = self.get_item_y(&from_point.y);
                    let to_x =
                        viewport.pixel_from_time(&from_point.x, ctx.cfg.canvas_size.x, range);
                    let to_y = self.get_item_y(&to_point.y);

                    if let (Some(from_y), Some(to_y)) = (from_y, to_y) {
                        let start_pos = (ctx.to_screen)(from_x, from_y);
                        let end_pos = (ctx.to_screen)(to_x, to_y);
                        let temp_rect = emath::Rect::from_two_pos(start_pos, end_pos);
                        let stroke: Stroke = Stroke { width: 3., color };

                        ctx.painter
                            .rect_stroke(temp_rect, 0.0, stroke, egui::StrokeKind::Middle);
                    }
                }
            }
        }
    }
}
