use egui::{Color32, Frame, Id, Pos2, Rect, Stroke, Ui};
use egui_remixicon::icons;
use emath::RectTransform;
use num::BigInt;
use tracing::warn;

use crate::{
    SystemState,
    arrow::ArrowAnnotation,
    comment::{Comment, CommentMessage},
    config::SurferTheme,
    displayed_item::DisplayedItemRef,
    graphics::GraphicsY,
    message::Message,
    rectangle::RectAnnotation,
    time::TimeFormatter,
    view::DrawingContext,
    viewport::Viewport,
    wave_data::WaveData,
};

/// Borrowed annotation inputs for one waveform view. Annotation content belongs
/// to the item list; selection and menu placement belong to this view.
pub struct AnnotationView<'a> {
    pub document: &'a WaveData,
    pub items: &'a crate::item_list::ItemList,
    pub viewport: &'a Viewport,
    pub selected_annotation: Option<Id>,
    pub menu_position: Option<Pos2>,
    pub menu_time: Option<&'a BigInt>,
}

impl std::ops::Deref for AnnotationView<'_> {
    type Target = WaveData;
    fn deref(&self) -> &Self::Target {
        self.document
    }
}

const DEFAULT_HIDE_RADIUS: f32 = 5.0;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct AnnotationData {
    pub id: Id,
    pub visible: bool,
    pub name: String,
    pub stroke: Stroke,
    pub show_comments: bool,
    pub comment_box: Comment,
}

impl AnnotationData {
    pub(crate) fn new(id_source: impl egui::AsId, name: String, num: i32) -> Self {
        let id = Id::new(id_source);
        let c_id = Id::new(("comment_box", num));
        AnnotationData {
            id,
            visible: true,
            name,
            stroke: Stroke::new(2.0, Color32::from_rgb(255, 255, 255)),
            show_comments: false,
            comment_box: Comment::new(c_id, id),
        }
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum Annotation {
    Arrow(ArrowAnnotation),
    Rect(RectAnnotation),
}
impl Annotatable for Annotation {
    fn get_id(&self) -> Id {
        match self {
            Annotation::Arrow(a) => a.get_id(),
            Annotation::Rect(r) => r.get_id(),
        }
    }

    fn get_type(&self) -> &str {
        match self {
            Annotation::Arrow(a) => a.get_type(),
            Annotation::Rect(r) => r.get_type(),
        }
    }

    fn set_name(&mut self, name: &str) {
        match self {
            Annotation::Arrow(a) => a.set_name(name),
            Annotation::Rect(r) => r.set_name(name),
        }
    }

    fn get_name(&self) -> String {
        match self {
            Annotation::Arrow(a) => a.get_name(),
            Annotation::Rect(r) => r.get_name(),
        }
    }

    fn is_selected(&mut self) {
        match self {
            Annotation::Arrow(a) => a.is_selected(),
            Annotation::Rect(r) => r.is_selected(),
        }
    }

    fn set_visibility(&mut self, visible: bool) {
        match self {
            Annotation::Arrow(a) => a.set_visibility(visible),
            Annotation::Rect(r) => r.set_visibility(visible),
        }
    }

    fn show_comments(&self) -> bool {
        match self {
            Annotation::Arrow(a) => a.show_comments(),
            Annotation::Rect(r) => r.show_comments(),
        }
    }
    fn show_comment_box(&self) -> bool {
        match self {
            Annotation::Arrow(a) => a.show_comment_box(),
            Annotation::Rect(r) => r.show_comment_box(),
        }
    }

    fn set_show_comments(&mut self, show: bool) {
        match self {
            Annotation::Arrow(a) => a.set_show_comments(show),
            Annotation::Rect(r) => r.set_show_comments(show),
        }
    }

    fn get_comment_box(&self) -> Comment {
        match self {
            Annotation::Arrow(a) => a.get_comment_box(),
            Annotation::Rect(r) => r.get_comment_box(),
        }
    }

    fn get_comment_box_mut(&mut self) -> &mut Comment {
        match self {
            Annotation::Arrow(a) => a.get_comment_box_mut(),
            Annotation::Rect(r) => r.get_comment_box_mut(),
        }
    }

    fn is_visible(&self) -> bool {
        match self {
            Annotation::Arrow(a) => a.is_visible(),
            Annotation::Rect(r) => r.is_visible(),
        }
    }

    fn get_center_time(&self) -> BigInt {
        match self {
            Annotation::Arrow(a) => a.get_center_time(),
            Annotation::Rect(r) => r.get_center_time(),
        }
    }

    fn get_start_time(&self) -> BigInt {
        match self {
            Annotation::Arrow(a) => a.get_start_time(),
            Annotation::Rect(r) => r.get_start_time(),
        }
    }

    fn get_end_time(&self) -> BigInt {
        match self {
            Annotation::Arrow(a) => a.get_end_time(),
            Annotation::Rect(r) => r.get_end_time(),
        }
    }

    fn is_attached(&self, removed_ref: &DisplayedItemRef) -> bool {
        match self {
            Annotation::Arrow(a) => a.is_attached(removed_ref),
            Annotation::Rect(r) => r.is_attached(removed_ref),
        }
    }

    fn get_from_wave(&self) -> Option<GraphicsY> {
        match self {
            Annotation::Arrow(a) => a.get_from_wave(),
            Annotation::Rect(r) => r.get_from_wave(),
        }
    }

    fn get_to_wave(&self) -> Option<GraphicsY> {
        match self {
            Annotation::Arrow(a) => a.get_to_wave(),
            Annotation::Rect(r) => r.get_to_wave(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw(
        &self,
        ui: &mut Ui,
        waves: &AnnotationView<'_>,
        tile_id: crate::tiles::TileId,
        ctx: &mut DrawingContext,
        theme: &SurferTheme,
        msgs: &mut Vec<Message>,
        y_offset: f32,
        to_screen: RectTransform,
        time_formatter: &TimeFormatter,
    ) {
        match self {
            Annotation::Arrow(a) => a.draw(
                ui,
                waves,
                tile_id,
                ctx,
                theme,
                msgs,
                y_offset,
                to_screen,
                time_formatter,
            ),
            Annotation::Rect(r) => r.draw(
                ui,
                waves,
                tile_id,
                ctx,
                theme,
                msgs,
                y_offset,
                to_screen,
                time_formatter,
            ),
        }
    }

    fn get_comment_position(
        &self,
        viewport: &Viewport,
        ctx: &DrawingContext,
        waves: &AnnotationView<'_>,
        offset: f32,
    ) -> Pos2 {
        match self {
            Annotation::Arrow(a) => a.get_comment_position(viewport, ctx, waves, offset),
            Annotation::Rect(r) => r.get_comment_position(viewport, ctx, waves, offset),
        }
    }

    fn get_time_info(&self, time_formatter: &TimeFormatter) -> String {
        match self {
            Annotation::Arrow(a) => a.get_time_info(time_formatter),
            Annotation::Rect(r) => r.get_time_info(time_formatter),
        }
    }

    fn get_messages(&self) -> Vec<CommentMessage> {
        match self {
            Annotation::Arrow(a) => a.get_messages(),
            Annotation::Rect(r) => r.get_messages(),
        }
    }
}

pub trait Annotatable {
    fn get_id(&self) -> Id;
    fn get_type(&self) -> &str;
    fn set_name(&mut self, name: &str);
    fn get_name(&self) -> String;
    fn is_selected(&mut self);
    fn set_visibility(&mut self, visible: bool);
    fn show_comments(&self) -> bool;
    fn show_comment_box(&self) -> bool;
    fn set_show_comments(&mut self, show: bool);
    fn get_comment_box(&self) -> Comment;
    fn get_comment_box_mut(&mut self) -> &mut Comment;
    fn get_messages(&self) -> Vec<CommentMessage>;
    fn is_visible(&self) -> bool;
    fn get_center_time(&self) -> BigInt;
    fn get_start_time(&self) -> BigInt;
    fn get_end_time(&self) -> BigInt;
    /// Checks whether the annotation is attached to the given Item.
    fn is_attached(&self, removed_ref: &DisplayedItemRef) -> bool;
    fn get_time_info(&self, time_formatter: &TimeFormatter) -> String;
    fn get_from_wave(&self) -> Option<GraphicsY>;
    fn get_to_wave(&self) -> Option<GraphicsY>;
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &self,
        ui: &mut Ui,
        waves: &AnnotationView<'_>,
        tile_id: crate::tiles::TileId,
        ctx: &mut DrawingContext,
        theme: &SurferTheme,
        msgs: &mut Vec<Message>,
        y_offset: f32,
        to_screen: RectTransform,
        time_formatter: &TimeFormatter,
    );
    fn draw_quick_menu(
        &self,
        ui: &mut egui::Ui,
        msgs: &mut Vec<Message>,
        tile_id: crate::tiles::TileId,
        viewport_rect: egui::Rect,
        position: Pos2,
    ) {
        let id: Id = self.get_id();

        let menu_rect = egui::Rect::from_min_size(position, egui::vec2(0.0, 0.0));

        if !viewport_rect.intersects(menu_rect) {
            return;
        }

        egui::Area::new(ui.make_persistent_id(("annotation_quick_menu", id)))
            .order(egui::Order::Foreground)
            .fixed_pos(position)
            .show(ui.ctx(), |ui| {
                Frame::popup(ui.style())
                    .fill(ui.visuals().extreme_bg_color)
                    .stroke(Stroke::new(
                        1.0,
                        ui.visuals().widgets.noninteractive.bg_stroke.color,
                    ))
                    .corner_radius(8.0)
                    .inner_margin(egui::Margin::same(4))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        ui.spacing_mut().button_padding = egui::vec2(4.0, 2.0);

                        ui.horizontal(|ui| {
                            if ui
                                .button(icons::SEARCH_LINE)
                                .on_hover_text("Go to annotation")
                                .clicked()
                            {
                                msgs.push(Message::GoToAnnotationPosition(id, tile_id));
                            }

                            let vis_icon = if self.is_visible() {
                                icons::EYE_LINE
                            } else {
                                icons::EYE_OFF_LINE
                            };

                            if ui
                                .button(vis_icon)
                                .on_hover_text("Toggle visibility")
                                .clicked()
                            {
                                msgs.push(Message::ToggleAnnotationVisiblility(id));
                            }

                            if ui
                                .button(icons::DELETE_BIN_LINE)
                                .on_hover_text("Delete annotation")
                                .clicked()
                            {
                                msgs.push(Message::RemoveAnnotation(id));
                            }

                            if self.is_visible() {
                                let comment = self.get_comment_box();

                                let chat_icon = if comment.visible {
                                    icons::CHAT_4_LINE
                                } else {
                                    icons::CHAT_OFF_LINE
                                };

                                if ui
                                    .button(chat_icon)
                                    .on_hover_text("Toggle comment visibility")
                                    .clicked()
                                {
                                    msgs.push(Message::ToggleCommentVisibility(id));
                                }
                            }
                        });
                    });
            });
    }

    fn draw_hover_info(
        &self,
        group_name: &str,
        ui: &mut egui::Ui,
        (time_start_str, time_end_str): (&str, &str),
    ) {
        ui.label(format!("Start time: {time_start_str} "));
        ui.label(format!("End time:   {time_end_str} "));
        ui.painter().add(egui::Shape::line_segment(
            [ui.cursor().left_top(), ui.cursor().right_top()],
            egui::Stroke::new(0.2, egui::Color32::LIGHT_GRAY),
        ));
        ui.label(format!("Name: {}", self.get_name()));
        ui.label(format!("Group: {group_name}"));
        ui.label(format!("Type: {}", self.get_type()));
        ui.label(format!("ID: {:?}", self.get_id()));
    }
    fn hide_annotation(&self, ui: &mut egui::Ui, stroke: Stroke, center: Pos2) -> Rect {
        ui.painter()
            .circle_filled(center, DEFAULT_HIDE_RADIUS, stroke.color);

        egui::Rect::from_center_size(
            center,
            egui::vec2(DEFAULT_HIDE_RADIUS * 2.0, DEFAULT_HIDE_RADIUS * 2.0),
        )
    }
    fn get_comment_position(
        &self,
        viewport: &Viewport,
        ctx: &DrawingContext,
        waves: &AnnotationView<'_>,
        offset: f32,
    ) -> Pos2;

    fn draw_comment_box(
        &self,
        ui: &mut egui::Ui,
        msgs: &mut Vec<Message>,
        comment_position: Pos2,
    ) -> (Id, Comment) {
        let mut comment = self.get_comment_box();
        comment.id = ui.make_persistent_id(comment.id);

        comment.name = self.get_name();

        // X-coordinate
        comment.rect.min.x = comment_position.x + comment.offset.x;
        comment.rect.max.x = comment_position.x + comment.offset.x + comment.size.x;

        // Y-coordinate
        comment.rect.min.y = comment_position.y + comment.offset.y;
        comment.rect.max.y = comment_position.y + comment.offset.y + comment.size.y;

        comment.anchor = comment_position;
        ui.add(&mut comment);
        // Handle "Enter" key to submit new comment
        if let Some(save_text) = &comment.save_text {
            msgs.push(Message::AddCommentMessage(
                comment.annotation_id,
                save_text.clone(),
                "user".to_string(),
            ));
        }

        (comment.annotation_id, comment)
    }

    fn update_comment_box(&mut self, comment: Comment) {
        let c = self.get_comment_box_mut();
        c.name = comment.name;
        c.new_text = comment.new_text;
        c.offset = comment.offset;
        c.size = comment.size;
        c.rect = comment.rect;
        c.visible = comment.visible;
    }
}

impl crate::item_list::ItemList {
    pub fn delete_annotation(&mut self, id: egui::Id) {
        self.annotations
            .retain(|annotation| annotation.get_id() != id);
    }

    #[must_use]
    pub fn get_annotation_by_id(&self, id: &egui::Id) -> Option<&Annotation> {
        self.annotations.iter().find(|anno| anno.get_id() == *id)
    }
}

impl AnnotationView<'_> {
    #[allow(clippy::too_many_arguments)]
    pub fn draw_annotations(
        &self,
        ui: &mut egui::Ui,
        tile_id: crate::tiles::TileId,
        ctx: &mut DrawingContext,
        theme: &SurferTheme,
        msgs: &mut Vec<Message>,
        y_offset: f32,
        viewport_rect: egui::Rect,
        to_screen: RectTransform,
        time_formatter: &TimeFormatter,
    ) {
        let mut comment_changes = Vec::new();

        for annotation in &self.items.annotations {
            annotation.draw(
                ui,
                self,
                tile_id,
                ctx,
                theme,
                msgs,
                y_offset,
                to_screen,
                time_formatter,
            );

            if self.selected_annotation == Some(annotation.get_id())
                && let (Some(mut menu_position), Some(menu_time)) =
                    (self.menu_position, self.menu_time)
            {
                menu_position.x = self.viewport.pixel_from_time(
                    menu_time,
                    ctx.cfg.canvas_size.x,
                    self.time_range(),
                );
                let temp_y = menu_position.y;
                menu_position = (ctx.to_screen)(menu_position.x, menu_position.y);
                menu_position.y = temp_y;

                annotation.draw_quick_menu(ui, msgs, tile_id, viewport_rect, menu_position);
            }
        }
        for annotation in &self.items.annotations {
            if annotation.show_comment_box() && annotation.is_visible() {
                let comment_position =
                    annotation.get_comment_position(self.viewport, ctx, self, y_offset);
                let (id, comment) = annotation.draw_comment_box(ui, msgs, comment_position);
                // Only update comment if change has been made or something is being written
                if comment.change || annotation.get_comment_box().new_text != comment.new_text {
                    comment_changes.push((id, comment));
                }
            }
        }
        if !comment_changes.is_empty() {
            msgs.push(Message::UpdateCommentBox(comment_changes));
        }
    }
}

impl SystemState {
    pub(crate) fn go_to_annotation_position(&mut self, anno_id: Id, tile_id: crate::tiles::TileId) {
        if let Some(waves) = self.user.waveform_edit_at(tile_id) {
            if waves.max_timestamp().is_some() {
                if let Some(target) = waves.items.get_annotation_by_id(&anno_id) {
                    let mut left = target.get_start_time();
                    let mut right = target.get_end_time();
                    let from_wave = target.get_from_wave();
                    let to_wave = target.get_to_wave();

                    let difference = (&right - &left) / 2;
                    left -= &difference;
                    right += difference;
                    let range = waves.time_range().clone();
                    waves.view.viewport.zoom_to_range(&left, &right, &range);

                    if let Some(from_wave) = from_wave
                        && let Some(to_wave) = to_wave
                    {
                        if let Some(y_1) = waves.items.get_item_y(&from_wave)
                            && let Some(y_2) = waves.items.get_item_y(&to_wave)
                        {
                            // let y_diff = (y_2 - y_1) * 0.5;
                            // let center = y_1 + y_diff;
                            if let Some(item) = waves.items.get_item_at_y(y_1.min(y_2)) {
                                waves.view.scroll_to_item(waves.items, item.0);
                            }
                        } else {
                            warn!("GoToAnnotationPosition: got None from get_item_y");
                        }
                    } else {
                        warn!("GoToAnnotationPosition: got None from to_wave");
                    }
                }

                self.invalidate_draw_commands();
            } else {
                warn!(
                    "Go to marker position: No timestamps count, even though waveforms should be loaded"
                );
            }
        }
    }

    pub(crate) fn annotation_id(&mut self) -> Id {
        let id = egui::Id::new(("annotation", self.annotation_id_source));
        self.annotation_id_source += 1;
        id
    }
}

#[cfg(test)]
mod view_tests {
    use super::*;

    #[test]
    fn linked_annotation_comments_have_stable_distinct_view_ids() {
        let annotation = RectAnnotation::new(
            Id::new("shared annotation"),
            BigInt::from(0),
            BigInt::from(10),
            None,
            None,
            Rect::ZERO,
            1,
        );
        let original_id = annotation.annotation_data.comment_box.id;
        let ctx = egui::Context::default();
        let render = |order: [usize; 2]| {
            let mut ids = [Id::NULL; 2];
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                for tile in order {
                    let mut pane = ui.new_child(egui::UiBuilder::new().id(Id::new(("tile", tile))));
                    let (annotation_id, comment) =
                        annotation.draw_comment_box(&mut pane, &mut Vec::new(), Pos2::ZERO);
                    assert_eq!(annotation_id, annotation.get_id());
                    ids[tile] = comment.id;
                }
            });
            output.textures_delta.clear();
            ids
        };
        let first = render([0, 1]);
        assert_ne!(first[0], first[1]);
        assert_eq!(first, render([1, 0]));
        assert_eq!(annotation.annotation_data.comment_box.id, original_id);
    }
}
