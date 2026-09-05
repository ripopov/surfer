//! A complete waveform pane, borrowing its document, list and navigation.

use egui::{
    Align, CentralPanel, Frame, Layout, Panel, RichText, ScrollArea, Stroke, TextWrapMode, Ui, Vec2,
};

use super::waveform_services::WaveformReadServices;
use crate::{
    Message,
    drawing_canvas::{CanvasView, WaveDrawCache},
    view::ItemListView,
};

pub(crate) struct WaveformColumns {
    pub focus_ids: bool,
    pub names: Option<f32>,
    pub values: Option<f32>,
}

pub(crate) struct WaveformBodyResponse {
    pub height: f32,
    pub scroll_offset: Option<f32>,
    pub name_width: Option<f32>,
    pub value_width: Option<f32>,
}

/// Keep preferred widths in the workspace, even when a small pane clips them.
/// The resize handle reports user changes without persisting layout constraints.
fn column(
    ui: &mut Ui,
    salt: &str,
    preferred_width: f32,
    frame: Frame,
    contents: impl FnOnce(&mut Ui),
) -> Option<f32> {
    let id = ui.make_persistent_id(salt);
    let max_width = (ui.available_width() * 0.5).max(10.0);
    let width = preferred_width.clamp(10.0, max_width);
    let response = Panel::left(id)
        .frame(frame)
        .resizable(false)
        .exact_size(width)
        .show(ui, contents);
    let rect = response.response.rect;
    let handle = egui::Rect::from_min_max(
        rect.right_top() - egui::vec2(3.0, 0.0),
        rect.right_bottom() + egui::vec2(3.0, 0.0),
    )
    .intersect(ui.clip_rect());
    let resize = ui
        .interact(handle, id.with("resize"), egui::Sense::drag())
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    if (resize.dragged() || resize.drag_stopped())
        && let Some(pointer) = resize.interact_pointer_pos()
    {
        let new_width = (pointer.x - rect.left()).clamp(10.0, max_width);
        return (new_width != preferred_width).then_some(new_width);
    }
    None
}

impl WaveformReadServices<'_> {
    pub(crate) fn draw_waveform_body(
        &self,
        view: &CanvasView<'_>,
        cache: &mut WaveDrawCache,
        columns: WaveformColumns,
        ui: &mut Ui,
        msgs: &mut Vec<Message>,
    ) -> WaveformBodyResponse {
        if ui.rect_contains_pointer(ui.max_rect()) && ui.input(|input| input.pointer.any_pressed())
        {
            msgs.push(Message::SetActiveViewport(view.source.tile_id));
        }
        self.ensure_drawing_infos_cached(view.source.items);
        let column_view = ItemListView {
            document: view.source.document,
            items: view.source.items,
            focused_item: view.source.focused_item,
            tile_id: view.source.tile_id,
        };
        let mut scroll_offset = None;
        let mut name_width = None;
        let mut value_width = None;
        let frame = Frame::default()
            .inner_margin(0)
            .outer_margin(0)
            .fill(self.config.theme.secondary_ui_color.background)
            .stroke(Stroke::NONE);
        if columns.focus_ids {
            Panel::left(ui.make_persistent_id("focus id list"))
                .default_size(40.0)
                .size_range(40.0..=ui.available_width().max(40.0))
                .show(ui, |ui| {
                    ui.add_space(self.default_timeline_offset());
                    let response = ScrollArea::both()
                        .vertical_scroll_offset(view.scroll_offset)
                        .show(ui, |ui| self.draw_item_focus_list(&column_view, ui));
                    if (view.scroll_offset - response.state.offset.y).abs() > 5.0 {
                        scroll_offset = Some(response.state.offset.y);
                    }
                });
        }
        if let Some(width) = columns.names {
            name_width = column(ui, "variable list", width, frame, |ui| {
                ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);
                ui.spacing_mut().item_spacing.y = 0.0;
                if self.show_default_timeline {
                    let header_top = ui.cursor().top();
                    let text_margin = Self::item_text_margin(ui);
                    ui.allocate_ui_with_layout(
                        Vec2::new(ui.available_width(), self.config.layout.waveforms_text_size),
                        Layout::top_down(Align::LEFT),
                        |ui| {
                            ui.horizontal(|ui| {
                                ui.add_space(text_margin.x);
                                ui.label(RichText::new("Time").italics());
                            });
                        },
                    );
                    let padding = header_top + self.default_timeline_offset() - ui.cursor().top();
                    if padding > 0.0 {
                        ui.add_space(padding);
                    }
                }
                let response = ScrollArea::both()
                    .auto_shrink([false; 2])
                    .vertical_scroll_offset(view.scroll_offset)
                    .show(ui, |ui| self.draw_item_list(&column_view, msgs, ui));
                if (view.scroll_offset - response.state.offset.y).abs() > 5.0 {
                    scroll_offset = Some(response.state.offset.y);
                }
            });
        }
        if let Some(width) = columns.values {
            value_width = column(ui, "variable values", width, frame, |ui| {
                ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);
                ui.add_space(self.default_timeline_offset());
                let response = ScrollArea::both()
                    .auto_shrink([false; 2])
                    .vertical_scroll_offset(view.scroll_offset)
                    .show(ui, |ui| self.draw_var_values(&column_view, ui, msgs));
                if (view.scroll_offset - response.state.offset.y).abs() > 5.0 {
                    scroll_offset = Some(response.state.offset.y);
                }
            });
        }
        let mut height = 0.0;
        CentralPanel::default().frame(Frame::NONE).show(ui, |ui| {
            height = (ui.available_height() - self.default_timeline_offset()).max(0.0);
            self.draw_canvas(view, cache, ui, msgs, view.source.tile_id);
        });
        WaveformBodyResponse {
            height,
            scroll_offset,
            name_width,
            value_width,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_resizing_is_local_and_constraints_do_not_change_preferred_widths() {
        let ctx = egui::Context::default();
        let draw = |widths: [f32; 2], pane_width: f32, events: Vec<egui::Event>| {
            let mut changes = [None; 2];
            let mut edges = [egui::Pos2::ZERO; 2];
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 300.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    for index in 0..2 {
                        let rect = egui::Rect::from_min_size(
                            egui::pos2(index as f32 * 400.0, 0.0),
                            egui::vec2(pane_width, 250.0),
                        );
                        let mut pane =
                            ui.new_child(egui::UiBuilder::new().id_salt(index).max_rect(rect));
                        pane.set_clip_rect(rect);
                        changes[index] =
                            column(&mut pane, "names", widths[index], Frame::NONE, |ui| {
                                ScrollArea::both().show(ui, |ui| {
                                    ui.take_available_space();
                                });
                            });
                        column(&mut pane, "values", 60.0, Frame::NONE, |ui| {
                            ScrollArea::both().show(ui, |ui| {
                                ui.take_available_space();
                            });
                        });
                        CentralPanel::default().show(&mut pane, |ui| {
                            ui.allocate_response(ui.available_size(), egui::Sense::drag());
                        });
                        let id = pane.make_persistent_id("names");
                        edges[index] = egui::containers::panel::PanelState::load(&ctx, id)
                            .unwrap()
                            .outer_rect
                            .right_center();
                    }
                },
            );
            output.textures_delta.clear();
            (changes, edges)
        };
        let widths = [100.0, 120.0];
        let (changes, edges) = draw(widths, 380.0, vec![]);
        assert_eq!(changes, [None; 2]);
        let pointer = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        draw(
            widths,
            380.0,
            vec![egui::Event::PointerMoved(edges[0]), pointer(edges[0], true)],
        );
        let dragged = edges[0] + egui::vec2(30.0, 0.0);
        let (changes, _) = draw(widths, 380.0, vec![egui::Event::PointerMoved(dragged)]);
        assert_eq!(changes, [Some(130.0), None]);
        let widths = [changes[0].unwrap(), widths[1]];
        draw(widths, 380.0, vec![pointer(dragged, false)]);
        let (changes, _) = draw(widths, 80.0, vec![]);
        assert_eq!(changes, [None; 2]);
        let (changes, restored) = draw(widths, 380.0, vec![]);
        assert_eq!(changes, [None; 2]);
        assert_eq!(restored[0].x, 130.0);
        assert_eq!(restored[1].x, 520.0);
        // Replacing the workspace's saved preference must override egui's old size.
        let (_, replaced) = draw([150.0, 120.0], 380.0, vec![]);
        assert_eq!(replaced[0].x, 150.0);
    }
}
