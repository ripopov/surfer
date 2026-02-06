use crate::message::Message;
use crate::table::{
    SignalAnalysisConfig, SignalAnalysisSamplingConfig, SignalAnalysisSamplingMode,
    SignalAnalysisSignal,
};
use crate::wave_container::{VariableRef, VariableRefExt};
use ecolor::Color32;
use egui::{ComboBox, Key, Layout, RichText, ScrollArea};
use emath::Align;

#[derive(Debug, Default, Copy, Clone)]
pub struct ReloadWaveformDialog {
    /// `true` to persist the setting returned by the dialog.
    do_not_show_again: bool,
}

#[derive(Debug, Default, Copy, Clone)]
pub struct OpenSiblingStateFileDialog {
    do_not_show_again: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalAnalysisWizardSamplingOption {
    pub variable: VariableRef,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalAnalysisWizardSignal {
    pub variable: VariableRef,
    pub display_name: String,
    pub include: bool,
    pub translator: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalAnalysisWizardDialog {
    pub sampling_options: Vec<SignalAnalysisWizardSamplingOption>,
    pub sampling_signal: VariableRef,
    pub signals: Vec<SignalAnalysisWizardSignal>,
    pub translators: Vec<String>,
    pub marker_count: usize,
}

impl SignalAnalysisWizardDialog {
    #[must_use]
    pub fn has_selected_signals(&self) -> bool {
        self.signals.iter().any(|signal| signal.include)
    }

    #[must_use]
    pub fn marker_info_text(&self) -> String {
        if self.marker_count == 0 {
            "No markers - only global statistics".to_string()
        } else {
            let marker_word = if self.marker_count == 1 {
                "marker"
            } else {
                "markers"
            };
            format!(
                "{} {} will define {} intervals",
                self.marker_count,
                marker_word,
                self.marker_count + 1
            )
        }
    }

    #[must_use]
    pub fn to_config(&self) -> Option<SignalAnalysisConfig> {
        let signals = self
            .signals
            .iter()
            .filter(|signal| signal.include)
            .map(|signal| SignalAnalysisSignal {
                variable: signal.variable.clone(),
                field: vec![],
                translator: signal.translator.clone(),
            })
            .collect::<Vec<_>>();

        if signals.is_empty() {
            return None;
        }

        Some(SignalAnalysisConfig {
            sampling: SignalAnalysisSamplingConfig {
                signal: self.sampling_signal.clone(),
            },
            signals,
            run_revision: 0,
        })
    }
}

#[must_use]
pub(crate) fn draw_signal_analysis_wizard_dialog(
    ctx: &egui::Context,
    dialog: &mut SignalAnalysisWizardDialog,
    resolved_mode: Option<SignalAnalysisSamplingMode>,
    msgs: &mut Vec<Message>,
) -> bool {
    let mut is_open = true;
    let mut run_requested = false;
    let mut cancel_requested = false;
    let run_enabled = dialog.has_selected_signals();

    let selected_sampling_label = dialog
        .sampling_options
        .iter()
        .find(|option| option.variable == dialog.sampling_signal)
        .map_or_else(
            || dialog.sampling_signal.full_path_string(),
            |option| option.display_name.clone(),
        );
    let resolved_mode_text = match resolved_mode {
        Some(SignalAnalysisSamplingMode::Event) => "Event",
        Some(SignalAnalysisSamplingMode::PosEdge) => "Pos. Edge",
        Some(SignalAnalysisSamplingMode::AnyChange) => "Any Change",
        None => "Unknown",
    };

    egui::Window::new("Signal Analyzer Configuration")
        .open(&mut is_open)
        .collapsible(false)
        .resizable(true)
        .default_width(720.0)
        .show(ctx, |ui| {
            ui.label(RichText::new("Sampling Signal").strong());
            ComboBox::from_id_salt("signal_analysis_sampling_signal")
                .width(ui.available_width())
                .selected_text(selected_sampling_label)
                .show_ui(ui, |ui| {
                    for option in &dialog.sampling_options {
                        ui.selectable_value(
                            &mut dialog.sampling_signal,
                            option.variable.clone(),
                            option.display_name.clone(),
                        );
                    }
                });

            ui.add_space(8.0);
            ui.label(format!("Resolved Mode: {resolved_mode_text}"));
            ui.add_space(8.0);

            ui.label(
                RichText::new(format!("Signals to analyze ({})", dialog.signals.len())).strong(),
            );
            ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                for (idx, signal) in dialog.signals.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut signal.include, "");
                        ui.label(&signal.display_name);
                        ui.add_space(8.0);
                        ComboBox::from_id_salt(("signal_analysis_translator", idx))
                            .width(180.0)
                            .selected_text(&signal.translator)
                            .show_ui(ui, |ui| {
                                for translator in &dialog.translators {
                                    ui.selectable_value(
                                        &mut signal.translator,
                                        translator.clone(),
                                        translator,
                                    );
                                }
                            });
                    });
                }
            });

            ui.add_space(8.0);
            ui.label(dialog.marker_info_text());
            ui.add_space(12.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let run_button = ui.add_enabled(run_enabled, egui::Button::new("Run"));
                if run_button.clicked() {
                    run_requested = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel_requested = true;
                }
            });
        });

    if !is_open || ctx.input(|input| input.key_pressed(Key::Escape)) {
        cancel_requested = true;
    }
    if run_enabled && ctx.input(|input| input.key_pressed(Key::Enter)) {
        run_requested = true;
    }

    if run_requested && let Some(config) = dialog.to_config() {
        msgs.push(Message::RunSignalAnalysis { config });
        return true;
    }

    cancel_requested
}

/// Draw a dialog that asks the user if it wants to load a state file situated in the same directory as the waveform file.
pub(crate) fn draw_open_sibling_state_file_dialog(
    ctx: &egui::Context,
    dialog: OpenSiblingStateFileDialog,
    msgs: &mut Vec<Message>,
) {
    let mut do_not_show_again = dialog.do_not_show_again;
    egui::Window::new("State file detected")
            .auto_sized()
            .collapsible(false)
            .fixed_pos(ctx.available_rect().center())
            .show(ctx, |ui| {
                let label = ui.label(RichText::new("A state file was detected in the same directory as the loaded file.\nLoad state?").heading());
                ui.set_width(label.rect.width());
                ui.add_space(5.0);
                ui.checkbox(
                    &mut do_not_show_again,
                    "Remember my decision for this session",
                );
                ui.add_space(14.0);
                ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                    // Sets the style when focused
                    ui.style_mut().visuals.widgets.active.weak_bg_fill = Color32::BLUE;
                    let load_button = ui.button("Load");
                    let dont_load_button = ui.button("Don't load");
                    ctx.memory_mut(|mem| {
                        if !matches!(mem.focused(), Some(id) if id == load_button.id || id == dont_load_button.id)
                        {
                            mem.request_focus(load_button.id);
                        }
                    });

                    if load_button.clicked() {
                        msgs.push(Message::CloseOpenSiblingStateFileDialog {
                            load_state: true,
                            do_not_show_again,
                        });
                    } else if dont_load_button.clicked() {
                        msgs.push(Message::CloseOpenSiblingStateFileDialog {
                            load_state: false,
                            do_not_show_again,
                        });
                    } else if do_not_show_again != dialog.do_not_show_again {
                        msgs.push(Message::UpdateOpenSiblingStateFileDialog(OpenSiblingStateFileDialog {
                            do_not_show_again,
                        }));
                    }
                });
            });
}

/// Draw a dialog that asks for user confirmation before re-loading a file.
/// This is triggered by a file loading event from disk.
pub(crate) fn draw_reload_waveform_dialog(
    ctx: &egui::Context,
    dialog: ReloadWaveformDialog,
    msgs: &mut Vec<Message>,
) {
    let mut do_not_show_again = dialog.do_not_show_again;
    egui::Window::new("File Change")
        .auto_sized()
        .collapsible(false)
        .fixed_pos(ctx.available_rect().center())
        .show(ctx, |ui| {
            let label = ui.label(RichText::new("File on disk has changed. Reload?").heading());
            ui.set_width(label.rect.width());
            ui.add_space(5.0);
            ui.checkbox(
                &mut do_not_show_again,
                "Remember my decision for this session",
            );
            ui.add_space(14.0);
            ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                // Sets the style when focused
                ui.style_mut().visuals.widgets.active.weak_bg_fill = Color32::BLUE;
                let reload_button = ui.button("Reload");
                let leave_button = ui.button("Leave");
                ctx.memory_mut(|mem| {
                    if !matches!(mem.focused(), Some(id) if id == reload_button.id || id == leave_button.id)
                    {
                        mem.request_focus(reload_button.id);
                    }
                });

                if reload_button.clicked() {
                    msgs.push(Message::CloseReloadWaveformDialog {
                        reload_file: true,
                        do_not_show_again,
                    });
                } else if leave_button.clicked() {
                    msgs.push(Message::CloseReloadWaveformDialog {
                        reload_file: false,
                        do_not_show_again,
                    });
                } else if do_not_show_again != dialog.do_not_show_again {
                    msgs.push(Message::UpdateReloadWaveformDialog(ReloadWaveformDialog {
                        do_not_show_again,
                    }));
                }
            });
        });
}
