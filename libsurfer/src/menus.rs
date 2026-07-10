//! Menu handling.
use egui::containers::menu::{MenuConfig, SubMenuButton};
use egui::{Button, Panel, PopupCloseBehavior, TextWrapMode, Ui};
use eyre::WrapErr as _;
use futures::executor::block_on;
use itertools::Itertools;
use std::sync::atomic::Ordering;
use surfer_translation_types::{TranslationPreference, Translator};

use crate::config::{PrimaryMouseDrag, TransitionValue};
use crate::displayed_item_tree::VisibleItemIndex;
use crate::hierarchy::{HierarchyStyle, ParameterDisplayLocation, ScopeExpandType};
use crate::keyboard_shortcuts::ShortcutAction;
use crate::message::MessageTarget;
use crate::source::SourceId;
use crate::table::{MultiSignalEntry, TableModelSpec};
use crate::trace_style::TraceStyle;
use crate::transaction_container::{StreamScopeRef, TransactionStreamRef};
use crate::transaction_events::EventDisplayMode;
use crate::wave_container::{FieldRef, VariableRefExt};
use crate::wave_data::{ScopeType, WaveData};
use crate::wave_source::LoadOptions;
use crate::{
    SystemState,
    clock_highlighting::clock_highlight_type_menu,
    config::ArrowKeyBindings,
    displayed_item::{DisplayedFieldRef, DisplayedItem},
    file_dialog::OpenMode,
    message::Message,
    time::{timeformat_menu, timeunit_menu},
    toolbar::toolbar_group_specs,
    variable_name_type::VariableNameType,
};
use surfer_wcp::{WcpEvent, WcpSCMessage};

// Button builder. Short name because we use it a ton
struct ButtonBuilder {
    text: String,
    shortcut: Option<String>,
    message: Message,
    enabled: bool,
}

impl ButtonBuilder {
    fn new(text: impl Into<String>, message: Message) -> Self {
        Self {
            text: text.into(),
            message,
            shortcut: None,
            enabled: true,
        }
    }

    fn shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    #[cfg_attr(not(feature = "python"), allow(dead_code))]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn add_closing_menu(self, msgs: &mut Vec<Message>, ui: &mut Ui) {
        self.add_inner(msgs, ui);
    }

    pub fn add_inner(self, msgs: &mut Vec<Message>, ui: &mut Ui) {
        let button = Button::new(self.text);
        let button = if let Some(s) = self.shortcut {
            button.shortcut_text(s)
        } else {
            button
        };
        if ui.add_enabled(self.enabled, button).clicked() {
            msgs.push(self.message);
        }
    }
}

impl SystemState {
    pub fn add_menu_panel(&self, ui: &mut Ui, msgs: &mut Vec<Message>) {
        Panel::top("menu").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                self.menu_contents(ui, msgs);
            });
        });
    }

    pub fn menu_contents(&self, ui: &mut Ui, msgs: &mut Vec<Message>) {
        /// Helper function to get a new `ButtonBuilder`.
        fn b(text: impl Into<String>, message: Message) -> ButtonBuilder {
            ButtonBuilder::new(text, message)
        }

        let waves_loaded = self.user.waves.is_some();

        ui.menu_button("File", |ui| {
            b("Open file...", Message::OpenFileDialog(OpenMode::Open))
                .shortcut(
                    self.user
                        .config
                        .shortcuts
                        .format_shortcut(ShortcutAction::OpenFile),
                )
                .add_closing_menu(msgs, ui);
            b("Switch file...", Message::OpenFileDialog(OpenMode::Switch))
                .shortcut(
                    self.user
                        .config
                        .shortcuts
                        .format_shortcut(ShortcutAction::SwitchFile),
                )
                .add_closing_menu(msgs, ui);
            b(
                "Add source...",
                Message::OpenFileDialog(OpenMode::AddSource),
            )
            .add_closing_menu(msgs, ui);
            #[cfg(not(target_arch = "wasm32"))]
            b(
                "Open waveform + transactions...",
                Message::OpenFileDialog(OpenMode::OpenMultipleSources),
            )
            .add_closing_menu(msgs, ui);

            #[cfg(not(target_arch = "wasm32"))]
            ui.menu_button("Recent files", |ui| {
                if self.file_history.files().is_empty() {
                    ui.add_enabled(false, Button::new("No recent files"));
                    return;
                }

                let labels = self.file_history.display_labels();
                for (path, label) in self.file_history.files().iter().zip(labels.iter()) {
                    let response = ui.button(label).on_hover_text(path.as_str());
                    if response.clicked() {
                        msgs.push(Message::LoadFile(path.clone(), LoadOptions::Clear));
                    }
                }
            });
            b(
                "Reload",
                Message::ReloadWaveform(self.user.config.behavior.keep_during_reload),
            )
            .shortcut(
                self.user
                    .config
                    .shortcuts
                    .format_shortcut(ShortcutAction::ReloadWaveform),
            )
            .enabled(self.user.waves.is_some())
            .add_closing_menu(msgs, ui);

            b("Load state...", Message::LoadStateFile(None)).add_closing_menu(msgs, ui);
            #[cfg(not(target_arch = "wasm32"))]
            {
                let save_text = if self.user.state_file.is_some() {
                    "Save state"
                } else {
                    "Save state..."
                };
                b(
                    save_text,
                    Message::SaveStateFile(self.user.state_file.clone()),
                )
                .shortcut(
                    self.user
                        .config
                        .shortcuts
                        .format_shortcut(ShortcutAction::SaveStateFile),
                )
                .add_closing_menu(msgs, ui);
            }
            b("Save state as...", Message::SaveStateFile(None)).add_closing_menu(msgs, ui);
            b(
                "Open URL...",
                Message::SetUrlEntryVisible(
                    true,
                    Some(Box::new(|url: String| {
                        Message::LoadWaveformFileFromUrl(url.clone(), LoadOptions::Clear)
                    })),
                ),
            )
            .add_closing_menu(msgs, ui);
            #[cfg(target_arch = "wasm32")]
            b("Run command file...", Message::OpenCommandFileDialog)
                .enabled(waves_loaded)
                .add_closing_menu(msgs, ui);
            #[cfg(not(target_arch = "wasm32"))]
            b("Run command file...", Message::OpenCommandFileDialog).add_closing_menu(msgs, ui);
            b(
                "Run command file from URL...",
                Message::SetUrlEntryVisible(
                    true,
                    Some(Box::new(|url: String| {
                        Message::LoadCommandFileFromUrl(url.clone())
                    })),
                ),
            )
            .add_closing_menu(msgs, ui);

            #[cfg(feature = "python")]
            {
                b("Add Python translator", Message::OpenPythonPluginDialog)
                    .add_closing_menu(msgs, ui);
                b("Reload Python translator", Message::ReloadPythonPlugin)
                    .enabled(self.translators.has_python_translator())
                    .add_closing_menu(msgs, ui);
            }
            #[cfg(not(target_arch = "wasm32"))]
            b("Exit", Message::Exit).add_closing_menu(msgs, ui);
        });
        ui.menu_button("View", |ui: &mut Ui| {
            let viewport_idx = self
                .user
                .waves
                .as_ref()
                .map_or(0, |waves| waves.last_active_viewport_idx);
            ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);
            b(
                "Zoom in",
                Message::CanvasZoom {
                    mouse_ptr: None,
                    delta: 0.5,
                    viewport_idx,
                },
            )
            .shortcut(
                self.user
                    .config
                    .shortcuts
                    .format_shortcut(ShortcutAction::ZoomIn),
            )
            .enabled(waves_loaded)
            .add_closing_menu(msgs, ui);

            b(
                "Zoom out",
                Message::CanvasZoom {
                    mouse_ptr: None,
                    delta: 2.0,
                    viewport_idx,
                },
            )
            .shortcut(
                self.user
                    .config
                    .shortcuts
                    .format_shortcut(ShortcutAction::ZoomOut),
            )
            .enabled(waves_loaded)
            .add_closing_menu(msgs, ui);

            b("Zoom to fit", Message::ZoomToFit { viewport_idx })
                .shortcut(
                    self.user
                        .config
                        .shortcuts
                        .format_shortcut(ShortcutAction::ZoomToFit),
                )
                .enabled(waves_loaded)
                .add_closing_menu(msgs, ui);

            ui.separator();

            b("Go to start", Message::GoToStart { viewport_idx })
                .shortcut(
                    self.user
                        .config
                        .shortcuts
                        .format_shortcut(ShortcutAction::GoToStart),
                )
                .enabled(waves_loaded)
                .add_closing_menu(msgs, ui);
            b("Go to end", Message::GoToEnd { viewport_idx })
                .shortcut(
                    self.user
                        .config
                        .shortcuts
                        .format_shortcut(ShortcutAction::GoToEnd),
                )
                .enabled(waves_loaded)
                .add_closing_menu(msgs, ui);
            ui.separator();
            b("Add viewport", Message::AddViewport)
                .enabled(waves_loaded)
                .add_closing_menu(msgs, ui);
            b("Remove viewport", Message::RemoveViewport)
                .enabled(waves_loaded)
                .add_closing_menu(msgs, ui);
            let konata_generators = self
                .user
                .waves
                .as_ref()
                .into_iter()
                .flat_map(|waves| {
                    waves.source_ids().into_iter().flat_map(move |source| {
                        let source_label = waves.source_label_for(source).unwrap_or_default();
                        waves.transactions_for_source(source).into_iter().flat_map(
                            move |transactions| {
                                transactions.get_generators().into_iter().filter_map({
                                    let source_label = source_label.clone();
                                    move |generator| {
                                        transactions
                                            .event_index()
                                            .events_generator_of(generator.id)
                                            .map(|_| {
                                                (
                                                    source,
                                                    source_label.clone(),
                                                    TransactionStreamRef::new_gen(
                                                        generator.stream_id,
                                                        generator.id,
                                                        generator.name.clone(),
                                                    ),
                                                )
                                            })
                                    }
                                })
                            },
                        )
                    })
                })
                .collect_vec();
            ui.menu_button("New Konata view…", |ui| {
                if konata_generators.is_empty() {
                    ui.add_enabled(false, Button::new("No pipeline generators"));
                }
                for (source, source_label, generator) in &konata_generators {
                    if ui
                        .button(format!("{source_label}: {}", generator.name))
                        .clicked()
                    {
                        msgs.push(Message::OpenKonataView {
                            source: *source,
                            generator: generator.clone(),
                        });
                        ui.close();
                    }
                }
            });
            ui.separator();

            b(
                "Toggle side panel",
                Message::SetSidePanelVisible(!self.show_hierarchy()),
            )
            .shortcut(
                self.user
                    .config
                    .shortcuts
                    .format_shortcut(ShortcutAction::ToggleSidePanel),
            )
            .add_closing_menu(msgs, ui);
            b("Toggle menu", Message::SetMenuVisible(!self.show_menu()))
                .shortcut(
                    self.user
                        .config
                        .shortcuts
                        .format_shortcut(ShortcutAction::ToggleMenu),
                )
                .add_closing_menu(msgs, ui);
            b(
                "Toggle toolbar",
                Message::SetToolbarVisible(!self.show_toolbar()),
            )
            .shortcut(
                self.user
                    .config
                    .shortcuts
                    .format_shortcut(ShortcutAction::ToggleToolbar),
            )
            .add_closing_menu(msgs, ui);
            b(
                "Toggle overview",
                Message::SetOverviewVisible(!self.show_overview()),
            )
            .add_closing_menu(msgs, ui);
            b(
                "Toggle statusbar",
                Message::SetStatusbarVisible(!self.show_statusbar()),
            )
            .add_closing_menu(msgs, ui);
            b(
                "Toggle timeline",
                Message::SetDefaultTimeline(!self.show_default_timeline()),
            )
            .add_closing_menu(msgs, ui);
            if self
                .user
                .waves
                .as_ref()
                .is_some_and(|waves| waves.inner.is_transactions())
            {
                b(
                    "Toggle raw event generators",
                    Message::SetShowRawEventGenerators(!self.user.show_raw_event_generators),
                )
                .add_closing_menu(msgs, ui);
            }
            #[cfg(not(target_arch = "wasm32"))]
            b("Toggle full screen", Message::ToggleFullscreen)
                .shortcut("F11")
                .add_closing_menu(msgs, ui);
            ui.menu_button("Theme", |ui| {
                ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);
                b("Default theme", Message::SelectTheme(None)).add_closing_menu(msgs, ui);
                for theme_name in self.user.config.theme.theme_names.clone() {
                    b(theme_name.clone(), Message::SelectTheme(Some(theme_name)))
                        .add_closing_menu(msgs, ui);
                }
            });
            ui.menu_button("UI zoom factor", |ui| {
                ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);
                for scale in &self.user.config.layout.zoom_factors {
                    ui.radio(
                        self.ui_zoom_factor() == *scale,
                        format!("{} %", scale * 100.),
                    )
                    .clicked()
                    .then(|| msgs.push(Message::SetUIZoomFactor(*scale)));
                }
            });
            ui.menu_button("Toolbar groups", |ui| {
                for spec in toolbar_group_specs() {
                    let mut enabled = self.toolbar_group_enabled(spec.id);
                    if ui.checkbox(&mut enabled, spec.label).changed() {
                        msgs.push(Message::SetToolbarGroupEnabled(
                            spec.id.to_string(),
                            enabled,
                        ));
                    }
                }
            });
        });

        ui.menu_button("Settings", |ui| {
            ui.menu_button("Clock highlighting", |ui| {
                clock_highlight_type_menu(ui, msgs, self.clock_highlight_type());
            });
            ui.menu_button("Time unit", |ui| {
                timeunit_menu(ui, msgs, &self.user.wanted_timeunit);
            });
            ui.menu_button("Time format", |ui| {
                timeformat_menu(ui, msgs, &self.get_time_format());
            });
            if let Some(waves) = &self.user.waves {
                let variable_name_type = waves.default_variable_name_type;
                ui.menu_button("Variable names", |ui| {
                    for name_type in enum_iterator::all::<VariableNameType>() {
                        ui.radio(variable_name_type == name_type, name_type.to_string())
                            .clicked()
                            .then(|| {
                                msgs.push(Message::ForceVariableNameTypes(name_type));
                            });
                    }
                });
            }
            ui.menu_button("Variable name alignment", |ui| {
                let align_right = self
                    .user
                    .align_names_right
                    .unwrap_or_else(|| self.user.config.layout.align_names_right());
                ui.radio(!align_right, "Left").clicked().then(|| {
                    msgs.push(Message::SetNameAlignRight(false));
                });
                ui.radio(align_right, "Right").clicked().then(|| {
                    msgs.push(Message::SetNameAlignRight(true));
                });
            });

            ui.menu_button("Hierarchy", |ui| {
                self.hierarchy_menu(msgs, ui);
            });

            ui.menu_button("Parameter display location", |ui| {
                for location in enum_iterator::all::<ParameterDisplayLocation>() {
                    ui.radio(
                        self.parameter_display_location() == location,
                        location.to_string(),
                    )
                    .clicked()
                    .then(|| {
                        msgs.push(Message::SetParameterDisplayLocation(location));
                    });
                }
            });

            ui.menu_button("Arrow keys", |ui| {
                for binding in enum_iterator::all::<ArrowKeyBindings>() {
                    ui.radio(self.arrow_key_bindings() == binding, binding.to_string())
                        .clicked()
                        .then(|| {
                            msgs.push(Message::SetArrowKeyBindings(binding));
                        });
                }
            });

            ui.menu_button("Primary mouse button drag", |ui| {
                for behavior in enum_iterator::all::<PrimaryMouseDrag>() {
                    ui.radio(
                        self.primary_button_drag_behavior() == behavior,
                        behavior.to_string(),
                    )
                    .clicked()
                    .then(|| {
                        msgs.push(Message::SetPrimaryMouseDragBehavior(behavior));
                    });
                }
            });

            ui.menu_button("Value at transition", |ui| {
                for transition_value in enum_iterator::all::<TransitionValue>() {
                    ui.radio(
                        self.transition_value() == transition_value,
                        transition_value.to_string(),
                    )
                    .clicked()
                    .then(|| {
                        msgs.push(Message::SetTransitionValue(transition_value));
                    });
                }
            });

            ui.radio(self.show_ticks(), "Show tick lines")
                .clicked()
                .then(|| {
                    msgs.push(Message::SetTickLines(!self.show_ticks()));
                });

            ui.radio(self.show_tooltip(), "Show variable tooltip")
                .clicked()
                .then(|| {
                    msgs.push(Message::SetVariableTooltip(!self.show_tooltip()));
                });

            ui.radio(self.show_scope_tooltip(), "Show scope tooltip")
                .clicked()
                .then(|| {
                    msgs.push(Message::SetScopeTooltip(!self.show_scope_tooltip()));
                });

            ui.radio(self.show_variable_indices(), "Show variable indices")
                .clicked()
                .then(|| {
                    msgs.push(Message::SetShowIndices(!self.show_variable_indices()));
                });

            ui.radio(self.show_variable_direction(), "Show variable direction")
                .clicked()
                .then(|| {
                    msgs.push(Message::SetShowVariableDirection(
                        !self.show_variable_direction(),
                    ));
                });

            ui.radio(self.show_empty_scopes(), "Show empty scopes")
                .clicked()
                .then(|| {
                    msgs.push(Message::SetShowEmptyScopes(!self.show_empty_scopes()));
                });

            ui.radio(self.show_hierarchy_icons(), "Show hierarchy icons")
                .clicked()
                .then(|| {
                    msgs.push(Message::SetShowHierarchyIcons(!self.show_hierarchy_icons()));
                });

            ui.radio(self.highlight_focused(), "Highlight focused")
                .clicked()
                .then(|| msgs.push(Message::SetHighlightFocused(!self.highlight_focused())));

            ui.radio(self.fill_high_values(), "Fill high values")
                .clicked()
                .then(|| {
                    msgs.push(Message::SetFillHighValues(!self.fill_high_values()));
                });
            ui.radio(self.animation_enabled(), "UI animations")
                .clicked()
                .then(|| {
                    msgs.push(Message::EnableAnimations(!self.animation_enabled()));
                });
            ui.radio(self.show_divider_text(), "Show Divider Text")
                .clicked()
                .then(|| {
                    msgs.push(Message::ShowDividerText(!self.show_divider_text()));
                });
            ui.menu_button("Trace style", |ui| {
                for style in enum_iterator::all::<TraceStyle>() {
                    ui.radio(self.trace_style() == style, style.to_string())
                        .clicked()
                        .then(|| {
                            msgs.push(Message::SetTraceStyle(style));
                        });
                }
            });
        });
        ui.menu_button("Help", |ui| {
            b("Quick start", Message::SetQuickStartVisible(true)).add_closing_menu(msgs, ui);
            b("Control keys", Message::SetKeyHelpVisible(true)).add_closing_menu(msgs, ui);
            b("Mouse gestures", Message::SetGestureHelpVisible(true)).add_closing_menu(msgs, ui);

            ui.separator();
            b("Show logs", Message::SetLogsVisible(true)).add_closing_menu(msgs, ui);

            ui.separator();
            b("License information", Message::SetLicenseVisible(true)).add_closing_menu(msgs, ui);
            ui.separator();
            b("About", Message::SetAboutVisible(true)).add_closing_menu(msgs, ui);
        });
    }

    pub fn hierarchy_menu(&self, msgs: &mut Vec<Message>, ui: &mut Ui) {
        for style in enum_iterator::all::<HierarchyStyle>() {
            ui.radio(self.hierarchy_style() == style, style.to_string())
                .clicked()
                .then(|| {
                    msgs.push(Message::SetHierarchyStyle(style));
                });
        }
    }

    pub fn item_context_menu(
        &self,
        path: Option<&FieldRef>,
        msgs: &mut Vec<Message>,
        ui: &mut Ui,
        vidx: VisibleItemIndex,
        show_reset_name: bool,
        group_target: MessageTarget<VisibleItemIndex>,
    ) {
        let Some(waves) = &self.user.waves else {
            return;
        };

        let (clicked_item_ref, clicked_item) = waves
            .items_tree
            .get_visible(vidx)
            .map(|node| (node.item_ref, &waves.displayed_items[&node.item_ref]))
            .unwrap();

        if let Some(source) = source_for_context_item(clicked_item)
            && waves.source_count() > 1
        {
            if ui.button("Show Source").clicked() {
                msgs.extend(show_source_messages(source));
                ui.close();
            }
            if let Some(reveal_messages) = reveal_in_hierarchy_messages(waves, clicked_item, path)
                && ui.button("Reveal in Hierarchy").clicked()
            {
                msgs.extend(reveal_messages);
                ui.close();
            }
        }

        if let Some(path) = path {
            let dfr = DisplayedFieldRef {
                item: clicked_item_ref,
                field: path.field.clone(),
            };
            self.add_format_menu(&dfr, clicked_item, path, msgs, ui, group_target);
        }

        ui.menu_button("Color", |ui| {
            let selected_color = clicked_item.color();
            for color_name in self.user.config.theme.colors.keys() {
                ui.radio(selected_color == Some(color_name), color_name)
                    .clicked()
                    .then(|| {
                        msgs.push(Message::ItemColorChange(
                            group_target,
                            Some(color_name.clone()),
                        ));
                    });
            }
            ui.separator();
            ui.radio(selected_color.is_none(), "Default")
                .clicked()
                .then(|| {
                    msgs.push(Message::ItemColorChange(group_target, None));
                });
        });

        ui.menu_button("Background color", |ui| {
            let selected_color = clicked_item.background_color();
            for color_name in self.user.config.theme.colors.keys() {
                ui.radio(selected_color == Some(color_name), color_name)
                    .clicked()
                    .then(|| {
                        msgs.push(Message::ItemBackgroundColorChange(
                            group_target,
                            Some(color_name.clone()),
                        ));
                    });
            }
            ui.separator();
            ui.radio(selected_color.is_none(), "Default")
                .clicked()
                .then(|| {
                    msgs.push(Message::ItemBackgroundColorChange(group_target, None));
                });
        });

        if let DisplayedItem::Variable(variable) = clicked_item {
            ui.menu_button("Name", |ui| {
                let variable_name_type = variable.display_name_type;
                for name_type in enum_iterator::all::<VariableNameType>() {
                    ui.radio(variable_name_type == name_type, name_type.to_string())
                        .clicked()
                        .then(|| {
                            msgs.push(Message::ChangeVariableNameType(group_target, name_type));
                        });
                }
            });

            ui.menu_button("Height", |ui| {
                let selected_size = clicked_item.height_scaling_factor();
                for size in &self.user.config.layout.waveforms_line_height_multiples {
                    ui.radio(selected_size == *size, format!("{size}"))
                        .clicked()
                        .then(|| {
                            msgs.push(Message::ItemHeightScalingFactorChange(group_target, *size));
                        });
                }
            });

            // Count selected variables to decide between single vs multi-signal table.
            let selected_variables: Vec<MultiSignalEntry> = waves
                .items_tree
                .iter_visible_selected()
                .filter_map(|node| {
                    let item = waves.displayed_items.get(&node.item_ref)?;
                    if let DisplayedItem::Variable(var) = item {
                        Some(MultiSignalEntry {
                            source: var.source,
                            variable: var.variable_ref.clone(),
                            field: vec![],
                        })
                    } else {
                        None
                    }
                })
                .collect();

            if selected_variables.len() >= 2 {
                if ui.button("Multi-signal change list").clicked() {
                    msgs.push(Message::AddTableTile {
                        spec: TableModelSpec::MultiSignalChangeList {
                            variables: selected_variables,
                        },
                    });
                    ui.close();
                }
            } else if ui.button("Signal change list").clicked() {
                let (variable_ref, field) = path
                    .map(|path| (path.root.clone(), path.field.clone()))
                    .unwrap_or_else(|| (variable.variable_ref.clone(), Vec::new()));

                msgs.push(Message::AddTableTile {
                    spec: TableModelSpec::SignalChangeList {
                        source: variable.source,
                        variable: variable_ref,
                        field,
                    },
                });
                ui.close();
            }

            if self.has_signal_analysis_selection()
                && ui.button("Analyze selected signals...").clicked()
            {
                msgs.push(Message::OpenSignalAnalysisWizard);
                ui.close();
            }

            if self.wcp_greeted_signal.load(Ordering::Relaxed) {
                if self.wcp_client_capabilities.goto_declaration
                    && ui.button("Go to declaration").clicked()
                {
                    let variable = variable.variable_ref.full_path_string_no_index();
                    self.channels.wcp_s2c_sender.as_ref().map(|ch| {
                        block_on(
                            ch.send(WcpSCMessage::event(WcpEvent::goto_declaration { variable })),
                        )
                    });
                }
                if self.wcp_client_capabilities.add_drivers && ui.button("Add drivers").clicked() {
                    let variable = variable.variable_ref.full_path_string_no_index();
                    self.channels.wcp_s2c_sender.as_ref().map(|ch| {
                        block_on(ch.send(WcpSCMessage::event(WcpEvent::add_drivers { variable })))
                    });
                }
                if self.wcp_client_capabilities.add_loads && ui.button("Add loads").clicked() {
                    let variable = variable.variable_ref.full_path_string_no_index();
                    self.channels.wcp_s2c_sender.as_ref().map(|ch| {
                        block_on(ch.send(WcpSCMessage::event(WcpEvent::add_loads { variable })))
                    });
                }
            }
        }

        if let DisplayedItem::Stream(stream) = clicked_item {
            let events_enabled = self.user.config.behavior.ftr_events_enabled();
            if stream.transaction_stream_ref.gen_id.is_some() {
                // Single generator — open one table
                if ui.button("Show transactions in table").clicked() {
                    msgs.push(Message::OpenTransactionTable {
                        source: stream.source,
                        generator: stream.transaction_stream_ref.clone(),
                    });
                    ui.close();
                }
                let has_events = events_enabled
                    && waves
                        .transactions_for_source(stream.source)
                        .is_some_and(|tc| {
                            stream
                                .transaction_stream_ref
                                .gen_id
                                .and_then(|gen_id| {
                                    tc.event_index().conforming_events_generator_of(gen_id)
                                })
                                .is_some()
                        });
                if has_events && ui.button("Show events in table").clicked() {
                    msgs.push(Message::OpenEventTable {
                        source: stream.source,
                        generator: stream.transaction_stream_ref.clone(),
                    });
                    ui.close();
                }
                let has_pipeline_pair =
                    waves
                        .transactions_for_source(stream.source)
                        .is_some_and(|transactions| {
                            stream
                                .transaction_stream_ref
                                .gen_id
                                .and_then(|generator| {
                                    transactions.event_index().events_generator_of(generator)
                                })
                                .is_some()
                        });
                if has_pipeline_pair && ui.button("Open in Konata view").clicked() {
                    msgs.push(Message::OpenKonataView {
                        source: stream.source,
                        generator: stream.transaction_stream_ref.clone(),
                    });
                    ui.close();
                }
            } else if let Some(tc) = waves.transactions_for_source(stream.source) {
                // Stream — open one table per generator
                let scope = StreamScopeRef::Stream(stream.transaction_stream_ref.clone());
                let generators = tc.generators_in_stream(&scope);
                if !generators.is_empty() && ui.button("Show transactions in table").clicked() {
                    for gen_ref in generators {
                        msgs.push(Message::OpenTransactionTable {
                            source: stream.source,
                            generator: gen_ref,
                        });
                    }
                    ui.close();
                }
            }

            // Three-way event display control for rows whose generators
            // carry FTR events
            let row_has_events = events_enabled
                && waves
                    .transactions_for_source(stream.source)
                    .is_some_and(|tc| {
                        let index = tc.event_index();
                        match stream.transaction_stream_ref.gen_id {
                            Some(gen_id) => index.conforming_events_generator_of(gen_id).is_some(),
                            None => tc
                                .get_stream(stream.transaction_stream_ref.stream_id)
                                .is_some_and(|s| {
                                    s.generators
                                        .iter()
                                        .any(|gen_id| index.is_conforming_events_generator(*gen_id))
                                }),
                        }
                    });
            if row_has_events {
                ui.menu_button("Events", |ui| {
                    for (mode, label) in [
                        (EventDisplayMode::Overlay, "Overlay"),
                        (EventDisplayMode::SeparateRow, "Separate row"),
                        (EventDisplayMode::Hidden, "Hidden"),
                    ] {
                        ui.radio(stream.event_display_mode == mode, label)
                            .clicked()
                            .then(|| {
                                msgs.push(Message::SetEventDisplayMode { vidx, mode });
                            });
                    }
                });
            }
        }

        if let Some(path) = path
            && let DisplayedItem::Variable(variable) = clicked_item
            && let Some(wave_container) = waves.waves_for_source(variable.source)
        {
            let meta = wave_container.variable_meta(&path.root).ok();
            let is_parameter = meta
                .as_ref()
                .is_some_and(surfer_translation_types::VariableMeta::is_parameter);
            if !is_parameter && ui.button("Expand scope").clicked() {
                let scope_path = path.root.path.clone();
                let scope_type = ScopeType::WaveScope(scope_path.clone());
                msgs.push(Message::SetActiveScopeFromSource(
                    variable.source,
                    Some(scope_type),
                ));
                msgs.push(Message::ExpandScope(ScopeExpandType::ExpandSpecific(
                    scope_path,
                )));
            }

            if wave_container.supports_analog() {
                let displayed_field_ref: DisplayedFieldRef = clicked_item_ref.into();
                let translator = waves.variable_translator(&displayed_field_ref, &self.translators);
                let type_limits_available = meta
                    .as_ref()
                    .is_some_and(|m| translator.numeric_range(m).is_some());

                SubMenuButton::new("Analog")
                    .config(
                        MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside),
                    )
                    .ui(ui, |ui| {
                        Self::analog_submenu(
                            ui,
                            msgs,
                            variable,
                            group_target,
                            type_limits_available,
                        );
                    });
            }
        }

        if ui.button("Rename").clicked() {
            let name = clicked_item.name();
            msgs.push(Message::FocusItem(vidx));
            msgs.push(Message::ShowCommandPrompt(
                "item_rename ".to_owned(),
                Some(name),
            ));
        }

        if show_reset_name && ui.button("Reset Name").clicked() {
            msgs.push(Message::ItemNameReset(group_target));
        }

        if ui.button("Remove").clicked() {
            if waves
                .items_tree
                .iter_visible_selected()
                .map(|node| node.item_ref)
                .contains(&clicked_item_ref)
            {
                msgs.push(Message::UnfocusItem);
            }
            msgs.push(Message::RemoveVisibleItems(group_target));
        }
        if let Some(path) = path {
            // Actual signal. Not one of: divider, timeline, marker.
            if ui.button("Show frame buffer").clicked() {
                msgs.push(Message::SetFrameBufferVisibleVariable(Some(vidx)));
            }
            if let DisplayedItem::Variable(_) = clicked_item
                && ui.button("Show Memory Viewer").clicked()
            {
                msgs.push(Message::OpenMemoryViewer {
                    scope: path.root.path.clone(),
                    name: Some(path.root.name.clone()),
                });
            }
            ui.menu_button("Copy", |ui| {
                if waves.cursor.is_some() && ui.button("Value").clicked() {
                    msgs.push(Message::VariableValueToClipbord(MessageTarget::Explicit(
                        vidx,
                    )));
                }
                if ui.button("Name").clicked() {
                    msgs.push(Message::VariableNameToClipboard(MessageTarget::Explicit(
                        vidx,
                    )));
                }
                if ui.button("Full name").clicked() {
                    msgs.push(Message::VariableFullNameToClipboard(
                        MessageTarget::Explicit(vidx),
                    ));
                }
            });
        }
        ui.separator();
        ui.menu_button("Insert", |ui| {
            if ui.button("Divider").clicked() {
                msgs.push(Message::AddDivider(None, Some(vidx)));
            }
            if ui.button("Timeline").clicked() {
                msgs.push(Message::AddTimeLine(Some(vidx)));
            }
        });

        ui.menu_button("Group", |ui| {
            let info = waves
                .items_tree
                .iter_visible_extra()
                .find(|info| info.node.item_ref == clicked_item_ref)
                .expect("Inconsistent, could not find displayed signal in tree");

            if ui.button("Create").clicked() {
                msgs.push(Message::GroupNew {
                    name: None,
                    before: Some(info.idx),
                    items: None,
                });
            }
            if matches!(clicked_item, DisplayedItem::Group(_)) {
                if ui.button("Dissolve").clicked() {
                    msgs.push(Message::GroupDissolve(Some(clicked_item_ref)));
                }

                let (text, msg, msg_recursive) = if info.node.unfolded {
                    (
                        "Collapse",
                        Message::GroupFold(Some(clicked_item_ref)),
                        Message::GroupFoldRecursive(Some(clicked_item_ref)),
                    )
                } else {
                    (
                        "Expand",
                        Message::GroupUnfold(Some(clicked_item_ref)),
                        Message::GroupUnfoldRecursive(Some(clicked_item_ref)),
                    )
                };
                if ui.button(text).clicked() {
                    msgs.push(msg);
                }
                if ui.button(text.to_owned() + " recursive").clicked() {
                    msgs.push(msg_recursive);
                }
            }
        });
        if let DisplayedItem::Marker(_) = clicked_item {
            ui.separator();
            if ui.button("View markers").clicked() {
                msgs.push(Message::SetCursorWindowVisible(true));
            }
        }
    }

    fn analog_submenu(
        ui: &mut Ui,
        msgs: &mut Vec<Message>,
        variable: &crate::displayed_item::DisplayedVariable,
        group_target: MessageTarget<VisibleItemIndex>,
        type_limits_available: bool,
    ) {
        use crate::displayed_item::{AnalogRenderStyle, AnalogSettings, AnalogYAxisScale};

        let current = variable.analog.as_ref().map(|a| a.settings);
        let current_style = current.map(|s| s.render_style);
        let current_scale = current.map(|s| s.y_axis_scale);

        ui.label("Render style");
        if ui.radio(current.is_none(), "Off").clicked() && current.is_some() {
            msgs.push(Message::SetAnalogSettings(group_target, None));
        }
        for style in [AnalogRenderStyle::Step, AnalogRenderStyle::Interpolated] {
            if ui
                .radio(current_style == Some(style), style.label())
                .clicked()
                && current_style != Some(style)
            {
                let new = AnalogSettings {
                    render_style: style,
                    ..current.unwrap_or_default()
                };
                msgs.push(Message::SetAnalogSettings(group_target, Some(new)));
            }
        }

        ui.separator();

        ui.label("Y-axis scale");
        for scale in [AnalogYAxisScale::Viewport, AnalogYAxisScale::Global] {
            if ui
                .radio(current_scale == Some(scale), scale.label())
                .clicked()
                && current_scale != Some(scale)
            {
                let new = AnalogSettings {
                    y_axis_scale: scale,
                    ..current.unwrap_or_default()
                };
                msgs.push(Message::SetAnalogSettings(group_target, Some(new)));
            }
        }

        let scale = AnalogYAxisScale::TypeLimits;
        let response = ui.add_enabled(
            type_limits_available,
            egui::RadioButton::new(current_scale == Some(scale), scale.label()),
        );
        if !type_limits_available {
            response.on_disabled_hover_text("Type range not available for this translator");
        } else if response.clicked() && current_scale != Some(scale) {
            let new = AnalogSettings {
                y_axis_scale: scale,
                ..current.unwrap_or_default()
            };
            msgs.push(Message::SetAnalogSettings(group_target, Some(new)));
        }

        ui.separator();
        if ui.button("Done").clicked() {
            ui.close();
        }
    }

    fn add_format_menu(
        &self,
        clicked_field_ref: &DisplayedFieldRef,
        clicked_item: &DisplayedItem,
        path: &FieldRef,
        msgs: &mut Vec<Message>,
        ui: &mut Ui,
        group_target: MessageTarget<VisibleItemIndex>,
    ) {
        // Should not call this unless a variable is selected, and, hence, a VCD is loaded
        let Some(waves) = &self.user.waves else {
            return;
        };

        let (mut preferred_translators, mut bad_translators) = if path.field.is_empty() {
            self.translators
                .all_translator_names()
                .into_iter()
                .partition(|translator_name| {
                    let t = self.translators.get_translator(translator_name);

                    if self
                        .user
                        .blacklisted_translators
                        .contains(&(path.root.clone(), (*translator_name).to_string()))
                    {
                        false
                    } else {
                        match waves
                            .inner
                            .as_waves()
                            .unwrap()
                            .variable_meta(&path.root)
                            .and_then(|meta| t.translates(&meta))
                            .context(format!(
                                "Failed to check if {translator_name} translates {}",
                                path.root.full_path_string_no_index(),
                            )) {
                            Ok(TranslationPreference::Yes) => true,
                            Ok(TranslationPreference::Prefer) => true,
                            Ok(TranslationPreference::No) => false,
                            Err(e) => {
                                msgs.push(Message::BlacklistTranslator(
                                    path.root.clone(),
                                    (*translator_name).to_string(),
                                ));
                                msgs.push(Message::Error(e));
                                false
                            }
                        }
                    }
                })
        } else {
            (self.translators.basic_translator_names(), vec![])
        };

        preferred_translators.sort_by(|a, b| numeric_sort::cmp(a, b));
        bad_translators.sort_by(|a, b| numeric_sort::cmp(a, b));

        let selected_translator = match clicked_item {
            DisplayedItem::Variable(var) => Some(var),
            _ => None,
        }
        .and_then(|displayed_variable| displayed_variable.get_format(&clicked_field_ref.field));

        let mut menu_entry = |ui: &mut Ui, name: &str| {
            ui.radio(selected_translator.is_some_and(|st| st == name), name)
                .clicked()
                .then(|| {
                    let target = match group_target {
                        MessageTarget::Explicit(_) => {
                            MessageTarget::Explicit(clicked_field_ref.clone())
                        }
                        MessageTarget::CurrentSelection => MessageTarget::CurrentSelection,
                    };
                    msgs.push(Message::VariableFormatChange(target, name.to_string()));
                });
        };

        ui.menu_button("Format", |ui| {
            ui.set_min_width(180.0);

            for name in preferred_translators {
                menu_entry(ui, name);
            }

            if !bad_translators.is_empty() {
                ui.separator();

                ui.menu_button("Not recommended", |ui| {
                    ui.set_min_width(180.0);

                    for name in bad_translators {
                        menu_entry(ui, name);
                    }
                });
            }
        });
    }
}

fn source_for_context_item(item: &DisplayedItem) -> Option<SourceId> {
    match item {
        DisplayedItem::Variable(variable) => Some(variable.source),
        DisplayedItem::Placeholder(placeholder) => Some(placeholder.source),
        DisplayedItem::Stream(stream) => Some(stream.source),
        DisplayedItem::Divider(_)
        | DisplayedItem::Marker(_)
        | DisplayedItem::TimeLine(_)
        | DisplayedItem::Group(_) => None,
    }
}

fn show_source_messages(source: SourceId) -> Vec<Message> {
    vec![
        Message::SetSidePanelVisible(true),
        Message::SetActiveScopeFromSource(source, None),
    ]
}

fn reveal_in_hierarchy_messages(
    waves: &WaveData,
    item: &DisplayedItem,
    path: Option<&FieldRef>,
) -> Option<Vec<Message>> {
    match item {
        DisplayedItem::Variable(variable) => {
            let scope = path
                .map(|path| path.root.path.clone())
                .unwrap_or_else(|| variable.variable_ref.path.clone());
            Some(vec![
                Message::SetSidePanelVisible(true),
                Message::SetHierarchyStyle(HierarchyStyle::Tree),
                Message::SetActiveScopeFromSource(
                    variable.source,
                    Some(ScopeType::WaveScope(scope.clone())),
                ),
                Message::ExpandScope(ScopeExpandType::ExpandSpecific(scope)),
            ])
        }
        DisplayedItem::Stream(stream) => {
            let transactions = waves.transactions_for_source(stream.source)?;
            let stream_name = transactions
                .get_stream(stream.transaction_stream_ref.stream_id)
                .map(|stream| stream.name.clone())
                .unwrap_or_else(|| stream.transaction_stream_ref.name.clone());
            let stream_ref = TransactionStreamRef::new_stream(
                stream.transaction_stream_ref.stream_id,
                stream_name,
            );
            Some(vec![
                Message::SetSidePanelVisible(true),
                Message::SetHierarchyStyle(HierarchyStyle::Separate),
                Message::SetActiveScopeFromSource(
                    stream.source,
                    Some(ScopeType::StreamScope(StreamScopeRef::Stream(stream_ref))),
                ),
            ])
        }
        DisplayedItem::Placeholder(_)
        | DisplayedItem::Divider(_)
        | DisplayedItem::Marker(_)
        | DisplayedItem::TimeLine(_)
        | DisplayedItem::Group(_) => None,
    }
}

pub fn generic_context_menu(msgs: &mut Vec<Message>, response: &egui::Response) {
    response.context_menu(|ui| {
        if ui.button("Add divider").clicked() {
            msgs.push(Message::AddDivider(None, None));
        }
        if ui.button("Add timeline").clicked() {
            msgs.push(Message::AddTimeLine(None));
        }
    });
}

#[cfg(test)]
mod tests {
    use ftr_parser::types::{GeneratorId, StreamId};
    use project_root::get_project_root;

    use super::*;
    use crate::{
        StartupParams, WaveSource,
        displayed_item::DisplayedItem,
        wave_container::{FieldRefExt, VariableRef, VariableRefExt},
    };

    fn fixture(path: &str) -> camino::Utf8PathBuf {
        get_project_root().unwrap().join(path).try_into().unwrap()
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn show_source_messages_open_side_panel_and_select_source() {
        let messages = show_source_messages(SourceId(7));
        assert!(matches!(
            messages.first(),
            Some(Message::SetSidePanelVisible(true))
        ));
        assert!(matches!(
            messages.get(1),
            Some(Message::SetActiveScopeFromSource(SourceId(7), None))
        ));
    }

    #[test]
    fn reveal_variable_in_hierarchy_selects_source_scope_and_expands_tree() {
        let runtime = runtime();
        let _guard = runtime.enter();
        let mut state = SystemState::new_default_config()
            .unwrap()
            .with_params(StartupParams {
                waves: Some(WaveSource::File(fixture("examples/fused_ftr_wave.vcd"))),
                startup_commands: vec![],
                ..Default::default()
            });
        crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

        state.update(Message::AddVariables(vec![
            VariableRef::from_hierarchy_string("tb.clk"),
        ]));
        crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

        let waves = state.user.waves.as_ref().expect("waves loaded");
        let item = waves
            .displayed_items
            .values()
            .find(|item| matches!(item, DisplayedItem::Variable(_)))
            .expect("variable row");
        let DisplayedItem::Variable(variable) = item else {
            unreachable!();
        };
        let field = FieldRef::without_fields(variable.variable_ref.clone());
        let messages = reveal_in_hierarchy_messages(waves, item, Some(&field))
            .expect("variable reveal messages");

        assert!(matches!(
            messages.first(),
            Some(Message::SetSidePanelVisible(true))
        ));
        assert!(matches!(
            messages.get(1),
            Some(Message::SetHierarchyStyle(HierarchyStyle::Tree))
        ));
        assert!(matches!(
            messages.get(2),
            Some(Message::SetActiveScopeFromSource(
                SourceId(0),
                Some(ScopeType::WaveScope(scope)),
            )) if scope == &VariableRef::from_hierarchy_string("tb.clk").path
        ));
        assert!(matches!(
            messages.get(3),
            Some(Message::ExpandScope(ScopeExpandType::ExpandSpecific(scope)))
                if scope == &VariableRef::from_hierarchy_string("tb.clk").path
        ));
    }

    #[test]
    fn reveal_transaction_row_selects_owning_stream_scope() {
        let runtime = runtime();
        let _guard = runtime.enter();
        let mut state = SystemState::new_default_config()
            .unwrap()
            .with_params(StartupParams {
                waves: Some(WaveSource::File(fixture("examples/my_db.ftr"))),
                startup_commands: vec![],
                ..Default::default()
            });
        crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

        state.update(Message::AddStreamOrGenerator(
            TransactionStreamRef::new_gen(
                StreamId(1),
                GeneratorId(4),
                "pipelined_stream.read".to_string(),
            ),
        ));

        let waves = state.user.waves.as_ref().expect("waves loaded");
        let item = waves
            .displayed_items
            .values()
            .find(|item| matches!(item, DisplayedItem::Stream(_)))
            .expect("stream row");
        let messages =
            reveal_in_hierarchy_messages(waves, item, None).expect("stream reveal messages");

        assert!(matches!(
            messages.first(),
            Some(Message::SetSidePanelVisible(true))
        ));
        assert!(matches!(
            messages.get(1),
            Some(Message::SetHierarchyStyle(HierarchyStyle::Separate))
        ));
        let Some(Message::SetActiveScopeFromSource(
            SourceId(0),
            Some(ScopeType::StreamScope(StreamScopeRef::Stream(stream))),
        )) = messages.get(2)
        else {
            panic!("expected stream active-scope message, got {messages:?}");
        };
        assert_eq!(stream.stream_id, StreamId(1));
        assert!(stream.gen_id.is_none());
        assert_eq!(stream.name, "tr.pipelined_stream");
    }
}
