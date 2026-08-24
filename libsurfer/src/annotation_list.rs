use crate::{Message, annotation::Annotatable, time::TimeFormatter, wave_data::WaveData};
use egui::{Align, Color32, Key, Layout, Ui};
use egui_remixicon::icons;
use tracing::warn;

#[derive(Clone, Default)]
pub struct AnnotationList {}

pub(crate) const DEFAULT_GROUP_NAME: &str = "Ungrouped";
const TIME_FONT_SIZE: f32 = 11.;
const DEFAULT_SPACE: f32 = 4.;
const WIDTH_CONSTRAINT: f32 = 30.;

impl AnnotationList {}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct AnnotationGroup {
    pub name: String,
    pub cycle_counter: usize,
    pub annotations: Vec<egui::Id>,
}

impl AnnotationList {}

impl WaveData {
    pub fn draw_annotation_list(
        &self,
        ui: &mut Ui,
        msgs: &mut Vec<Message>,
        time_formatter: &TimeFormatter,
        annotation_groups: &mut [AnnotationGroup],
    ) {
        ui.style_mut()
            .visuals
            .widgets
            .noninteractive
            .bg_stroke
            .width = 0.5;

        ui.horizontal(|ui| {
            ui.allocate_space(egui::vec2(ui.available_width() - WIDTH_CONSTRAINT, 0.0));
            if ui.button(icons::CLOSE_LARGE_LINE).clicked() {
                msgs.push(Message::ToggleAnnotationlistVisibility());
            }
        });

        ui.vertical_centered(|ui| {
            ui.heading("Annotation List");
            if self.annotations.is_empty() {
                ui.label("Your annotations will be displayed here.");
            }
        });

        ui.add_space(DEFAULT_SPACE * 2.);
        ui.separator();

        // Create Group UI (Using egui Temp Memory)
        ui.horizontal(|ui| {
            ui.add_space(DEFAULT_SPACE * 2.);
            ui.label(egui::RichText::new("Manage Groups").small().strong());
        });
        ui.horizontal(|ui| {
            ui.add_space(DEFAULT_SPACE * 2.);
            let input_id = ui.make_persistent_id("group_input_buffer");
            let mut buffer = ui.data_mut(|d| d.get_temp::<String>(input_id).unwrap_or_default());

            let text_edit_res = ui.add(
                egui::TextEdit::singleline(&mut buffer)
                    .hint_text("Type group name...")
                    .desired_width(ui.available_width() - 160.0),
            );

            // Handle focusing of the text area when user clicks elsewhere, enables shortcuts.
            let focus_id = ui.make_persistent_id("group_input_focus_init");
            let has_focused = ui.data_mut(|d| d.get_temp::<bool>(focus_id).unwrap_or(false));

            if !has_focused {
                text_edit_res.request_focus();
                ui.data_mut(|d| d.insert_temp(focus_id, true));
            }

            ui.data_mut(|d| d.insert_temp(input_id, buffer.clone()));

            // create group when user press enter
            if text_edit_res.ctx.input(|i| i.key_pressed(Key::Enter)) && !buffer.is_empty() {
                let flag = annotation_groups.iter().any(|group| {
                    if group.name == buffer.trim() {
                        return true;
                    }
                    false
                });

                if !flag {
                    msgs.push(Message::CreateAnnotationGroup(buffer.trim().to_string()));
                    ui.data_mut(|d| d.insert_temp(input_id, String::new()));
                }
                // Keep focus here so users can type the next group immediately
                text_edit_res.request_focus();
            }
            // create group when user press plus button
            if ui
                .button(icons::ADD_LINE)
                .on_hover_text("Create Group")
                .clicked()
                && !buffer.is_empty()
            {
                msgs.push(Message::CreateAnnotationGroup(buffer.trim().to_string()));
                ui.data_mut(|d| d.insert_temp(input_id, String::new()));
            }

            // delete group when user press plus button
            if ui
                .button(icons::DELETE_BIN_LINE)
                .on_hover_text("Delete Group")
                .clicked()
                && !buffer.is_empty()
            {
                msgs.push(Message::DeleteAnnotationGroup(buffer.trim().to_string()));
                ui.data_mut(|d| d.insert_temp(input_id, String::new()));
            }
        });

        ui.add_space(DEFAULT_SPACE);
        ui.separator();

        // Scrollable List
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                // this is so ungrouped annotations are listed last
                for group in annotation_groups.iter_mut().rev() {
                    self.render_group_section(ui, group, msgs, time_formatter);
                }
            });
    }

    fn render_group_section(
        &self,
        ui: &mut Ui,
        group: &mut AnnotationGroup,
        msgs: &mut Vec<Message>,
        time_formatter: &TimeFormatter,
    ) {
        // Determine if the group is "mostly visible" or "mostly hidden" to pick the icon
        let any_visible = group.annotations.iter().any(|id| {
            if let Some(annotation) = self.get_annotation_by_id(id) {
                annotation.is_visible()
            } else {
                warn!("Got id to non existing annotatation");
                false
            }
        });
        let group_icon = if any_visible {
            icons::EYE_LINE
        } else {
            icons::EYE_OFF_LINE
        };

        // Create the header manually to inject the button
        let id = ui.make_persistent_id(&group.name);
        let state =
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true);

        state
            .show_header(ui, |ui| {
                ui.label(format!("{} ({})", group.name, group.annotations.len()));

                let (delete_tooltip, delete_message) = if group.annotations.is_empty() {
                    (
                        "Delete this group",
                        Message::DeleteAnnotationGroup(group.name.clone()),
                    )
                } else {
                    (
                        "Delete all annotations in this group",
                        Message::DeleteAllAnnotationInGroup(group.name.clone()),
                    )
                };
                // Push everything else to the right
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if group.name != DEFAULT_GROUP_NAME
                        && ui
                            .button(icons::DELETE_BIN_LINE)
                            .on_hover_text(delete_tooltip)
                            .clicked()
                    {
                        msgs.push(delete_message);
                    }
                    if ui
                        .button(group_icon)
                        .on_hover_text("Toggle visibility for all in this group")
                        .clicked()
                    {
                        msgs.push(Message::SetGroupVisibility(group.clone(), !any_visible));
                    }
                    // No need to allow user to cycle unless there are more than one annotations in group
                    if group.annotations.len() > 1
                        && ui
                            .button(icons::SKIP_FORWARD_LINE)
                            .on_hover_text("Cycle through group")
                            .clicked()
                    {
                        msgs.push(Message::GoToAnnotationPosition(
                            group.annotations[group.cycle_counter],
                            self.last_active_viewport_idx,
                        ));
                        group.cycle_counter += 1;

                        if group.cycle_counter >= group.annotations.len() {
                            group.cycle_counter = 0;
                        }
                    }
                });
            })
            .body(|ui| {
                if group.annotations.is_empty() {
                    ui.weak("  No items");
                }

                for id in &group.annotations {
                    if let Some(annotation) = self.get_annotation_by_id(id) {
                        ui.horizontal(|ui| {
                            ui.add_space(6.0);

                            // Editable Name Logic
                            let editing_id =
                                ui.make_persistent_id(("editing_name", annotation.get_name()));
                            let is_editing =
                                ui.data(|d| d.get_temp::<bool>(editing_id).unwrap_or(false));

                            let current_name = annotation.get_name();

                            if is_editing {
                                let mut buffer = ui.data_mut(|d| {
                                    d.get_temp::<String>(editing_id)
                                        .unwrap_or_else(|| current_name.clone())
                                });

                                let res = ui.add(
                                    egui::TextEdit::singleline(&mut buffer).desired_width(120.0),
                                );

                                if res.has_focus() {
                                    ui.data_mut(|d| d.insert_temp(editing_id, buffer.clone()));
                                }

                                // Save on Enter or if focus is lost
                                if res.lost_focus()
                                    || (res.has_focus() && ui.input(|i| i.key_pressed(Key::Enter)))
                                {
                                    msgs.push(Message::UpdateAnnotationName(
                                        *id,
                                        buffer.trim().to_string(),
                                    ));
                                    ui.data_mut(|d| d.insert_temp(editing_id, false));
                                }

                                // Request focus once when we start editing
                                if ui.data(|d| {
                                    d.get_temp::<bool>(
                                        ui.make_persistent_id(("focus_req", &current_name)),
                                    )
                                    .unwrap_or(true)
                                }) {
                                    res.request_focus();
                                    ui.data_mut(|d| {
                                        d.insert_temp(
                                            ui.make_persistent_id(("focus_req", current_name)),
                                            false,
                                        )
                                    });
                                }
                            } else {
                                // Display the name as a clickable label
                                let response = ui.add(
                                    egui::Label::new(egui::RichText::new(&current_name).strong())
                                        .sense(egui::Sense::click()),
                                );
                                if response.clicked() {
                                    ui.data_mut(|d| d.insert_temp(editing_id, true));
                                    ui.data_mut(|d| {
                                        d.insert_temp(
                                            ui.make_persistent_id(("focus_req", current_name)),
                                            true,
                                        )
                                    });
                                }
                                response.on_hover_text("Click to rename");
                            }

                            let show_comment_icon = if annotation.show_comments() {
                                icons::ARROW_DOWN_S_LINE
                            } else {
                                icons::ARROW_RIGHT_S_LINE
                            };

                            if ui
                                .button(show_comment_icon)
                                .on_hover_text("Show comments")
                                .clicked()
                            {
                                msgs.push(Message::ToggleAnnotationListShowComments(*id));
                            }

                            //This is only here because selectable_value needs a string, we dont want it to match any group we have.
                            let placeholder = "ungrouped".to_string();

                            ui.menu_button(icons::FOLDER_TRANSFER_LINE, |ui| {
                                for group in self.annotation_groups.iter().rev() {
                                    if ui
                                        .selectable_value(
                                            &mut Some(placeholder.clone()),
                                            Some(group.name.clone()),
                                            group.name.clone(),
                                        )
                                        .clicked()
                                    {
                                        msgs.push(Message::UpdateAnnotationGroup(
                                            *id,
                                            Some(group.name.clone()),
                                        ));
                                        ui.close();
                                    }
                                }
                            })
                            .response
                            .on_hover_text("Change Group");

                            // Buttons on the right
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui
                                    .button(icons::DELETE_BIN_LINE)
                                    .on_hover_text("Delete annotation")
                                    .clicked()
                                {
                                    msgs.push(Message::RemoveAnnotation(*id));
                                }

                                let vis_icon = if annotation.is_visible() {
                                    icons::EYE_LINE
                                } else {
                                    icons::EYE_OFF_LINE
                                };
                                if ui
                                    .button(vis_icon)
                                    .on_hover_text("Toggle visibility")
                                    .clicked()
                                {
                                    msgs.push(Message::ToggleAnnotationVisiblility(*id));
                                }

                                let comment = annotation.get_comment_box();

                                if annotation.is_visible() {
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
                                        msgs.push(Message::ToggleCommentVisibility(*id));
                                    }
                                }
                                if ui
                                    .button(icons::SEARCH_LINE)
                                    .on_hover_text("Go to annotation")
                                    .clicked()
                                {
                                    msgs.push(Message::GoToAnnotationPosition(
                                        *id,
                                        self.last_active_viewport_idx,
                                    ));
                                }
                            });
                        });

                        ui.horizontal(|ui| {
                            ui.add_space(16.0);
                            ui.label(
                                egui::RichText::new(annotation.get_time_info(time_formatter))
                                    .size(TIME_FONT_SIZE)
                                    .color(Color32::LIGHT_GRAY),
                            )
                        });

                        // Show comments for this annotation
                        if annotation.show_comments() {
                            let messages = annotation.get_messages();
                            for c in messages {
                                let mut line_left = ui.cursor().left_top();
                                line_left.x += 16.;
                                ui.painter().add(egui::Shape::line_segment(
                                    [line_left, ui.cursor().right_top()],
                                    egui::Stroke::new(0.5, egui::Color32::WHITE),
                                ));
                                ui.horizontal(|ui| {
                                    ui.add_space(18.0); // Indent comments
                                    ui.vertical(|ui| {
                                        ui.add_space(DEFAULT_SPACE * 0.5);
                                        ui.set_max_width(ui.available_width() - WIDTH_CONSTRAINT);
                                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);

                                        ui.add(egui::Label::new(c.text.as_str()).wrap());
                                    });

                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        let response = ui.add_sized(
                                            egui::Vec2::new(10.0, 10.0),
                                            egui::Button::new(icons::DELETE_BIN_LINE),
                                        );

                                        if response.on_hover_text("Delete message").clicked() {
                                            msgs.push(Message::RemoveCommentMessage(
                                                annotation.get_id(),
                                                c.id,
                                            ));
                                        }
                                    });
                                });
                            }
                        }
                    }
                }
            });
        ui.separator();
        ui.add_space(DEFAULT_SPACE);
    }

    pub fn remove_annotation_from_group(&mut self, id_to_remove: egui::Id) -> Option<egui::Id> {
        for group in &mut self.annotation_groups {
            if let Some(idx) = group.annotations.iter().position(|&id| id == id_to_remove) {
                group.cycle_counter = 0;
                return Some(group.annotations.remove(idx));
            }
        }

        None
    }

    pub fn remove_all_annotations_from_group(&mut self, name: &str) {
        for group in &mut self.annotation_groups {
            if group.name == name {
                self.annotations
                    .retain(|annotation| !group.annotations.contains(&annotation.get_id()));
                group.cycle_counter = 0;
                group.annotations = Vec::new();
            }
        }
    }

    pub fn delete_group(&mut self, group_name: &str) {
        if let Some(idx) = self
            .annotation_groups
            .iter()
            .position(|group| group.name == group_name)
        {
            self.annotation_groups.remove(idx);
        }
    }

    pub fn add_annotation_to_group(&mut self, group_name: &str, id_to_add: egui::Id) {
        if let Some(idx) = self
            .annotation_groups
            .iter()
            .position(|group| group.name == group_name)
        {
            self.annotation_groups[idx].annotations.push(id_to_add);
        }
    }

    #[must_use]
    pub fn annotation_is_in_group(&self, annotation_id: egui::Id) -> bool {
        for group in &self.annotation_groups {
            if group.annotations.contains(&annotation_id) {
                return true;
            }
        }

        false
    }

    #[must_use]
    pub fn get_group_from_annotation(&self, annotation_id: egui::Id) -> Option<&AnnotationGroup> {
        self.annotation_groups
            .iter()
            .find(|&group| group.annotations.contains(&annotation_id))
            .map(|g| g as _)
    }

    pub fn get_group_from_name(&mut self, group_name: &str) -> Option<&mut AnnotationGroup> {
        self.annotation_groups
            .iter_mut()
            .find(|group| group.name == group_name)
            .map(|g| g as _)
    }
}
