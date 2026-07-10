use crate::{
    config::TransitionValue,
    dialog::{
        draw_open_sibling_state_file_dialog, draw_reload_waveform_dialog,
        draw_signal_analysis_wizard_dialog,
    },
    displayed_item::DisplayedVariable,
    wave_container::{ScopeId, VarId, VariableMeta},
};
use ecolor::Color32;
#[cfg(not(target_arch = "wasm32"))]
use egui::ViewportCommand;
use egui::{CentralPanel, Frame, Painter, Panel, Ui};
use emath::{Pos2, Rect, Vec2};
use epaint::{
    CornerRadius,
    text::{FontId, LayoutJob, TextFormat},
};
use num::{BigUint, One, Zero};
use tracing::info;

use surfer_translation_types::{
    TranslatedValue, Translator, VariableValue,
    translator::{TrueName, VariableNameInfo},
};

use crate::OUTSTANDING_TRANSACTIONS;
#[cfg(feature = "performance_plot")]
use crate::benchmark::NUM_PERF_SAMPLES;
use crate::config::SurferTheme;
use crate::displayed_item::{DisplayedFieldRef, DisplayedItem};
use crate::displayed_item_tree::VisibleItemIndex;
use crate::help::{
    draw_about_window, draw_control_help_window, draw_license_window, draw_quickstart_help_window,
};
use crate::item_drawing_info::ItemDrawingInfo;
use crate::translation::TranslationResultExt;
use crate::wave_container::VariableRef;
use crate::{
    Message, MoveDir, SystemState, command_prompt::show_command_prompt, hierarchy::HierarchyStyle,
    wave_data::WaveData,
};

pub struct DrawingContext<'a> {
    pub painter: &'a mut Painter,
    pub cfg: &'a DrawConfig,
    pub to_screen: &'a dyn Fn(f32, f32) -> Pos2,
    pub theme: &'a SurferTheme,
}

#[derive(Debug)]
pub struct DrawConfig {
    pub canvas_size: Vec2,
    pub line_height: f32,
    pub text_size: f32,
    pub extra_draw_width: i32,
}

impl DrawConfig {
    #[must_use]
    pub fn new(canvas_size: Vec2, line_height: f32, text_size: f32) -> Self {
        Self {
            canvas_size,
            line_height,
            text_size,
            extra_draw_width: 6,
        }
    }
}

impl eframe::App for SystemState {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        #[cfg(feature = "performance_plot")]
        self.timing.borrow_mut().start_frame();

        if self.continuous_redraw {
            self.invalidate_draw_commands();
        }

        let (fullscreen, window_size) = ui.input(|i| {
            (
                i.viewport().fullscreen.unwrap_or_default(),
                Some(i.viewport_rect().size()),
            )
        });
        #[cfg(target_arch = "wasm32")]
        let _ = fullscreen;

        #[cfg(feature = "performance_plot")]
        self.timing.borrow_mut().start("draw");
        let mut msgs = self.draw(ui, window_size);
        #[cfg(feature = "performance_plot")]
        self.timing.borrow_mut().end("draw");

        #[cfg(feature = "performance_plot")]
        self.timing.borrow_mut().start("push_async_messages");
        self.push_async_messages(&mut msgs);
        #[cfg(feature = "performance_plot")]
        self.timing.borrow_mut().end("push_async_messages");

        #[cfg(feature = "performance_plot")]
        self.timing.borrow_mut().start("update");
        let ui_zoom_factor = self.ui_zoom_factor();
        if ui.zoom_factor() != ui_zoom_factor {
            ui.set_zoom_factor(ui_zoom_factor);
        }

        self.items_to_expand.borrow_mut().clear();

        while let Some(msg) = msgs.pop() {
            #[cfg(not(target_arch = "wasm32"))]
            if let Message::Exit = msg {
                ui.send_viewport_cmd(ViewportCommand::Close);
            }
            #[cfg(not(target_arch = "wasm32"))]
            if let Message::ToggleFullscreen = msg {
                ui.send_viewport_cmd(ViewportCommand::Fullscreen(!fullscreen));
            }
            self.update(msg);
        }
        #[cfg(feature = "performance_plot")]
        self.timing.borrow_mut().end("update");

        self.handle_batch_commands();
        #[cfg(target_arch = "wasm32")]
        self.handle_wasm_external_messages();

        // Keep the Konata "Synchronize scroll" group and the primary waveform viewport showing the
        // same time window. Runs after messages so this frame's zoom/pan is already applied.
        if self.synchronize_konata_wave_viewports() {
            ui.request_repaint();
        }

        let viewport_is_moving = if let Some(waves) = &mut self.user.waves {
            let mut is_moving = false;
            for vp in &mut waves.viewports {
                if vp.is_moving() {
                    vp.move_viewport(ui.input(|i| i.stable_dt));
                    is_moving = true;
                }
            }
            is_moving
        } else {
            false
        };

        if let Some(waves) = self.user.waves.as_ref().and_then(|w| w.inner.as_waves()) {
            waves.tick();
        }

        if viewport_is_moving {
            self.invalidate_draw_commands();
            ui.request_repaint();
        }

        #[cfg(feature = "performance_plot")]
        self.timing.borrow_mut().start("handle_wcp_commands");
        self.handle_wcp_commands();
        #[cfg(feature = "performance_plot")]
        self.timing.borrow_mut().end("handle_wcp_commands");

        // We can save some user battery life by not redrawing unless needed. At the moment,
        // we only need to continuously redraw to make surfer interactive during loading, otherwise
        // we'll let egui manage repainting. In practice
        if self.continuous_redraw
            || self.progress_tracker.is_some()
            || self.user.show_performance
            || OUTSTANDING_TRANSACTIONS.load(std::sync::atomic::Ordering::SeqCst) != 0
        {
            ui.request_repaint();
        }

        #[cfg(feature = "performance_plot")]
        if let Some(prev_cpu) = frame.info().cpu_usage {
            self.rendering_cpu_times.push_back(prev_cpu);
            if self.rendering_cpu_times.len() > NUM_PERF_SAMPLES {
                self.rendering_cpu_times.pop_front();
            }
        }

        #[cfg(feature = "performance_plot")]
        self.timing.borrow_mut().end_frame();

        // Must be last: override cursor after all rendering so nothing can overwrite it.
        if self.toolbar_dragging_group.is_some() {
            ui.ctx()
                .output_mut(|o| o.cursor_icon = egui::CursorIcon::Grabbing);
        }
    }
}

impl SystemState {
    pub(crate) fn draw(&mut self, ui: &mut Ui, window_size: Option<Vec2>) -> Vec<Message> {
        let ctx = ui.ctx().clone();
        let max_width = ui.available_size().x;
        let max_height = ui.available_size().y;

        let mut msgs = vec![];

        if self.user.show_about {
            draw_about_window(&ctx, &mut msgs);
        }

        if self.user.show_license {
            draw_license_window(&ctx, &mut msgs);
        }

        if self.user.show_keys {
            draw_control_help_window(&ctx, &mut msgs, &self.user.config.shortcuts);
        }

        if self.user.show_quick_start {
            draw_quickstart_help_window(&ctx, &mut msgs, &self.user.config.shortcuts);
        }

        if self.user.show_gestures {
            self.mouse_gesture_help(&ctx, &mut msgs);
        }

        if self.user.show_logs {
            self.draw_log_window(&ctx, &mut msgs);
        }

        if self.frame_buffer_content.is_some() {
            self.draw_frame_buffer_window(&ctx, &mut msgs);
        }

        self.draw_memory_viewer_window(&ctx, &mut msgs);

        if let Some(dialog) = self.user.show_reload_suggestion {
            draw_reload_waveform_dialog(&ctx, dialog, &mut msgs);
        }

        if let Some(dialog) = self.user.show_open_sibling_state_file_suggestion {
            draw_open_sibling_state_file_dialog(&ctx, dialog, &mut msgs);
        }

        if let Some((sampling_source, sampling_signal)) = self
            .user
            .show_signal_analysis_wizard
            .as_ref()
            .map(|dialog| (dialog.sampling_source, dialog.sampling_signal.clone()))
        {
            let resolved_mode =
                self.signal_analysis_sampling_mode(sampling_source, &sampling_signal);
            if let Some(dialog) = self.user.show_signal_analysis_wizard.as_mut()
                && draw_signal_analysis_wizard_dialog(&ctx, dialog, resolved_mode, &mut msgs)
            {
                self.user.show_signal_analysis_wizard = None;
                self.user.signal_analysis_wizard_edit_target = None;
            }
        }

        if self.user.show_performance {
            #[cfg(feature = "performance_plot")]
            self.draw_performance_graph(&ctx, &mut msgs);
        }

        if self.user.show_cursor_window
            && let Some(waves) = &self.user.waves
        {
            self.draw_marker_window(waves, &ctx, &mut msgs);
        }

        if self
            .user
            .show_menu
            .unwrap_or_else(|| self.user.config.layout.show_menu())
        {
            self.add_menu_panel(ui, &mut msgs);
        }

        if self.show_toolbar() {
            self.add_toolbar_panel(ui, &mut msgs);
        }

        if self.user.show_url_entry {
            self.draw_load_url(&ctx, &mut msgs);
        }

        if self.user.show_server_file_window {
            self.draw_surver_file_window(&ctx, &mut msgs);
        }

        if self.show_statusbar() {
            self.add_statusbar_panel(ui, self.user.waves.as_ref(), &mut msgs);
        }

        if let Some(waves) = &self.user.waves
            && self.show_overview()
            && !waves.items_tree.is_empty()
        {
            self.add_overview_panel(ui, waves, &mut msgs);
        }

        if self.show_hierarchy() {
            Panel::left("variable select left panel")
                .default_size(300.)
                .size_range(100.0..=max_width)
                .frame(Frame {
                    fill: self.user.config.theme.primary_ui_color.background,
                    ..Default::default()
                })
                .show_inside(ui, |ui| {
                    self.user.sidepanel_width = Some(ui.clip_rect().width());
                    match self.hierarchy_style() {
                        HierarchyStyle::Separate => self.separate(ui, &mut msgs),
                        HierarchyStyle::Tree => self.tree(ui, &mut msgs),
                        HierarchyStyle::Variables => self.variable_list(ui, &mut msgs),
                    }
                });
        }

        if self.command_prompt.visible {
            show_command_prompt(self, &ctx, window_size, &mut msgs);
            if let Some(new_idx) = self.command_prompt.new_selection {
                self.command_prompt.selected = new_idx;
                self.command_prompt.new_selection = None;
            }
        }

        // Preserve the pre-tiles waveform layout until there is a second pane.
        // `egui_tiles` has its own hidden container geometry even with chrome disabled.
        if self.user.tile_tree.is_single_waveform()
            && self
                .user
                .waves
                .as_ref()
                .is_none_or(|waves| !waves.any_displayed())
        {
            self.draw_welcome_screen(ui, max_width, max_height);
        } else if self.user.tile_tree.is_single_waveform() {
            self.draw_waveform_tile(&ctx, ui, &mut msgs);
        } else {
            CentralPanel::default()
                .frame(Frame::NONE)
                .show_inside(ui, |ui| {
                    self.draw_tiles(&ctx, &mut msgs, ui);
                });
        }

        ctx.input(|i| {
            if !i.raw.dropped_files.is_empty() {
                info!("Got dropped file");
                msgs.push(Message::FilesDropped(i.raw.dropped_files.clone()));
            }
        });

        // If some dialogs are open, skip decoding keypresses
        if !self.user.show_url_entry
            && self.user.show_reload_suggestion.is_none()
            && self.user.show_signal_analysis_wizard.is_none()
            && !ui.egui_wants_keyboard_input()
        {
            self.handle_pressed_keys(&ctx, &mut msgs);
        }

        msgs
    }

    fn draw_load_url(&self, ctx: &egui::Context, msgs: &mut Vec<Message>) {
        let mut open = true;
        egui::Window::new("Load URL")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    let url = &mut *self.url.borrow_mut();
                    let response = ui.text_edit_singleline(url);
                    ui.horizontal(|ui| {
                        if ui.button("Load URL").clicked()
                            || (response.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                        {
                            if let Some(callback) = &self.url_callback {
                                msgs.push(callback(url.clone()));
                            }
                            msgs.push(Message::SetUrlEntryVisible(false, None));
                        }
                        if ui.button("Cancel").clicked() {
                            msgs.push(Message::SetUrlEntryVisible(false, None));
                        }
                    });
                });
            });
        if !open {
            msgs.push(Message::SetUrlEntryVisible(false, None));
        }
    }

    pub fn handle_pointer_in_ui(&self, ui: &mut Ui, msgs: &mut Vec<Message>) {
        if ui.ui_contains_pointer() {
            let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
            if scroll_delta.y > 0.0 {
                msgs.push(Message::InvalidateCount);
                msgs.push(Message::VerticalScroll(MoveDir::Up, self.get_count()));
            } else if scroll_delta.y < 0.0 {
                msgs.push(Message::InvalidateCount);
                msgs.push(Message::VerticalScroll(MoveDir::Down, self.get_count()));
            }
        }
    }

    pub fn get_variable_value(
        &self,
        waves: &WaveData,
        displayed_field_ref: &DisplayedFieldRef,
        ucursor: Option<&num::BigUint>,
    ) -> Option<String> {
        let ucursor = ucursor?;

        let DisplayedItem::Variable(displayed_variable) =
            waves.displayed_items.get(&displayed_field_ref.item)?
        else {
            return None;
        };

        let variable = &displayed_variable.variable_ref;
        let meta = waves
            .inner
            .as_waves()
            .unwrap()
            .variable_meta(variable)
            .ok()?;
        let translator = waves.variable_translator_with_meta(
            &displayed_field_ref.without_field(),
            &self.translators,
            &meta,
        );

        let wave_container = waves.inner.as_waves().unwrap();
        let query_result = wave_container
            .query_variable(variable, ucursor)
            .ok()
            .flatten()?;

        let (time, val) = query_result.current?;
        let curr = self.translate_query_result(
            displayed_field_ref,
            displayed_variable,
            translator,
            meta.clone(),
            val,
        );

        // If time doesn't match cursor, i.e., we are not at a transition or the cursor is at zero
        // or we want the next value after the transition, return current
        if time != *ucursor
            || (*ucursor).is_zero()
            || self.transition_value() == TransitionValue::Next
        {
            return curr;
        }

        // Otherwise, we need to check the previous value for transition display
        let prev_query_result = wave_container
            .query_variable(variable, &(ucursor - BigUint::one()))
            .ok()
            .flatten()?;

        let (_, prev_val) = prev_query_result.current?;
        let prev = self.translate_query_result(
            displayed_field_ref,
            displayed_variable,
            translator,
            meta,
            prev_val,
        );

        match self.transition_value() {
            TransitionValue::Previous => Some(format!("←{}", prev.unwrap_or_default())),
            TransitionValue::Both => match (curr, prev) {
                (Some(curr_val), Some(prev_val)) => Some(format!("{prev_val} → {curr_val}")),
                (None, Some(prev_val)) => Some(format!("{prev_val} →")),
                (Some(curr_val), None) => Some(format!("→ {curr_val}")),
                _ => None,
            },
            TransitionValue::Next => curr, // This will never happen due to the earlier check
        }
    }

    fn translate_query_result(
        &self,
        displayed_field_ref: &DisplayedFieldRef,
        displayed_variable: &DisplayedVariable,
        translator: &dyn Translator<VarId, ScopeId, Message>,
        meta: VariableMeta,
        val: VariableValue,
    ) -> Option<String> {
        let translated = translator.translate(&meta, &val).ok()?;
        let fields = translated.format_flat(
            &displayed_variable.format,
            &displayed_variable.field_formats,
            &self.translators,
        );

        let subfield = fields
            .iter()
            .find(|res| res.names == displayed_field_ref.field)?;

        match &subfield.value {
            Some(TranslatedValue { value, .. }) => Some(value.clone()),
            None => Some("-".to_string()),
        }
    }

    pub fn get_variable_name_info(
        &self,
        var: &VariableRef,
        meta: Option<&VariableMeta>,
    ) -> Option<VariableNameInfo> {
        self.variable_name_info_cache
            .borrow_mut()
            .entry(var.clone())
            .or_insert_with(|| {
                meta.as_ref().and_then(|meta| {
                    self.translators
                        .all_translators()
                        .iter()
                        .find_map(|t| t.variable_name_info(meta))
                })
            })
            .clone()
    }

    pub fn draw_background(
        &self,
        drawing_info: &ItemDrawingInfo,
        ctx: &DrawingContext<'_>,
        background_color: Color32,
    ) {
        let row_top = drawing_info.top();
        let row_bottom = drawing_info.bottom();
        let left = (ctx.to_screen)(0.0, 0.0).x;
        let right = (ctx.to_screen)(ctx.cfg.canvas_size.x, 0.0).x;
        let min = Pos2::new(left, row_top);
        let max = Pos2::new(right, row_bottom);
        ctx.painter
            .rect_filled(Rect { min, max }, CornerRadius::ZERO, background_color);
    }

    pub fn get_background_color(
        &self,
        waves: &WaveData,
        vidx: VisibleItemIndex,
        item_count: usize,
    ) -> Color32 {
        if let Some(focused) = waves.focused_item
            && self.highlight_focused()
            && focused == vidx
        {
            return self.user.config.theme.highlight_background;
        }
        waves
            .items_tree
            .get_visible(vidx)
            .and_then(|visible| waves.displayed_items.get(&visible.item_ref))
            .and_then(super::displayed_item::DisplayedItem::background_color)
            .and_then(|color| self.user.config.theme.get_color(color))
            .unwrap_or_else(|| self.get_default_alternating_background_color(item_count))
    }

    fn get_default_alternating_background_color(&self, item_count: usize) -> Color32 {
        // Set background color
        if self.user.config.theme.alt_frequency != 0
            && (item_count / self.user.config.theme.alt_frequency) % 2 == 1
        {
            self.user.config.theme.canvas_colors.alt_background
        } else {
            Color32::TRANSPARENT
        }
    }

    /// Draw the default timeline at the top of the canvas
    pub fn draw_default_timeline(
        &self,
        waves: &WaveData,
        ctx: &DrawingContext,
        viewport_idx: usize,
        _frame_width: f32,
        _cfg: &DrawConfig,
    ) {
        let ticks = self.get_ticks_for_viewport_idx(waves, viewport_idx, ctx.cfg);
        let wave_top_padding = self.user.config.layout.waveforms_gap;

        waves.draw_ticks(
            self.user.config.theme.foreground,
            &ticks,
            ctx,
            wave_top_padding,
            emath::Align2::CENTER_TOP,
        );
    }
}

pub fn draw_true_name(
    true_name: &TrueName,
    layout_job: &mut LayoutJob,
    font: &FontId,
    foreground: Color32,
    char_width: f32,
    allowed_space: f32,
    line_height: f32,
) {
    let char_budget = (allowed_space / char_width) as usize;

    match true_name {
        TrueName::SourceCode {
            line_number,
            before,
            this,
            after,
        } => {
            let before_chars = before.chars().collect::<Vec<_>>();
            let this_chars = this.chars().collect::<Vec<_>>();
            let after_chars = after.chars().collect::<Vec<_>>();
            let line_num = format!("{line_number} ");
            let important_chars = line_num.len() + this_chars.len();
            let required_extra_chars = before_chars.len() + after_chars.len();

            // If everything fits, things are very easy
            let (line_num, before, this, after) =
                if char_budget >= important_chars + required_extra_chars {
                    (line_num, before.clone(), this.clone(), after.clone())
                } else if char_budget > important_chars {
                    // How many extra chars we have available
                    let extra_chars = char_budget - important_chars;

                    let max_from_before = (extra_chars as f32 / 2.).ceil() as usize;
                    let max_from_after = (extra_chars as f32 / 2.).floor() as usize;

                    let (chars_from_before, chars_from_after) =
                        if max_from_before > before_chars.len() {
                            (before_chars.len(), extra_chars - before_chars.len())
                        } else if max_from_after > after_chars.len() {
                            (extra_chars - after_chars.len(), before_chars.len())
                        } else {
                            (max_from_before, max_from_after)
                        };

                    let mut before = before_chars
                        .into_iter()
                        .rev()
                        .take(chars_from_before)
                        .rev()
                        .collect::<Vec<_>>();
                    if !before.is_empty() {
                        before[0] = '…';
                    }
                    let mut after = after_chars
                        .into_iter()
                        .take(chars_from_after)
                        .collect::<Vec<_>>();
                    if !after.is_empty() {
                        let last_elem = after.len() - 1;
                        after[last_elem] = '…';
                    }

                    (
                        line_num,
                        before.into_iter().collect(),
                        this.clone(),
                        after.into_iter().collect(),
                    )
                } else {
                    // If we can't even fit the whole important part,
                    // we'll prefer the line number
                    let from_line_num = line_num.len();
                    let from_this = char_budget.saturating_sub(from_line_num);
                    let this = this
                        .chars()
                        .take(from_this)
                        .enumerate()
                        .map(|(i, c)| if i == from_this - 1 { '…' } else { c })
                        .collect();
                    (line_num, String::new(), this, String::new())
                };

            layout_job.append(
                &line_num,
                0.0,
                TextFormat {
                    font_id: font.clone(),
                    color: foreground.gamma_multiply(0.75),
                    line_height: Some(line_height),
                    ..Default::default()
                },
            );
            layout_job.append(
                &before,
                0.0,
                TextFormat {
                    font_id: font.clone(),
                    color: foreground.gamma_multiply(0.5),
                    line_height: Some(line_height),
                    ..Default::default()
                },
            );
            layout_job.append(
                &this,
                0.0,
                TextFormat {
                    font_id: font.clone(),
                    color: foreground,
                    line_height: Some(line_height),
                    ..Default::default()
                },
            );
            layout_job.append(
                after.trim_end(),
                0.0,
                TextFormat {
                    font_id: font.clone(),
                    color: foreground.gamma_multiply(0.5),
                    line_height: Some(line_height),
                    ..Default::default()
                },
            );
        }
    }
}
