use crate::tile_kinds::annotation_list::AnnotationCommand;
use crate::{Message, annotation::Annotatable, item_list::ItemList, time::TimeFormatter};
use egui::{Align, Color32, Key, Layout, Ui};
use egui_remixicon::icons;
use tracing::warn;

pub(crate) const DEFAULT_GROUP_NAME: &str = "Ungrouped";
const TIME_FONT_SIZE: f32 = 11.;
const DEFAULT_SPACE: f32 = 4.;
const WIDTH_CONSTRAINT: f32 = 30.;

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct AnnotationGroup {
    pub name: String,
    pub annotations: Vec<egui::Id>,
}

impl AnnotationGroup {
    fn cycle_in_view(&self, ui: &Ui) -> Option<egui::Id> {
        if self.annotations.is_empty() {
            return None;
        }
        let key = ui.make_persistent_id(("annotation_cycle", &self.name, &self.annotations));
        let index =
            ui.data(|data| data.get_temp::<usize>(key).unwrap_or(0)) % self.annotations.len();
        ui.data_mut(|data| data.insert_temp(key, (index + 1) % self.annotations.len()));
        Some(self.annotations[index])
    }
}

impl ItemList {
    pub fn draw_annotation_list(
        &self,
        ui: &mut Ui,
        msgs: &mut Vec<Message>,
        time_formatter: &TimeFormatter,
        tile_id: crate::tiles::TileId,
        show_comments: bool,
    ) {
        ui.style_mut()
            .visuals
            .widgets
            .noninteractive
            .bg_stroke
            .width = 0.5;

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

            ui.data_mut(|d| d.insert_temp(input_id, buffer.clone()));

            // create group when user press enter
            let trimmed_buffer = buffer.trim();
            if text_edit_res.has_focus()
                && text_edit_res.ctx.input(|i| i.key_pressed(Key::Enter))
                && !trimmed_buffer.is_empty()
            {
                let flag = self
                    .annotation_groups
                    .iter()
                    .any(|group| group.name == trimmed_buffer);

                if !flag {
                    msgs.push(annotation_edit(
                        tile_id,
                        AnnotationCommand::CreateGroup(trimmed_buffer.to_string()),
                    ));
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
                && !trimmed_buffer.is_empty()
            {
                msgs.push(annotation_edit(
                    tile_id,
                    AnnotationCommand::CreateGroup(trimmed_buffer.to_string()),
                ));
                ui.data_mut(|d| d.insert_temp(input_id, String::new()));
            }

            // delete group when user press plus button
            if ui
                .button(icons::DELETE_BIN_LINE)
                .on_hover_text("Delete Group")
                .clicked()
                && !trimmed_buffer.is_empty()
            {
                msgs.push(annotation_edit(
                    tile_id,
                    AnnotationCommand::DeleteGroup(trimmed_buffer.to_string()),
                ));
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
                for group in self.annotation_groups.iter().rev() {
                    self.render_group_section(
                        ui,
                        group,
                        msgs,
                        time_formatter,
                        tile_id,
                        show_comments,
                    );
                }
            });
    }

    fn render_group_section(
        &self,
        ui: &mut Ui,
        group: &AnnotationGroup,
        msgs: &mut Vec<Message>,
        time_formatter: &TimeFormatter,
        tile_id: crate::tiles::TileId,
        show_comments: bool,
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
                        annotation_edit(
                            tile_id,
                            AnnotationCommand::DeleteGroup(group.name.clone()),
                        ),
                    )
                } else {
                    (
                        "Delete all annotations in this group",
                        annotation_edit(
                            tile_id,
                            AnnotationCommand::DeleteGroupAnnotations(group.name.clone()),
                        ),
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
                        msgs.push(annotation_edit(
                            tile_id,
                            AnnotationCommand::GroupVisibility {
                                group: group.name.clone(),
                                visible: !any_visible,
                            },
                        ));
                    }
                    // No need to allow user to cycle unless there are more than one annotations in group
                    if group.annotations.len() > 1
                        && ui
                            .button(icons::SKIP_FORWARD_LINE)
                            .on_hover_text("Cycle through group")
                            .clicked()
                        && let Some(id) = group.cycle_in_view(ui)
                    {
                        msgs.push(Message::GoToAnnotationPosition(id, tile_id));
                    }
                });
            })
            .body(|ui| {
                if group.annotations.is_empty() {
                    ui.weak("  No items");
                }

                for id in &group.annotations {
                    if let Some(annotation) = self.get_annotation_by_id(id) {
                        let comments_key =
                            ui.make_persistent_id(("annotation_comments", tile_id, id));
                        ui.horizontal(|ui| {
                            ui.add_space(6.0);

                            // Editable Name Logic
                            let editing_id =
                                ui.make_persistent_id(("editing_name", annotation.get_id()));
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
                                    msgs.push(annotation_edit(
                                        tile_id,
                                        AnnotationCommand::Rename {
                                            annotation: *id,
                                            name: buffer.trim().to_string(),
                                        },
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

                            let comments_open = ui
                                .data(|data| data.get_temp::<bool>(comments_key))
                                .unwrap_or(show_comments);
                            let show_comment_icon = if comments_open {
                                icons::ARROW_DOWN_S_LINE
                            } else {
                                icons::ARROW_RIGHT_S_LINE
                            };

                            if ui
                                .button(show_comment_icon)
                                .on_hover_text("Show comments")
                                .clicked()
                            {
                                ui.data_mut(|data| data.insert_temp(comments_key, !comments_open));
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
                                        msgs.push(annotation_edit(
                                            tile_id,
                                            AnnotationCommand::MoveToGroup {
                                                annotation: *id,
                                                group: Some(group.name.clone()),
                                            },
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
                                    msgs.push(annotation_edit(
                                        tile_id,
                                        AnnotationCommand::Remove(*id),
                                    ));
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
                                    msgs.push(annotation_edit(
                                        tile_id,
                                        AnnotationCommand::ToggleVisibility(*id),
                                    ));
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
                                        msgs.push(annotation_edit(
                                            tile_id,
                                            AnnotationCommand::ToggleCommentBox(*id),
                                        ));
                                    }
                                }
                                if ui
                                    .button(icons::SEARCH_LINE)
                                    .on_hover_text("Go to annotation")
                                    .clicked()
                                {
                                    msgs.push(Message::GoToAnnotationPosition(*id, tile_id));
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
                        if ui
                            .data(|data| data.get_temp::<bool>(comments_key))
                            .unwrap_or(show_comments)
                        {
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
                                            msgs.push(annotation_edit(
                                                tile_id,
                                                AnnotationCommand::RemoveComment {
                                                    annotation: annotation.get_id(),
                                                    comment: c.id,
                                                },
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

fn annotation_edit(tile_id: crate::tiles::TileId, command: AnnotationCommand) -> Message {
    Message::ToTile(
        tile_id,
        crate::tiles::kind::TileMessage::Waveform(
            crate::tile_kinds::waveform::WaveformMessage::Annotation(command),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_cycling_is_view_local_and_does_not_change_content() {
        let mut group = AnnotationGroup {
            name: "group".into(),
            annotations: vec![egui::Id::new("a"), egui::Id::new("b")],
        };
        let before = ron::to_string(&group).unwrap();
        let ctx = egui::Context::default();
        let cycle = |group: &AnnotationGroup, views: &[usize]| {
            let mut result = Vec::new();
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                for view in views {
                    let pane =
                        ui.new_child(egui::UiBuilder::new().id(egui::Id::new(("view", *view))));
                    result.push(group.cycle_in_view(&pane));
                }
            });
            output.textures_delta.clear();
            result
        };
        let [a, b] = [group.annotations[0], group.annotations[1]];
        assert_eq!(cycle(&group, &[0, 0, 1]), [Some(a), Some(b), Some(a)]);
        assert_eq!(cycle(&group, &[1, 0]), [Some(b), Some(a)]);
        assert_eq!(ron::to_string(&group).unwrap(), before);
        let inserted = egui::Id::new("new");
        group.annotations.insert(0, inserted);
        assert_eq!(cycle(&group, &[0, 1]), [Some(inserted), Some(inserted)]);
        group.annotations.clear();
        assert_eq!(cycle(&group, &[0]), [None]);
    }
}
