//! Functions for drawing the left hand panel showing scopes and variables.
use crate::SystemState;
use crate::data_container::{DataContainer, VariableType as VarType};
use crate::displayed_item_tree::VisibleItemIndex;
use crate::message::Message;
use crate::tooltips::{scope_tooltip_text, variable_tooltip_text};
use crate::transaction_container::StreamScopeRef;
use crate::transactions::{draw_transaction_root, draw_transaction_variable_list};
use crate::variable_direction::get_direction_string;
use crate::view::draw_true_name;
use crate::wave_container::{ScopeRef, ScopeRefExt, VariableRef, WaveContainer};
use crate::wave_data::{ScopeType, WaveData};
use derive_more::{Display, FromStr};
use ecolor::Color32;
use egui::text::LayoutJob;
use egui::{CentralPanel, Frame, Layout, ScrollArea, TextStyle, TopBottomPanel, Ui};
use emath::Align;
use enum_iterator::Sequence;
use epaint::{
    Margin,
    text::{TextFormat, TextWrapMode},
};
use eyre::Context;
use itertools::Itertools;
use num::BigUint;
use serde::{Deserialize, Serialize};
use std::ops::Range;
use tracing::warn;
#[derive(Clone, Copy, Debug, Deserialize, Display, FromStr, PartialEq, Eq, Serialize, Sequence)]
pub enum HierarchyStyle {
    Separate,
    Tree,
    Variables,
}

#[derive(Clone, Copy, Debug, Deserialize, Display, FromStr, PartialEq, Eq, Serialize, Sequence)]
pub enum ParameterDisplayLocation {
    Variables,
    Scopes,
    Tooltips,
    None,
}

impl SystemState {
    /// Scopes and variables in two separate lists
    pub fn separate(&mut self, ui: &mut Ui, msgs: &mut Vec<Message>) {
        ui.visuals_mut().override_text_color =
            Some(self.user.config.theme.primary_ui_color.foreground);

        let total_space = ui.available_height();
        TopBottomPanel::top("scopes")
            .resizable(true)
            .default_height(total_space / 2.0)
            .max_height(total_space - 64.0)
            .frame(Frame::new().inner_margin(Margin::same(5)))
            .show_inside(ui, |ui| {
                ui.heading("Scopes")
                    .context_menu(|ui| self.hierarchy_menu(msgs, ui));
                ui.add_space(3.0);

                ScrollArea::both()
                    .id_salt("scopes")
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);
                        if let Some(waves) = &self.user.waves {
                            self.draw_all_scopes(msgs, waves, false, ui);
                        }
                    });
            });
        CentralPanel::default()
            .frame(Frame::new().inner_margin(Margin::same(5)))
            .show_inside(ui, |ui| {
                ui.with_layout(Layout::left_to_right(Align::TOP), |ui| {
                    ui.heading("Variables")
                        .context_menu(|ui| self.hierarchy_menu(msgs, ui));
                    ui.add_space(3.0);
                    self.draw_variable_filter_edit(ui, msgs, false);
                });
                ui.add_space(3.0);

                self.draw_variables(msgs, ui);
            });
    }

    fn draw_variables(&mut self, msgs: &mut Vec<Message>, ui: &mut Ui) {
        if let Some(waves) = &self.user.waves {
            let empty_scope = if waves.inner.is_waves() {
                ScopeType::WaveScope(ScopeRef::empty())
            } else {
                ScopeType::StreamScope(StreamScopeRef::Empty(String::default()))
            };
            let active_scope = waves.active_scope.as_ref().unwrap_or(&empty_scope);
            match active_scope {
                ScopeType::WaveScope(scope) => {
                    let Some(wave_container) = waves.inner.as_waves() else {
                        return;
                    };
                    let variables =
                        self.filtered_variables(&wave_container.variables_in_scope(scope), false);
                    // Parameters shown in variable list
                    if self.parameter_display_location() == ParameterDisplayLocation::Variables {
                        let parameters = wave_container.parameters_in_scope(scope);
                        if !parameters.is_empty() {
                            ScrollArea::both()
                            .auto_shrink([false; 2])
                            .id_salt("variables")
                            .show(ui, |ui| {
                                egui::collapsing_header::CollapsingState::load_with_default_open(
                                    ui.ctx(),
                                    egui::Id::new(&parameters),
                                    self.expand_parameter_section,
                                )
                                .show_header(ui, |ui| {
                                    ui.with_layout(
                                        Layout::top_down(Align::LEFT).with_cross_justify(true),
                                        |ui| {
                                            ui.label("Parameters");
                                        },
                                    );
                                })
                                .body(|ui| {
                                    self.filter_and_draw_variable_list(
                                        msgs,
                                        wave_container,
                                        ui,
                                        &parameters,
                                        None,
                                    );
                                });
                                self.draw_variable_list(
                                    msgs,
                                    wave_container,
                                    ui,
                                    &variables,
                                    None,
                                    false,
                                );
                            });
                            return; // Early exit
                        }
                    }
                    // Parameters not shown here or no parameters: use fast approach only drawing visible rows
                    let row_height = ui
                        .text_style_height(&TextStyle::Monospace)
                        .max(ui.text_style_height(&TextStyle::Body));
                    ScrollArea::both()
                        .auto_shrink([false; 2])
                        .id_salt("variables")
                        .show_rows(ui, row_height, variables.len(), |ui, row_range| {
                            self.draw_variable_list(
                                msgs,
                                wave_container,
                                ui,
                                &variables,
                                Some(&row_range),
                                false,
                            );
                        });
                }
                ScopeType::StreamScope(s) => {
                    ScrollArea::both()
                        .auto_shrink([false; 2])
                        .id_salt("variables")
                        .show(ui, |ui| {
                            draw_transaction_variable_list(msgs, waves, ui, s);
                        });
                }
            }
        }
    }

    /// Scopes and variables in a joint tree.
    pub fn tree(&mut self, ui: &mut Ui, msgs: &mut Vec<Message>) {
        ui.visuals_mut().override_text_color =
            Some(self.user.config.theme.primary_ui_color.foreground);

        ui.with_layout(
            Layout::top_down(Align::LEFT).with_cross_justify(true),
            |ui| {
                Frame::new().inner_margin(Margin::same(5)).show(ui, |ui| {
                    ui.with_layout(Layout::left_to_right(Align::TOP), |ui| {
                        ui.heading("Hierarchy")
                            .context_menu(|ui| self.hierarchy_menu(msgs, ui));
                        ui.add_space(3.0);
                        self.draw_variable_filter_edit(ui, msgs, false);
                    });
                    ui.add_space(3.0);

                    ScrollArea::both().id_salt("hierarchy").show(ui, |ui| {
                        ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);
                        if let Some(waves) = &self.user.waves {
                            self.draw_all_scopes(msgs, waves, true, ui);
                        }
                    });
                });
            },
        );
    }

    /// List with all variables.
    pub fn variable_list(&mut self, ui: &mut Ui, msgs: &mut Vec<Message>) {
        ui.visuals_mut().override_text_color =
            Some(self.user.config.theme.primary_ui_color.foreground);

        ui.with_layout(
            Layout::top_down(Align::LEFT).with_cross_justify(true),
            |ui| {
                Frame::new().inner_margin(Margin::same(5)).show(ui, |ui| {
                    ui.with_layout(Layout::left_to_right(Align::TOP), |ui| {
                        ui.heading("Variables")
                            .context_menu(|ui| self.hierarchy_menu(msgs, ui));
                        ui.add_space(3.0);
                        self.draw_variable_filter_edit(ui, msgs, true);
                    });
                    ui.add_space(3.0);

                    ScrollArea::both().id_salt("variables").show(ui, |ui| {
                        ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);
                        self.draw_all_variables(msgs, ui);
                    });
                });
            },
        );
    }

    fn draw_all_variables(&mut self, msgs: &mut Vec<Message>, ui: &mut Ui) {
        if let Some(waves) = &self.user.waves {
            match &waves.inner {
                DataContainer::Waves(wave_container) => {
                    let variables = self.filtered_variables(&wave_container.variables(false), true);
                    let row_height = ui
                        .text_style_height(&TextStyle::Monospace)
                        .max(ui.text_style_height(&TextStyle::Body));
                    ScrollArea::both()
                        .auto_shrink([false; 2])
                        .id_salt("variables")
                        .show_rows(ui, row_height, variables.len(), |ui, row_range| {
                            self.draw_variable_list(
                                msgs,
                                wave_container,
                                ui,
                                &variables,
                                Some(&row_range),
                                true,
                            );
                        });
                }
                DataContainer::Transactions(_) => {
                    // No support for Streams yet
                    ui.with_layout(
                        Layout::top_down(Align::LEFT).with_cross_justify(true),
                        |ui| {
                            ui.label("Streams are not yet supported.");
                            ui.label("Select another view.");
                        },
                    );
                }
                DataContainer::Empty => {}
            }
        }
    }

    fn draw_all_scopes(
        &self,
        msgs: &mut Vec<Message>,
        wave: &WaveData,
        draw_variables: bool,
        ui: &mut Ui,
    ) {
        for scope in wave.inner.root_scopes() {
            match scope {
                ScopeType::WaveScope(scope) => {
                    self.draw_selectable_child_or_orphan_scope(
                        msgs,
                        wave,
                        &scope,
                        draw_variables,
                        ui,
                    );
                }
                ScopeType::StreamScope(_) => {
                    draw_transaction_root(msgs, wave, ui);
                }
            }
        }
        if draw_variables && let Some(wave_container) = wave.inner.as_waves() {
            let scope = ScopeRef::empty();
            let variables = wave_container.variables_in_scope(&scope);
            self.filter_and_draw_variable_list(msgs, wave_container, ui, &variables, None);
        }
    }

    fn add_scope_selectable_label(
        &self,
        msgs: &mut Vec<Message>,
        wave: &WaveData,
        scope: &ScopeRef,
        ui: &mut Ui,
        scroll_to_label: bool,
    ) {
        let name = scope.name();
        let mut response = ui.add(egui::Button::selectable(
            wave.active_scope == Some(ScopeType::WaveScope(scope.clone())),
            name,
        ));
        let _ = response.interact(egui::Sense::click_and_drag());
        response.drag_started().then(|| {
            msgs.push(Message::VariableDragStarted(VisibleItemIndex(
                wave.display_item_ref_counter,
            )));
        });

        if scroll_to_label {
            response.scroll_to_me(Some(Align::Center));
        }

        response.drag_stopped().then(|| {
            if ui.input(|i| i.pointer.hover_pos().unwrap_or_default().x)
                > self.user.sidepanel_width.unwrap_or_default()
            {
                let scope_t = ScopeType::WaveScope(scope.clone());
                let variables = wave
                    .inner
                    .variables_in_scope(&scope_t)
                    .iter()
                    .filter_map(|var| match var {
                        VarType::Variable(var) => Some(var.clone()),
                        VarType::Generator(_) => None,
                    })
                    .collect_vec();

                msgs.push(Message::AddDraggedVariables(
                    self.filtered_variables(variables.as_slice(), false),
                ));
            }
        });
        if self.show_scope_tooltip() {
            response = response.on_hover_ui(|ui| {
                ui.set_max_width(ui.spacing().tooltip_width);
                ui.add(egui::Label::new(scope_tooltip_text(
                    wave,
                    scope,
                    self.parameter_display_location() == ParameterDisplayLocation::Tooltips,
                )));
            });
        }
        response.context_menu(|ui| {
            if ui.button("Add scope").clicked() {
                msgs.push(Message::AddScope(scope.clone(), false));
            }
            if ui.button("Add scope recursively").clicked() {
                msgs.push(Message::AddScope(scope.clone(), true));
            }
            if ui.button("Add scope as group").clicked() {
                msgs.push(Message::AddScopeAsGroup(scope.clone(), false));
            }
            if ui.button("Add scope as group recursively").clicked() {
                msgs.push(Message::AddScopeAsGroup(scope.clone(), true));
            }
        });
        response
            .clicked()
            .then(|| msgs.push(Message::SetActiveScope(ScopeType::WaveScope(scope.clone()))));
    }

    fn draw_selectable_child_or_orphan_scope(
        &self,
        msgs: &mut Vec<Message>,
        wave: &WaveData,
        scope: &ScopeRef,
        draw_variables: bool,
        ui: &mut Ui,
    ) {
        // Extract wave container once to avoid repeated as_waves().unwrap() calls
        let Some(wave_container) = wave.inner.as_waves() else {
            return;
        };

        let Some(child_scopes) = wave_container
            .child_scopes(scope)
            .context("Failed to get child scopes")
            .map_err(|e| warn!("{e:#?}"))
            .ok()
        else {
            return;
        };

        let no_variables_in_scope = wave_container.no_variables_in_scope(scope);
        if child_scopes.is_empty() && no_variables_in_scope && !self.show_empty_scopes() {
            return;
        }

        if child_scopes.is_empty() && (!draw_variables || no_variables_in_scope) {
            // Indent our label by both icon width and icon spacing to
            // match the other headers that actually have an icon.
            ui.horizontal(|ui| {
                ui.add_space(ui.spacing().icon_width + ui.spacing().icon_spacing);
                self.add_scope_selectable_label(msgs, wave, scope, ui, false);
            });
        } else {
            let should_open_header = self.should_open_header(scope);
            let mut collapsing_header =
                egui::collapsing_header::CollapsingState::load_with_default_open(
                    ui.ctx(),
                    egui::Id::new(scope),
                    false,
                );
            if should_open_header {
                collapsing_header.set_open(true);
            }
            collapsing_header
                .show_header(ui, |ui| {
                    ui.with_layout(
                        Layout::top_down(Align::LEFT).with_cross_justify(true),
                        |ui| {
                            self.add_scope_selectable_label(
                                msgs,
                                wave,
                                scope,
                                ui,
                                should_open_header,
                            );
                        },
                    );
                })
                .body(|ui| {
                    if draw_variables
                        || self.parameter_display_location() == ParameterDisplayLocation::Scopes
                    {
                        let parameters = wave_container.parameters_in_scope(scope);
                        if !parameters.is_empty() {
                            egui::collapsing_header::CollapsingState::load_with_default_open(
                                ui.ctx(),
                                egui::Id::new(scope).with("__surfer_parameters"),
                                false,
                            )
                            .show_header(ui, |ui| {
                                ui.with_layout(
                                    Layout::top_down(Align::LEFT).with_cross_justify(true),
                                    |ui| {
                                        ui.add(egui::Button::selectable(false, "Parameters"));
                                    },
                                );
                            })
                            .body(|ui| {
                                self.filter_and_draw_variable_list(
                                    msgs,
                                    wave_container,
                                    ui,
                                    &parameters,
                                    None,
                                );
                            });
                        }
                    }
                    self.draw_root_scope_view(msgs, wave, scope, draw_variables, ui);
                    if draw_variables {
                        let variables = wave_container.variables_in_scope(scope);
                        self.filter_and_draw_variable_list(
                            msgs,
                            wave_container,
                            ui,
                            &variables,
                            None,
                        );
                    }
                });
        }
    }

    fn draw_root_scope_view(
        &self,
        msgs: &mut Vec<Message>,
        wave: &WaveData,
        root_scope: &ScopeRef,
        draw_variables: bool,
        ui: &mut Ui,
    ) {
        // Extract wave container once to avoid unwrap
        let Some(wave_container) = wave.inner.as_waves() else {
            return;
        };

        let Some(child_scopes) = wave_container
            .child_scopes(root_scope)
            .context("Failed to get child scopes")
            .map_err(|e| warn!("{e:#?}"))
            .ok()
        else {
            return;
        };

        let child_scopes_sorted = child_scopes
            .iter()
            .sorted_by(|a, b| numeric_sort::cmp(&a.name(), &b.name()))
            .collect_vec();

        for child_scope in child_scopes_sorted {
            self.draw_selectable_child_or_orphan_scope(msgs, wave, child_scope, draw_variables, ui);
        }
    }

    fn filter_and_draw_variable_list(
        &self,
        msgs: &mut Vec<Message>,
        wave_container: &WaveContainer,
        ui: &mut Ui,
        variables: &[VariableRef],
        row_range: Option<&Range<usize>>,
    ) {
        let filtered_variables = self.filtered_variables(variables, false);
        self.draw_variable_list(
            msgs,
            wave_container,
            ui,
            &filtered_variables,
            row_range,
            false,
        );
    }

    fn draw_variable_list(
        &self,
        msgs: &mut Vec<Message>,
        wave_container: &WaveContainer,
        ui: &mut Ui,
        variables: &[VariableRef],
        row_range: Option<&Range<usize>>,
        display_full_path: bool,
    ) {
        // Get iterator with more info about each variable
        let variable_infos = variables
            .iter()
            .map(|var| {
                let meta = wave_container.variable_meta(var).ok();
                let name_info = self.get_variable_name_info(wave_container, var);
                (var, meta, name_info)
            })
            .sorted_by_key(|(_, _, name_info)| {
                -name_info
                    .as_ref()
                    .and_then(|info| info.priority)
                    .unwrap_or_default()
            })
            .skip(row_range.as_ref().map_or(0, |r| r.start))
            .take(
                row_range
                    .as_ref()
                    .map_or(variables.len(), |r| r.end - r.start),
            );

        // Precompute common font metrics once per frame to avoid expensive per-row work.
        // NOTE: Safe unwrap, we know that egui has its own built-in font.
        // Use precomputed font and char width where available to reduce work.
        let monospace_font = ui
            .style()
            .text_styles
            .get(&TextStyle::Monospace)
            .cloned()
            .unwrap();
        let body_font = ui
            .style()
            .text_styles
            .get(&TextStyle::Body)
            .cloned()
            .unwrap();
        let char_width_mono = ui.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(" ".to_string(), monospace_font.clone(), Color32::BLACK)
                .size()
                .x
        });
        // The button padding is added by egui on selectable labels
        let available_space = ui.available_width() - ui.spacing().button_padding.x * 2.;

        // Draw variables
        for (variable, meta, name_info) in variable_infos {
            // Get index string
            let index = meta
                .as_ref()
                .and_then(|meta| meta.index)
                .map(|index| {
                    if self.show_variable_indices() {
                        format!(" {index}")
                    } else {
                        String::new()
                    }
                })
                .unwrap_or_default();

            // Get direction icon
            let direction = self
                .show_variable_direction()
                .then(|| get_direction_string(meta.as_ref(), name_info.as_ref()))
                .flatten()
                .unwrap_or_default();
            // Get value in case of parameter
            let value = if meta.as_ref().is_some_and(|meta| meta.is_parameter()) {
                let res = wave_container.query_variable(variable, &BigUint::ZERO).ok();
                res.and_then(|o| o.and_then(|q| q.current.map(|v| format!(": {}", v.1))))
                    .unwrap_or_else(|| ": Undefined".to_string())
            } else {
                String::new()
            };

            ui.with_layout(
                Layout::top_down(Align::LEFT).with_cross_justify(true),
                |ui| {
                    let mut label = LayoutJob::default();

                    if let Some(name) = name_info.and_then(|info| info.true_name) {
                        let direction_size = direction.chars().count();
                        let index_size = index.chars().count();
                        let value_size = value.chars().count();
                        let used_space =
                            (direction_size + index_size + value_size) as f32 * char_width_mono;
                        let space_for_name = available_space - used_space;

                        let text_format = TextFormat {
                            font_id: monospace_font.clone(),
                            color: self.user.config.theme.foreground,
                            ..Default::default()
                        };

                        label.append(&direction, 0.0, text_format.clone());

                        draw_true_name(
                            &name,
                            &mut label,
                            monospace_font.clone(),
                            self.user.config.theme.foreground,
                            char_width_mono,
                            space_for_name,
                        );

                        label.append(&index, 0.0, text_format.clone());
                        label.append(&value, 0.0, text_format.clone());
                    } else {
                        let text_format = TextFormat {
                            font_id: body_font.clone(),
                            color: self.user.config.theme.foreground,
                            ..Default::default()
                        };
                        let name = if display_full_path {
                            variable.full_path().join(".")
                        } else {
                            variable.name.clone()
                        };
                        label.append(&direction, 0.0, text_format.clone());
                        label.append(&name, 0.0, text_format.clone());
                        label.append(&index, 0.0, text_format.clone());
                        label.append(&value, 0.0, text_format.clone());
                    }

                    let mut response = ui.add(egui::Button::selectable(false, label));

                    let _ = response.interact(egui::Sense::click_and_drag());

                    if self.show_tooltip() {
                        // Reuse the already-obtained `meta` and pass a clone of the variable
                        // reference into the closure so we don't call `variable_meta` again.
                        let tooltip_meta = meta.clone();
                        let tooltip_var = variable.clone();
                        response = response.on_hover_ui(move |ui| {
                            ui.set_max_width(ui.spacing().tooltip_width);
                            ui.add(egui::Label::new(variable_tooltip_text(
                                tooltip_meta.as_ref(),
                                &tooltip_var,
                            )));
                        });
                    }
                    response.drag_started().then(|| {
                        msgs.push(Message::VariableDragStarted(VisibleItemIndex(
                            self.user.waves.as_ref().unwrap().display_item_ref_counter,
                        )));
                    });
                    response.drag_stopped().then(|| {
                        if ui.input(|i| i.pointer.hover_pos().unwrap_or_default().x)
                            > self.user.sidepanel_width.unwrap_or_default()
                        {
                            msgs.push(Message::AddDraggedVariables(vec![variable.clone()]));
                        }
                    });
                    response
                        .clicked()
                        .then(|| msgs.push(Message::AddVariables(vec![variable.clone()])));
                },
            );
        }
    }

    fn should_open_header(&self, scope: &ScopeRef) -> bool {
        let mut scope_ref_cell = self.scope_ref_to_expand.borrow_mut();
        if let Some(state) = scope_ref_cell.as_mut()
            && state.strs.starts_with(&scope.strs)
        {
            if (state.strs.len() - 1) == scope.strs.len() {
                // need to compare vs. parent of signal
                *scope_ref_cell = None;
            }
            return true;
        }
        false
    }
}
