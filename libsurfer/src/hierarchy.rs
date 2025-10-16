//! Functions for drawing the left hand panel showing scopes and variables.
use crate::message::Message;
use crate::transaction_container::StreamScopeRef;
use crate::wave_container::{ScopeRef, ScopeRefExt};
use crate::wave_data::ScopeType;
use crate::SystemState;
use derive_more::{Display, FromStr};
use egui::{CentralPanel, Frame, Layout, Margin, ScrollArea, TextWrapMode, TopBottomPanel, Ui};
use emath::Align;
use enum_iterator::Sequence;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Display, FromStr, PartialEq, Eq, Serialize, Sequence)]
pub enum HierarchyStyle {
    Separate,
    Tree,
    Variables,
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
                ui.heading("Scopes");
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
                    ui.heading("Variables");
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
                    let wave_container = waves.inner.as_waves().unwrap();
                    let variables =
                        self.filtered_variables(&wave_container.variables_in_scope(scope), false);
                    // Parameters shown in variable list
                    if !self.show_parameters_in_scopes() {
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
                                    self.draw_variable_list(
                                        msgs,
                                        wave_container,
                                        ui,
                                        &parameters,
                                        None,
                                    );
                                });
                                self.draw_filtered_variable_list(
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
                        .text_style_height(&egui::TextStyle::Monospace)
                        .max(ui.text_style_height(&egui::TextStyle::Body));
                    ScrollArea::both()
                        .auto_shrink([false; 2])
                        .id_salt("variables")
                        .show_rows(ui, row_height, variables.len(), |ui, row_range| {
                            self.draw_filtered_variable_list(
                                msgs,
                                wave_container,
                                ui,
                                &variables,
                                Some(row_range),
                                false,
                            );
                        });
                }
                ScopeType::StreamScope(s) => {
                    ScrollArea::both()
                        .auto_shrink([false; 2])
                        .id_salt("variables")
                        .show(ui, |ui| {
                            self.draw_transaction_variable_list(msgs, waves, ui, s);
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
                        ui.heading("Hierarchy");
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
                        ui.heading("Variables");
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
            if waves.inner.is_waves() {
                let wave_container = waves.inner.as_waves().unwrap();
                let variables = self.filtered_variables(&wave_container.variables(false), true);
                let row_height = ui
                    .text_style_height(&egui::TextStyle::Monospace)
                    .max(ui.text_style_height(&egui::TextStyle::Body));
                ScrollArea::both()
                    .auto_shrink([false; 2])
                    .id_salt("variables")
                    .show_rows(ui, row_height, variables.len(), |ui, row_range| {
                        self.draw_filtered_variable_list(
                            msgs,
                            wave_container,
                            ui,
                            &variables,
                            Some(row_range),
                            true,
                        );
                    });
            } else {
                // No support for Streams yet
            };
        }
    }
}
