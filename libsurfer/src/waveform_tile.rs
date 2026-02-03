//! Waveform tile rendering - draws the main waveform view panels.
//!
//! This module contains the rendering logic for the waveform tile contents:
//! - Variable list panel (left)
//! - Variable values panel (left)
//! - Focus ID list (for command prompt item selection)
//! - Main waveform canvas (center)

use egui::{
    Align, CentralPanel, FontSelection, Frame, Layout, Pos2, Rect, RichText, ScrollArea, Sense,
    SidePanel, TextStyle, Ui, UiBuilder, Vec2, WidgetText,
};
use emath::{GuiRounding, RectTransform};
use epaint::text::LayoutJob;
use epaint::{Margin, Shape, Stroke, text::TextWrapMode};
use itertools::Itertools;
use surfer_translation_types::VariableInfo;

use crate::command_parser::get_parser;
use crate::config::ThemeColorPair;
use crate::displayed_item::{DisplayedFieldRef, DisplayedItem, DisplayedItemRef};
use crate::displayed_item_tree::{ItemIndex, VisibleItemIndex};
use crate::fzcmd::expand_command;
use crate::menus::generic_context_menu;
use crate::message::Message;
use crate::system_state::SystemState;
use crate::time::time_string;
use crate::tooltips::variable_tooltip_text;
use crate::util::get_alpha_focus_id;
use crate::view::{
    DividerDrawingInfo, DrawConfig, DrawingContext, GroupDrawingInfo, ItemDrawingInfo,
    MarkerDrawingInfo, PlaceholderDrawingInfo, StreamDrawingInfo, TimeLineDrawingInfo,
    VariableDrawingInfo, draw_true_name,
};
use crate::wave_container::{FieldRef, FieldRefExt, VariableMeta};

/// Info about a visible item needed for drawing.
struct VisibleItemInfo {
    item_ref: DisplayedItemRef,
    level: u8,
    unfolded: bool,
    vidx: VisibleItemIndex,
    has_children: bool,
    last: bool,
}

impl SystemState {
    /// Draws waveform tile contents (variable list + values + canvas).
    /// Uses show_inside() to render panels within the tile's UI area.
    pub fn draw_waveform_tile(
        &mut self,
        ctx: &egui::Context,
        ui: &mut Ui,
        msgs: &mut Vec<Message>,
    ) {
        let tile_clip = ui.max_rect();
        let max_width = ui.available_width();
        let max_height = ui.available_height();

        let has_waves = self
            .user
            .waves
            .as_ref()
            .is_some_and(|waves| waves.any_displayed());

        if has_waves {
            self.draw_waveform_panels(ctx, ui, msgs, tile_clip, max_width);
        } else {
            self.draw_welcome_screen(ui, max_width, max_height);
        }
    }

    /// Draws all waveform panels when waves are loaded.
    fn draw_waveform_panels(
        &mut self,
        ctx: &egui::Context,
        ui: &mut Ui,
        msgs: &mut Vec<Message>,
        tile_clip: Rect,
        max_width: f32,
    ) {
        let scroll_offset = self.user.waves.as_ref().unwrap().scroll_offset;

        self.draw_focus_id_panel(ui, msgs, scroll_offset, max_width);
        self.draw_variable_list_panel(ctx, ui, msgs, tile_clip, scroll_offset, max_width);
        self.draw_transaction_detail_panel(ui, max_width, msgs);
        self.draw_variable_values_panel(ui, msgs, tile_clip, scroll_offset, max_width);
        self.draw_additional_viewports(ctx, ui, msgs, tile_clip);
        self.draw_main_canvas_panel(ctx, ui, msgs, tile_clip);
    }

    /// Draws the focus ID panel for keyboard-based item selection commands.
    fn draw_focus_id_panel(
        &mut self,
        ui: &mut Ui,
        msgs: &mut Vec<Message>,
        scroll_offset: f32,
        max_width: f32,
    ) {
        let draw_focus_ids = self.command_prompt.visible
            && expand_command(&self.command_prompt_text.borrow(), get_parser(self))
                .expanded
                .starts_with("item_focus");

        if !draw_focus_ids {
            return;
        }

        SidePanel::left(ui.id().with("focus id list"))
            .default_width(40.)
            .width_range(40.0..=max_width)
            .show_inside(ui, |ui| {
                self.handle_pointer_in_ui(ui, msgs);
                let response = ScrollArea::both()
                    .vertical_scroll_offset(scroll_offset)
                    .show(ui, |ui| {
                        self.draw_item_focus_list(ui);
                    });
                self.user.waves.as_mut().unwrap().top_item_draw_offset = response.inner_rect.min.y;
                self.user.waves.as_mut().unwrap().total_height = response.inner_rect.height();
                if (scroll_offset - response.state.offset.y).abs() > 5. {
                    msgs.push(Message::SetScrollOffset(response.state.offset.y));
                }
            });
    }

    /// Draws the variable list panel showing displayed waveform items.
    fn draw_variable_list_panel(
        &mut self,
        ctx: &egui::Context,
        ui: &mut Ui,
        msgs: &mut Vec<Message>,
        tile_clip: Rect,
        scroll_offset: f32,
        max_width: f32,
    ) {
        SidePanel::left(ui.id().with("variable list"))
            .default_width(100.)
            .width_range(100.0..=max_width)
            .show_inside(ui, |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(tile_clip));
                ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);
                self.handle_pointer_in_ui(ui, msgs);
                if self.show_default_timeline() {
                    ui.label(RichText::new("Time").italics());
                }

                let response = ScrollArea::both()
                    .auto_shrink([false; 2])
                    .vertical_scroll_offset(scroll_offset)
                    .show(ui, |ui| {
                        self.draw_item_list(msgs, ui, ctx);
                    });
                self.user.waves.as_mut().unwrap().top_item_draw_offset = response.inner_rect.min.y;
                self.user.waves.as_mut().unwrap().total_height = response.inner_rect.height();
                if (scroll_offset - response.state.offset.y).abs() > 5. {
                    msgs.push(Message::SetScrollOffset(response.state.offset.y));
                }
            });
    }

    /// Draws the variable values panel showing current values at cursor position.
    fn draw_variable_values_panel(
        &mut self,
        ui: &mut Ui,
        msgs: &mut Vec<Message>,
        tile_clip: Rect,
        scroll_offset: f32,
        max_width: f32,
    ) {
        SidePanel::left(ui.id().with("variable values"))
            .frame(egui::Frame::default().inner_margin(0).outer_margin(0))
            .default_width(100.)
            .width_range(10.0..=max_width)
            .show_inside(ui, |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(tile_clip));
                // Draw background manually; Frame::fill draws before we can clamp.
                ui.painter().with_clip_rect(ui.clip_rect()).rect_filled(
                    ui.max_rect(),
                    0.0,
                    self.user.config.theme.secondary_ui_color.background,
                );
                ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);
                self.handle_pointer_in_ui(ui, msgs);
                let response = ScrollArea::both()
                    .auto_shrink([false; 2])
                    .vertical_scroll_offset(scroll_offset)
                    .show(ui, |ui| self.draw_var_values(ui, msgs));
                if (scroll_offset - response.state.offset.y).abs() > 5. {
                    msgs.push(Message::SetScrollOffset(response.state.offset.y));
                }
            });
    }

    /// Draws additional viewports when multiple viewports are configured.
    fn draw_additional_viewports(
        &mut self,
        ctx: &egui::Context,
        ui: &mut Ui,
        msgs: &mut Vec<Message>,
        tile_clip: Rect,
    ) {
        let number_of_viewports = self.user.waves.as_ref().unwrap().viewports.len();
        if number_of_viewports <= 1 {
            return;
        }

        let available_width = ui.available_width();
        let default_width = available_width / (number_of_viewports as f32);
        let viewport_stroke = Stroke::from(&self.user.config.theme.viewport_separator);
        let std_stroke = ui.style().visuals.widgets.noninteractive.bg_stroke;
        ui.style_mut().visuals.widgets.noninteractive.bg_stroke = viewport_stroke;

        for viewport_idx in 1..number_of_viewports {
            SidePanel::right(ui.id().with(format!("view port {viewport_idx}")))
                .default_width(default_width)
                .width_range(30.0..=available_width)
                .frame(Frame {
                    inner_margin: Margin::ZERO,
                    outer_margin: Margin::ZERO,
                    ..Default::default()
                })
                .show_inside(ui, |ui| {
                    ui.set_clip_rect(ui.clip_rect().intersect(tile_clip));
                    self.draw_items(ctx, msgs, ui, viewport_idx);
                });
        }

        ui.style_mut().visuals.widgets.noninteractive.bg_stroke = std_stroke;
    }

    /// Draws the main waveform canvas panel.
    fn draw_main_canvas_panel(
        &mut self,
        ctx: &egui::Context,
        ui: &mut Ui,
        msgs: &mut Vec<Message>,
        tile_clip: Rect,
    ) {
        CentralPanel::default()
            .frame(Frame {
                inner_margin: Margin::ZERO,
                outer_margin: Margin::ZERO,
                ..Default::default()
            })
            .show_inside(ui, |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(tile_clip));
                self.draw_items(ctx, msgs, ui, 0);
            });
    }

    /// Draws the welcome screen when no waves are loaded.
    fn draw_welcome_screen(&mut self, ui: &mut Ui, max_width: f32, max_height: f32) {
        CentralPanel::default()
            .frame(Frame::NONE.fill(self.user.config.theme.canvas_colors.background))
            .show_inside(ui, |ui| {
                ui.add_space(max_height * 0.1);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("🏄 Surfer").monospace().size(24.));
                    ui.add_space(20.);
                    let layout = Layout::top_down(Align::LEFT);
                    ui.allocate_ui_with_layout(
                        Vec2 {
                            x: max_width * 0.35,
                            y: max_height * 0.5,
                        },
                        layout,
                        |ui| self.help_message(ui),
                    );
                });
            });
    }

    /// Add bottom padding so the last item isn't clipped or covered by the scrollbar.
    fn add_padding_for_last_item(
        ui: &mut Ui,
        last_info: Option<&ItemDrawingInfo>,
        line_height: f32,
    ) {
        if let Some(info) = last_info {
            let target_bottom = info.bottom() + line_height;
            let next_y = ui.cursor().top();
            if next_y < target_bottom {
                ui.add_space(target_bottom - next_y);
            }
        }
    }

    /// Draws the focus ID list for keyboard-based item selection.
    /// Shows alphabetic IDs (a, b, c...) next to each visible item when
    /// the command prompt is active with an item_focus command.
    fn draw_item_focus_list(&self, ui: &mut Ui) {
        let Some(waves) = self.user.waves.as_ref() else {
            return;
        };
        let alignment = self.get_name_alignment();
        ui.with_layout(
            Layout::top_down(alignment).with_cross_justify(false),
            |ui| {
                if self.show_default_timeline() {
                    ui.add_space(ui.text_style_height(&TextStyle::Body) + 2.0);
                }
                // drawing_infos accounts for height_scaling_factor
                for drawing_info in waves.drawing_infos.iter() {
                    let next_y = ui.cursor().top();
                    // Align with the corresponding row in other panels
                    if next_y < drawing_info.top() {
                        ui.add_space(drawing_info.top() - next_y);
                    }
                    let vidx = drawing_info.vidx();
                    ui.scope(|ui| {
                        ui.style_mut().visuals.selection.bg_fill =
                            self.user.config.theme.accent_warn.background;
                        ui.style_mut().visuals.override_text_color =
                            Some(self.user.config.theme.accent_warn.foreground);
                        let _ = ui.selectable_label(true, get_alpha_focus_id(vidx, waves));
                    });
                }
                Self::add_padding_for_last_item(
                    ui,
                    waves.drawing_infos.last(),
                    self.user.config.layout.waveforms_line_height,
                );
            },
        );
    }

    /// Draws the variable/item list panel showing all displayed waveform items.
    fn draw_item_list(&mut self, msgs: &mut Vec<Message>, ui: &mut Ui, ctx: &egui::Context) {
        let mut item_offsets = Vec::new();
        let any_groups = self.has_any_groups();
        let alignment = self.get_name_alignment();

        ui.with_layout(Layout::top_down(alignment).with_cross_justify(true), |ui| {
            let available_rect = ui.available_rect_before_wrap();

            for info in self.collect_visible_items() {
                let Some(displayed_item) = self
                    .user
                    .waves
                    .as_ref()
                    .unwrap()
                    .displayed_items
                    .get(&info.item_ref)
                else {
                    continue;
                };

                self.draw_item_row(
                    ui,
                    ctx,
                    msgs,
                    &mut item_offsets,
                    &info,
                    displayed_item,
                    alignment,
                    any_groups,
                    available_rect,
                );
            }

            Self::add_padding_for_last_item(
                ui,
                item_offsets.last(),
                self.user.config.layout.waveforms_line_height,
            );
        });

        self.user.waves.as_mut().unwrap().drawing_infos = item_offsets;

        let response = ui.allocate_response(ui.available_size(), Sense::click());
        generic_context_menu(msgs, &response);
    }

    /// Checks if there are any grouped items in the tree.
    fn has_any_groups(&self) -> bool {
        self.user
            .waves
            .as_ref()
            .unwrap()
            .items_tree
            .iter()
            .any(|node| node.level > 0)
    }

    /// Collects visible item info for iteration.
    fn collect_visible_items(&self) -> Vec<VisibleItemInfo> {
        self.user
            .waves
            .as_ref()
            .unwrap()
            .items_tree
            .iter_visible_extra()
            .map(|info| VisibleItemInfo {
                item_ref: info.node.item_ref,
                level: info.node.level,
                unfolded: info.node.unfolded,
                vidx: info.vidx,
                has_children: info.has_children,
                last: info.last,
            })
            .collect()
    }

    /// Draws a single item row including hierarchy icon and content.
    #[allow(clippy::too_many_arguments)]
    fn draw_item_row(
        &self,
        ui: &mut Ui,
        ctx: &egui::Context,
        msgs: &mut Vec<Message>,
        item_offsets: &mut Vec<ItemDrawingInfo>,
        info: &VisibleItemInfo,
        displayed_item: &DisplayedItem,
        alignment: Align,
        any_groups: bool,
        available_rect: Rect,
    ) {
        let row_layout = if alignment == Align::LEFT {
            Layout::left_to_right(Align::TOP)
        } else {
            Layout::right_to_left(Align::TOP)
        };

        ui.with_layout(row_layout, |ui| {
            ui.add_space(10.0 * f32::from(info.level));

            self.draw_hierarchy_icon_with_action(ui, msgs, info, any_groups, alignment);

            let item_rect = self.draw_item_content(
                ui,
                ctx,
                msgs,
                item_offsets,
                info,
                displayed_item,
                alignment,
            );

            self.draw_item_drag_target(ui, msgs, info, item_rect, available_rect);
        });
    }

    /// Draws hierarchy icon and handles fold/unfold clicks.
    fn draw_hierarchy_icon_with_action(
        &self,
        ui: &mut Ui,
        msgs: &mut Vec<Message>,
        info: &VisibleItemInfo,
        any_groups: bool,
        alignment: Align,
    ) {
        if !any_groups {
            return;
        }

        let response = self.hierarchy_icon(ui, info.has_children, info.unfolded, alignment);
        if response.clicked() {
            let msg = if info.unfolded {
                Message::GroupFold(Some(info.item_ref))
            } else {
                Message::GroupUnfold(Some(info.item_ref))
            };
            msgs.push(msg);
        }
    }

    /// Draws the item content based on its type.
    #[allow(clippy::too_many_arguments)]
    fn draw_item_content(
        &self,
        ui: &mut Ui,
        ctx: &egui::Context,
        msgs: &mut Vec<Message>,
        item_offsets: &mut Vec<ItemDrawingInfo>,
        info: &VisibleItemInfo,
        displayed_item: &DisplayedItem,
        alignment: Align,
    ) -> Rect {
        match displayed_item {
            DisplayedItem::Variable(displayed_variable) => {
                let levels_to_force_expand =
                    self.items_to_expand
                        .borrow()
                        .iter()
                        .find_map(|(id, levels)| {
                            if info.item_ref == *id {
                                Some(*levels)
                            } else {
                                None
                            }
                        });

                self.draw_variable(
                    msgs,
                    info.vidx,
                    displayed_item,
                    info.item_ref,
                    FieldRef::without_fields(displayed_variable.variable_ref.clone()),
                    item_offsets,
                    &displayed_variable.info,
                    ui,
                    ctx,
                    levels_to_force_expand,
                    alignment,
                )
            }
            _ => {
                ui.with_layout(
                    ui.layout()
                        .with_main_justify(true)
                        .with_main_align(alignment),
                    |ui| {
                        self.draw_plain_item(
                            msgs,
                            info.vidx,
                            info.item_ref,
                            displayed_item,
                            item_offsets,
                            ui,
                            ctx,
                        )
                    },
                )
                .inner
            }
        }
    }

    /// Draws drag target indicator for item reordering.
    fn draw_item_drag_target(
        &self,
        ui: &mut Ui,
        msgs: &mut Vec<Message>,
        info: &VisibleItemInfo,
        item_rect: Rect,
        available_rect: Rect,
    ) {
        let mut expanded_rect = item_rect;
        expanded_rect.set_left(
            available_rect.left()
                + self.user.config.layout.waveforms_text_size
                + ui.spacing().item_spacing.x,
        );
        expanded_rect.set_right(available_rect.right());
        self.draw_drag_target(
            msgs,
            info.vidx,
            expanded_rect,
            available_rect,
            ui,
            info.last,
        );
    }

    /// Draws the variable values panel showing current values at cursor position.
    fn draw_var_values(&self, ui: &mut Ui, msgs: &mut Vec<Message>) {
        let Some(waves) = &self.user.waves else {
            return;
        };
        let response = ui.allocate_response(ui.available_size(), Sense::click());
        generic_context_menu(msgs, &response);

        let rect = response.rect;
        self.draw_values_background_layer(ui, waves, rect);

        let ucursor = waves.cursor.as_ref().and_then(num::BigInt::to_biguint);
        let rect_with_margin = Rect {
            min: rect.min + ui.spacing().item_spacing,
            max: rect.max + Vec2::new(0.0, 40.0),
        };

        let builder = UiBuilder::new().max_rect(rect_with_margin);
        ui.scope_builder(builder, |ui| {
            ui.style_mut().override_text_style = Some(TextStyle::Monospace);

            for (item_idx, drawing_info) in waves
                .drawing_infos
                .iter()
                .sorted_by_key(|o| o.top() as i32)
                .enumerate()
            {
                self.align_to_drawing_info(ui, drawing_info);
                let bg_color = self.get_background_color(waves, drawing_info.vidx(), item_idx);
                self.draw_value_for_item(ui, msgs, waves, drawing_info, ucursor.as_ref(), bg_color);
            }

            Self::add_padding_for_last_item(
                ui,
                waves.drawing_infos.last(),
                self.user.config.layout.waveforms_line_height,
            );
        });
    }

    /// Draws background colors for all items in the values panel.
    fn draw_values_background_layer(
        &self,
        ui: &Ui,
        waves: &crate::wave_data::WaveData,
        rect: Rect,
    ) {
        let mut painter = ui.painter().clone();
        let container_rect = Rect::from_min_size(Pos2::ZERO, rect.size());
        let to_screen = RectTransform::from_to(container_rect, rect);
        let cfg = DrawConfig::new(
            rect.height(),
            self.user.config.layout.waveforms_line_height,
            self.user.config.layout.waveforms_text_size,
        );
        let frame_width = rect.width();
        let gap = ui.spacing().item_spacing.y * 0.5;
        let y_zero = to_screen.transform_pos(Pos2::ZERO).y;

        let ctx = DrawingContext {
            painter: &mut painter,
            cfg: &cfg,
            // This 0.5 fixes lines being smushed across two pixels
            // https://github.com/emilk/egui/issues/1322
            to_screen: &|x, y| to_screen.transform_pos(Pos2::new(x, y) + Vec2::new(0.5, 0.5)),
            theme: &self.user.config.theme,
        };

        for (item_count, drawing_info) in waves
            .drawing_infos
            .iter()
            .sorted_by_key(|o| o.top() as i32)
            .enumerate()
        {
            let bg_color = self.get_background_color(waves, drawing_info.vidx(), item_count);
            self.draw_background(drawing_info, y_zero, &ctx, gap, frame_width, bg_color);
        }
    }

    /// Aligns the UI cursor to match the drawing info's vertical position.
    fn align_to_drawing_info(&self, ui: &mut Ui, drawing_info: &ItemDrawingInfo) {
        let next_y = ui.cursor().top();
        if next_y < drawing_info.top() {
            ui.add_space(drawing_info.top() - next_y);
        }
    }

    /// Draws the value display for a single item based on its type.
    fn draw_value_for_item(
        &self,
        ui: &mut Ui,
        msgs: &mut Vec<Message>,
        waves: &crate::wave_data::WaveData,
        drawing_info: &ItemDrawingInfo,
        ucursor: Option<&num::BigUint>,
        bg_color: ecolor::Color32,
    ) {
        match drawing_info {
            ItemDrawingInfo::Variable(var_info) => {
                self.draw_variable_value(ui, msgs, waves, var_info, ucursor, bg_color);
            }
            ItemDrawingInfo::Marker(marker_info) => {
                self.draw_marker_value(ui, msgs, waves, marker_info, bg_color);
            }
            ItemDrawingInfo::Divider(_)
            | ItemDrawingInfo::TimeLine(_)
            | ItemDrawingInfo::Stream(_)
            | ItemDrawingInfo::Group(_)
            | ItemDrawingInfo::Placeholder(_) => {
                ui.label("");
            }
        }
    }

    /// Draws the current value for a variable at the cursor position.
    fn draw_variable_value(
        &self,
        ui: &mut Ui,
        msgs: &mut Vec<Message>,
        waves: &crate::wave_data::WaveData,
        var_info: &VariableDrawingInfo,
        ucursor: Option<&num::BigUint>,
        bg_color: ecolor::Color32,
    ) {
        let Some(ucursor) = ucursor else {
            ui.label("");
            return;
        };

        let Some(value) =
            self.get_variable_value(waves, &var_info.displayed_field_ref, Some(ucursor))
        else {
            return;
        };

        let text_color = self.user.config.theme.get_best_text_color(bg_color);
        ui.label(
            RichText::new(value)
                .color(text_color)
                .line_height(Some(self.user.config.layout.waveforms_line_height)),
        )
        .context_menu(|ui| {
            self.item_context_menu(
                Some(&FieldRef::without_fields(var_info.field_ref.root.clone())),
                msgs,
                ui,
                var_info.vidx,
                true,
                crate::message::MessageTarget::CurrentSelection,
            );
        });
    }

    /// Draws the time delta for a marker relative to the cursor.
    fn draw_marker_value(
        &self,
        ui: &mut Ui,
        msgs: &mut Vec<Message>,
        waves: &crate::wave_data::WaveData,
        marker_info: &MarkerDrawingInfo,
        bg_color: ecolor::Color32,
    ) {
        let Some(cursor) = &waves.cursor else {
            ui.label("");
            return;
        };

        let delta = time_string(
            &(waves.numbered_marker_time(marker_info.idx) - cursor),
            &waves.inner.metadata().timescale,
            &self.user.wanted_timeunit,
            &self.get_time_format(),
        );

        let text_color = self.user.config.theme.get_best_text_color(bg_color);
        ui.label(RichText::new(format!("Δ: {delta}")).color(text_color))
            .context_menu(|ui| {
                self.item_context_menu(
                    None,
                    msgs,
                    ui,
                    marker_info.vidx,
                    true,
                    crate::message::MessageTarget::CurrentSelection,
                );
            });
    }

    fn hierarchy_icon(
        &self,
        ui: &mut Ui,
        has_children: bool,
        unfolded: bool,
        alignment: Align,
    ) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(
            Vec2::splat(self.user.config.layout.waveforms_text_size),
            Sense::click(),
        );
        if !has_children {
            return response;
        }

        // fixme: use the much nicer remixicon arrow? do a layout here and paint the galley into the rect?
        // or alternatively: change how the tree iterator works and use the egui facilities (cross widget?)
        let icon_rect = Rect::from_center_size(
            rect.center(),
            emath::vec2(rect.width(), rect.height()) * 0.75,
        );
        let mut points = vec![
            icon_rect.left_top(),
            icon_rect.right_top(),
            icon_rect.center_bottom(),
        ];
        let rotation = emath::Rot2::from_angle(if unfolded {
            0.0
        } else if alignment == Align::LEFT {
            -std::f32::consts::TAU / 4.0
        } else {
            std::f32::consts::TAU / 4.0
        });
        for p in &mut points {
            *p = icon_rect.center() + rotation * (*p - icon_rect.center());
        }

        let style = ui.style().interact(&response);
        ui.painter().add(Shape::convex_polygon(
            points,
            style.fg_stroke.color,
            Stroke::NONE,
        ));
        response
    }

    fn get_name_alignment(&self) -> Align {
        if self.align_names_right() {
            Align::RIGHT
        } else {
            Align::LEFT
        }
    }

    fn draw_drag_source(
        &self,
        msgs: &mut Vec<Message>,
        vidx: VisibleItemIndex,
        item_response: &egui::Response,
        modifiers: egui::Modifiers,
    ) {
        if item_response.dragged_by(egui::PointerButton::Primary)
            && item_response.drag_delta().length() > self.user.config.theme.drag_threshold
        {
            if !modifiers.ctrl
                && !(self.user.waves.as_ref())
                    .and_then(|w| w.items_tree.get_visible(vidx))
                    .is_some_and(|i| i.selected)
            {
                msgs.push(Message::FocusItem(vidx));
                msgs.push(Message::ItemSelectionClear);
            }
            msgs.push(Message::SetItemSelected(vidx, true));
            msgs.push(Message::VariableDragStarted(vidx));
        }

        if item_response.drag_stopped()
            && self
                .user
                .drag_source_idx
                .is_some_and(|source_idx| source_idx == vidx)
        {
            msgs.push(Message::VariableDragFinished);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_variable_label(
        &self,
        vidx: VisibleItemIndex,
        displayed_item: &DisplayedItem,
        displayed_id: DisplayedItemRef,
        field: FieldRef,
        msgs: &mut Vec<Message>,
        ui: &mut Ui,
        ctx: &egui::Context,
        meta: Option<&VariableMeta>,
    ) -> egui::Response {
        let mut variable_label = self.draw_item_label(
            vidx,
            displayed_id,
            displayed_item,
            Some(&field),
            msgs,
            ui,
            ctx,
            meta,
        );

        if self.show_tooltip() {
            variable_label = variable_label.on_hover_ui(|ui| {
                let tooltip = if self.user.waves.is_some() {
                    if field.field.is_empty() {
                        if let Some(meta) = meta {
                            variable_tooltip_text(Some(meta), &field.root)
                        } else {
                            let wave_container =
                                self.user.waves.as_ref().unwrap().inner.as_waves().unwrap();
                            let meta = wave_container.variable_meta(&field.root).ok();
                            variable_tooltip_text(meta.as_ref(), &field.root)
                        }
                    } else {
                        "From translator".to_string()
                    }
                } else {
                    "No waveform loaded".to_string()
                };
                ui.set_max_width(ui.spacing().tooltip_width);
                ui.add(egui::Label::new(tooltip));
            });
        }

        variable_label
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_variable(
        &self,
        msgs: &mut Vec<Message>,
        vidx: VisibleItemIndex,
        displayed_item: &DisplayedItem,
        displayed_id: DisplayedItemRef,
        field: FieldRef,
        drawing_infos: &mut Vec<ItemDrawingInfo>,
        info: &VariableInfo,
        ui: &mut Ui,
        ctx: &egui::Context,
        levels_to_force_expand: Option<usize>,
        alignment: Align,
    ) -> Rect {
        let displayed_field_ref = DisplayedFieldRef {
            item: displayed_id,
            field: field.field.clone(),
        };
        match info {
            VariableInfo::Compound { subfields } => {
                let mut header = egui::collapsing_header::CollapsingState::load_with_default_open(
                    ui.ctx(),
                    egui::Id::new(&field),
                    false,
                );

                if let Some(level) = levels_to_force_expand {
                    header.set_open(level > 0);
                }

                let response = ui
                    .with_layout(Layout::top_down(alignment).with_cross_justify(true), |ui| {
                        header
                            .show_header(ui, |ui| {
                                ui.with_layout(
                                    Layout::top_down(alignment).with_cross_justify(true),
                                    |ui| {
                                        self.draw_variable_label(
                                            vidx,
                                            displayed_item,
                                            displayed_id,
                                            field.clone(),
                                            msgs,
                                            ui,
                                            ctx,
                                            None,
                                        )
                                    },
                                );
                            })
                            .body(|ui| {
                                for (name, info) in subfields {
                                    let mut new_path = field.clone();
                                    new_path.field.push(name.clone());
                                    ui.with_layout(
                                        Layout::top_down(alignment).with_cross_justify(true),
                                        |ui| {
                                            self.draw_variable(
                                                msgs,
                                                vidx,
                                                displayed_item,
                                                displayed_id,
                                                new_path,
                                                drawing_infos,
                                                info,
                                                ui,
                                                ctx,
                                                levels_to_force_expand.map(|l| l.saturating_sub(1)),
                                                alignment,
                                            );
                                        },
                                    );
                                }
                            })
                    })
                    .inner;
                drawing_infos.push(ItemDrawingInfo::Variable(VariableDrawingInfo {
                    displayed_field_ref,
                    field_ref: field.clone(),
                    vidx,
                    top: response.0.rect.top(),
                    bottom: response.0.rect.bottom(),
                }));
                response.0.rect
            }
            VariableInfo::Bool
            | VariableInfo::Bits
            | VariableInfo::Clock
            | VariableInfo::String
            | VariableInfo::Event
            | VariableInfo::Real => {
                let label = ui
                    .with_layout(Layout::top_down(alignment).with_cross_justify(true), |ui| {
                        self.draw_variable_label(
                            vidx,
                            displayed_item,
                            displayed_id,
                            field.clone(),
                            msgs,
                            ui,
                            ctx,
                            None,
                        )
                    })
                    .inner;
                self.draw_drag_source(msgs, vidx, &label, ctx.input(|e| e.modifiers));
                drawing_infos.push(ItemDrawingInfo::Variable(VariableDrawingInfo {
                    displayed_field_ref,
                    field_ref: field.clone(),
                    vidx,
                    top: label.rect.top(),
                    bottom: label.rect.bottom(),
                }));
                label.rect
            }
        }
    }

    fn draw_drag_target(
        &self,
        msgs: &mut Vec<Message>,
        vidx: VisibleItemIndex,
        expanded_rect: Rect,
        available_rect: Rect,
        ui: &mut Ui,
        last: bool,
    ) {
        if !self.user.drag_started || self.user.drag_source_idx.is_none() {
            return;
        }

        let waves = self
            .user
            .waves
            .as_ref()
            .expect("waves not available, but expected");

        // expanded_rect is just for the label, leaving us with gaps between lines
        // expand to counter that
        let rect_with_margin = expanded_rect.expand2(ui.spacing().item_spacing / 2f32);

        // collision check rect need to be
        // - limited to half the height of the item text
        // - extended to cover the empty space to the left
        // - for the last element, expanded till the bottom
        let before_rect = rect_with_margin
            .with_max_y(rect_with_margin.left_center().y)
            .with_min_x(available_rect.left())
            .round_to_pixels(ui.painter().pixels_per_point());
        let after_rect = if last {
            rect_with_margin.with_max_y(ui.max_rect().max.y)
        } else {
            rect_with_margin
        }
        .with_min_y(rect_with_margin.left_center().y)
        .with_min_x(available_rect.left())
        .round_to_pixels(ui.painter().pixels_per_point());

        let (insert_vidx, line_y) = if ui.rect_contains_pointer(before_rect) {
            (vidx, rect_with_margin.top())
        } else if ui.rect_contains_pointer(after_rect) {
            (VisibleItemIndex(vidx.0 + 1), rect_with_margin.bottom())
        } else {
            return;
        };

        let level_range = waves.items_tree.valid_levels_visible(insert_vidx, |node| {
            matches!(
                waves.displayed_items.get(&node.item_ref),
                Some(DisplayedItem::Group(..))
            )
        });

        let left_x = |level: u8| -> f32 { rect_with_margin.left() + f32::from(level) * 10.0 };
        let Some(insert_level) = level_range.find_or_last(|&level| {
            let mut rect = expanded_rect.with_min_x(left_x(level));
            rect.set_width(10.0);
            if level == 0 {
                rect.set_left(available_rect.left());
            }
            ui.rect_contains_pointer(rect)
        }) else {
            return;
        };

        ui.painter().line_segment(
            [
                Pos2::new(left_x(insert_level), line_y),
                Pos2::new(rect_with_margin.right(), line_y),
            ],
            Stroke::new(
                self.user.config.theme.linewidth,
                self.user.config.theme.drag_hint_color,
            ),
        );
        msgs.push(Message::VariableDragTargetChanged(
            crate::displayed_item_tree::TargetPosition {
                before: ItemIndex(
                    waves
                        .items_tree
                        .to_displayed(insert_vidx)
                        .map_or_else(|| waves.items_tree.len(), |index| index.0),
                ),
                level: insert_level,
            },
        ));
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_item_label(
        &self,
        vidx: VisibleItemIndex,
        displayed_id: DisplayedItemRef,
        displayed_item: &DisplayedItem,
        field: Option<&FieldRef>,
        msgs: &mut Vec<Message>,
        ui: &mut Ui,
        ctx: &egui::Context,
        meta: Option<&VariableMeta>,
    ) -> egui::Response {
        let color_pair = self.get_item_color_pair(vidx, displayed_id, displayed_item);
        ui.style_mut().visuals.selection.bg_fill = color_pair.background;

        let layout_job =
            self.build_item_layout_job(ui, displayed_item, field, color_pair.foreground, meta);

        let item_label = ui
            .selectable_label(
                self.item_is_selected(displayed_id) || self.item_is_focused(vidx),
                WidgetText::LayoutJob(layout_job.into()),
            )
            .interact(Sense::drag());

        if item_label.clicked() || item_label.secondary_clicked() {
            self.handle_item_label_click(msgs, vidx, displayed_id, &item_label, ctx);
        }

        item_label.context_menu(|ui| {
            self.item_context_menu(
                field,
                msgs,
                ui,
                vidx,
                true,
                crate::message::MessageTarget::CurrentSelection,
            );
        });

        item_label
    }

    /// Returns the color pair for an item based on its focus/selection state.
    fn get_item_color_pair(
        &self,
        vidx: VisibleItemIndex,
        displayed_id: DisplayedItemRef,
        displayed_item: &DisplayedItem,
    ) -> ThemeColorPair {
        let theme = &self.user.config.theme;
        if self.item_is_focused(vidx) {
            ThemeColorPair {
                background: theme.accent_info.background,
                foreground: theme.accent_info.foreground,
            }
        } else if self.item_is_selected(displayed_id) {
            ThemeColorPair {
                background: theme.selected_elements_colors.background,
                foreground: theme.selected_elements_colors.foreground,
            }
        } else if matches!(
            displayed_item,
            DisplayedItem::Variable(_) | DisplayedItem::Placeholder(_)
        ) {
            ThemeColorPair {
                background: theme.primary_ui_color.background,
                foreground: theme.primary_ui_color.foreground,
            }
        } else {
            ThemeColorPair {
                background: theme.primary_ui_color.background,
                foreground: self.get_item_text_color(displayed_item),
            }
        }
    }

    /// Builds the layout job for rendering an item's label text.
    fn build_item_layout_job(
        &self,
        ui: &mut Ui,
        displayed_item: &DisplayedItem,
        field: Option<&FieldRef>,
        foreground: ecolor::Color32,
        meta: Option<&VariableMeta>,
    ) -> LayoutJob {
        let mut layout_job = LayoutJob::default();

        match displayed_item {
            DisplayedItem::Variable(var) if field.is_some() => {
                let field = field.unwrap();
                if field.field.is_empty() {
                    self.build_variable_root_layout(
                        ui,
                        var,
                        field,
                        foreground,
                        meta,
                        &mut layout_job,
                    );
                } else {
                    self.build_variable_field_layout(ui, field, foreground, &mut layout_job);
                }
            }
            _ => displayed_item.add_to_layout_job(
                foreground,
                ui.style(),
                &mut layout_job,
                field,
                &self.user.config,
            ),
        }

        layout_job
    }

    /// Builds layout for a variable's root name (not a subfield).
    fn build_variable_root_layout(
        &self,
        ui: &mut Ui,
        var: &crate::displayed_item::DisplayedVariable,
        field: &FieldRef,
        foreground: ecolor::Color32,
        meta: Option<&VariableMeta>,
        layout_job: &mut LayoutJob,
    ) {
        let name_info = self.get_variable_name_info(&var.variable_ref, meta);

        if let Some(true_name) = name_info.and_then(|info| info.true_name) {
            let monospace_font = ui.style().text_styles.get(&TextStyle::Monospace).unwrap();
            let monospace_width = ui.fonts_mut(|fonts| {
                fonts
                    .layout_no_wrap(
                        " ".to_string(),
                        monospace_font.clone(),
                        ecolor::Color32::BLACK,
                    )
                    .size()
                    .x
            });
            let available_width = ui.available_width();

            draw_true_name(
                &true_name,
                layout_job,
                monospace_font.clone(),
                foreground,
                monospace_width,
                available_width,
            );
        } else {
            DisplayedItem::Variable(var.clone()).add_to_layout_job(
                foreground,
                ui.style(),
                layout_job,
                Some(field),
                &self.user.config,
            );
        }
    }

    /// Builds layout for a variable's subfield name.
    fn build_variable_field_layout(
        &self,
        ui: &Ui,
        field: &FieldRef,
        foreground: ecolor::Color32,
        layout_job: &mut LayoutJob,
    ) {
        RichText::new(field.field.last().unwrap().clone())
            .color(foreground)
            .line_height(Some(self.user.config.layout.waveforms_line_height))
            .append_to(
                layout_job,
                ui.style(),
                FontSelection::Default,
                Align::Center,
            );
    }

    /// Handles click interactions on an item label (selection, focus, modifiers).
    ///
    /// Click behavior:
    /// - Primary click on single selected item: deselects it
    /// - Primary/secondary click otherwise: selects just the clicked item
    /// - Secondary click on selection: no change
    /// - Shift+click: selects range from focused to clicked
    /// - Ctrl+click: toggles selection of the item
    fn handle_item_label_click(
        &self,
        msgs: &mut Vec<Message>,
        vidx: VisibleItemIndex,
        displayed_id: DisplayedItemRef,
        item_label: &egui::Response,
        ctx: &egui::Context,
    ) {
        let focused_item = self.user.waves.as_ref().and_then(|w| w.focused_item);
        let is_selected = self.item_is_selected(displayed_id);
        let single_selected = self
            .user
            .waves
            .as_ref()
            .map(|w| w.items_tree.iter_visible_selected().count() == 1)
            .unwrap_or(false);

        let modifiers = ctx.input(|i| i.modifiers);
        tracing::trace!(
            focused_item=?focused_item,
            is_selected=?is_selected,
            single_selected=?single_selected,
            modifiers=?modifiers
        );

        // Deselect if clicking the only selected item
        if item_label.clicked() && is_selected && single_selected {
            msgs.push(Message::Batch(vec![
                Message::ItemSelectionClear,
                Message::UnfocusItem,
            ]));
            return;
        }

        let clicked = item_label.clicked();
        match (clicked, modifiers.command, modifiers.shift) {
            (false, false, false) if is_selected => {}
            (_, false, false) => {
                msgs.push(Message::Batch(vec![
                    Message::ItemSelectionClear,
                    Message::SetItemSelected(vidx, true),
                    Message::FocusItem(vidx),
                ]));
            }
            (_, _, true) => {
                msgs.push(Message::Batch(vec![
                    Message::ItemSelectRange(vidx),
                    Message::FocusItem(vidx),
                ]));
            }
            (_, true, false) => {
                if !is_selected {
                    msgs.push(Message::Batch(vec![
                        Message::SetItemSelected(vidx, true),
                        Message::FocusItem(vidx),
                    ]));
                } else if clicked {
                    msgs.push(Message::Batch(vec![
                        Message::SetItemSelected(vidx, false),
                        Message::UnfocusItem,
                    ]));
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_plain_item(
        &self,
        msgs: &mut Vec<Message>,
        vidx: VisibleItemIndex,
        displayed_id: DisplayedItemRef,
        displayed_item: &DisplayedItem,
        drawing_infos: &mut Vec<ItemDrawingInfo>,
        ui: &mut Ui,
        ctx: &egui::Context,
    ) -> Rect {
        let label = self.draw_item_label(
            vidx,
            displayed_id,
            displayed_item,
            None,
            msgs,
            ui,
            ctx,
            None,
        );

        self.draw_drag_source(msgs, vidx, &label, ui.ctx().input(|e| e.modifiers));
        match displayed_item {
            DisplayedItem::Divider(_) => {
                drawing_infos.push(ItemDrawingInfo::Divider(DividerDrawingInfo {
                    vidx,
                    top: label.rect.top(),
                    bottom: label.rect.bottom(),
                }));
            }
            DisplayedItem::Marker(cursor) => {
                drawing_infos.push(ItemDrawingInfo::Marker(MarkerDrawingInfo {
                    vidx,
                    top: label.rect.top(),
                    bottom: label.rect.bottom(),
                    idx: cursor.idx,
                }));
            }
            DisplayedItem::TimeLine(_) => {
                drawing_infos.push(ItemDrawingInfo::TimeLine(TimeLineDrawingInfo {
                    vidx,
                    top: label.rect.top(),
                    bottom: label.rect.bottom(),
                }));
            }
            DisplayedItem::Stream(stream) => {
                drawing_infos.push(ItemDrawingInfo::Stream(StreamDrawingInfo {
                    transaction_stream_ref: stream.transaction_stream_ref.clone(),
                    vidx,
                    top: label.rect.top(),
                    bottom: label.rect.bottom(),
                }));
            }
            DisplayedItem::Group(_) => {
                drawing_infos.push(ItemDrawingInfo::Group(GroupDrawingInfo {
                    vidx,
                    top: label.rect.top(),
                    bottom: label.rect.bottom(),
                }));
            }
            &DisplayedItem::Placeholder(_) => {
                drawing_infos.push(ItemDrawingInfo::Placeholder(PlaceholderDrawingInfo {
                    vidx,
                    top: label.rect.top(),
                    bottom: label.rect.bottom(),
                }));
            }
            &DisplayedItem::Variable(_) => {
                panic!(
                    "draw_plain_item must not be called with a Variable - use draw_variable instead"
                )
            }
        }
        label.rect
    }

    fn item_is_focused(&self, vidx: VisibleItemIndex) -> bool {
        if let Some(waves) = &self.user.waves {
            waves.focused_item == Some(vidx)
        } else {
            false
        }
    }

    fn item_is_selected(&self, id: DisplayedItemRef) -> bool {
        if let Some(waves) = &self.user.waves {
            waves
                .items_tree
                .iter_visible_selected()
                .any(|node| node.item_ref == id)
        } else {
            false
        }
    }
}
