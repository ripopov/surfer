use crate::{
    config::{FocusHighlight, ThemeColorPair, TransitionValue},
    dialog::{draw_open_sibling_state_file_dialog, draw_reload_waveform_dialog},
    displayed_item::DisplayedVariable,
    fzcmd::expand_command,
    item_drawing_info::{
        DividerDrawingInfo, GroupDrawingInfo, ItemDrawingInfo, MarkerDrawingInfo,
        PlaceholderDrawingInfo, StreamDrawingInfo, TimeLineDrawingInfo, VariableDrawingInfo,
    },
    menus::generic_context_menu,
    tooltips::variable_tooltip_text,
    wave_container::{ScopeId, VarId, VariableMeta},
};
use ahash::{AHashSet, AHasher};
use ecolor::Color32;
#[cfg(not(target_arch = "wasm32"))]
use egui::ViewportCommand;
use egui::{
    CentralPanel, FontSelection, Frame, Layout, Painter, Panel, RichText, Sense, TextStyle, Ui,
    UiBuilder, WidgetText,
};
use emath::{Align, GuiRounding, Pos2, Rect, RectTransform, Vec2};
use epaint::{
    CornerRadius, Shape, Stroke,
    text::{FontId, LayoutJob, TextFormat},
};
use itertools::Itertools;
use num::{BigUint, One, Zero};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use tracing::info;

use surfer_translation_types::{
    TranslatedValue, Translator, VariableInfo, VariableValue,
    translator::{TrueName, VariableNameInfo},
};

use crate::OUTSTANDING_TRANSACTIONS;
#[cfg(feature = "performance_plot")]
use crate::benchmark::NUM_PERF_SAMPLES;
use crate::command_parser::get_parser;
use crate::config::SurferTheme;
use crate::displayed_item::{DisplayedFieldRef, DisplayedItem, DisplayedItemRef};
use crate::displayed_item_tree::{ItemIndex, VisibleItemIndex};
use crate::help::{
    draw_about_window, draw_control_help_window, draw_license_window, draw_quickstart_help_window,
};
use crate::time::time_string;
use crate::translation::TranslationResultExt;
use crate::util::get_alpha_focus_id;
use crate::wave_container::{FieldRef, FieldRefExt, VariableRef};
use crate::{
    Message, SystemState,
    command_prompt::show_command_prompt,
    hierarchy::HierarchyStyle,
    wave_data::{WaveData, WaveformRead},
};

/// Read-only inputs shared by the waveform name and value columns.
#[derive(Clone, Copy)]
pub(crate) struct ItemListView<'a> {
    pub document: &'a WaveData,
    pub items: &'a crate::item_list::ItemList,
    pub focused_item: Option<VisibleItemIndex>,
    pub tile_id: crate::tiles::TileId,
}

impl<'a> From<&'a WaveformRead<'a>> for ItemListView<'a> {
    fn from(waves: &'a WaveformRead<'a>) -> Self {
        Self {
            document: waves.document,
            items: waves.items,
            focused_item: waves.view.focused_index(waves.items),
            tile_id: waves.tile_id,
        }
    }
}

impl ItemListView<'_> {
    fn command(&self, message: crate::tile_kinds::waveform::WaveformMessage) -> Message {
        Message::ToTile(
            self.tile_id,
            crate::tiles::kind::TileMessage::Waveform(message),
        )
    }
    fn selection(&self, selection: crate::item_list::ItemSelection) -> Message {
        self.command(crate::tile_kinds::waveform::WaveformMessage::Selection(
            selection,
        ))
    }
    fn focus(&self, item: Option<DisplayedItemRef>) -> Message {
        self.command(crate::tile_kinds::waveform::WaveformMessage::FocusItem(
            item,
        ))
    }
    fn select_range(&self, to: DisplayedItemRef) -> Message {
        let from = self
            .focused_item
            .and_then(|index| self.items.items_tree.get_visible(index))
            .map_or(to, |node| node.item_ref);
        self.selection(crate::item_list::ItemSelection::Range {
            from,
            to,
            selected: true,
        })
    }
}

impl std::ops::Deref for ItemListView<'_> {
    type Target = WaveData;
    fn deref(&self) -> &Self::Target {
        self.document
    }
}

impl ItemListView<'_> {
    fn item_is_selected(&self, id: DisplayedItemRef) -> bool {
        self.items
            .items_tree
            .iter_visible_selected()
            .any(|node| node.item_ref == id)
    }
}

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

        let viewport_is_moving = self
            .user
            .workspace
            .animate_waveforms(ui.input(|i| i.stable_dt));

        if let Some(waves) = self.user.waves.as_ref().and_then(|w| w.inner.as_waves()) {
            waves.tick();
        }

        #[cfg(not(target_arch = "wasm32"))]
        self.reconcile_native_transactions();

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

/// A single row produced by flattening a (possibly compound/nested) variable's expanded
/// fields; see `WaveformReadServices::flatten_variable_rows`.
pub(crate) struct VariableFieldRow {
    field: FieldRef,
    depth: u32,
    is_compound: bool,
    has_children: bool,
    unfolded: bool,
}

impl SystemState {
    pub(crate) fn draw(&mut self, ui: &mut Ui, window_size: Option<Vec2>) -> Vec<Message> {
        if let Some(crate::wave_container::WaveContainer::Wellen(waves)) =
            self.user.waves.as_ref().and_then(|w| w.inner.as_waves())
        {
            waves.begin_native_frame();
        }
        let max_width = ui.available_size().x;
        let max_height = ui.available_size().y;

        let mut msgs = vec![];
        if crate::logs::take_error_notification() {
            msgs.push(Message::Workspace(
                crate::tiles::commands::WorkspaceCommand::OpenTile {
                    kind: "logs".into(),
                    placement: crate::tiles::layout::Placement::Edge(
                        crate::tiles::layout::Direction::Down,
                    ),
                    focus: false,
                },
            ));
        }

        if self.user.show_about {
            draw_about_window(ui, &mut msgs);
        }

        if self.user.show_license {
            draw_license_window(ui, &mut msgs);
        }

        if self.user.show_keys {
            draw_control_help_window(ui, &mut msgs, &self.user.config.shortcuts);
        }

        if self.user.show_quick_start {
            draw_quickstart_help_window(ui, &mut msgs, &self.user.config.shortcuts);
        }

        if self.user.show_gestures {
            self.mouse_gesture_help(ui, &mut msgs);
        }

        if let Some(dialog) = self.user.show_reload_suggestion {
            draw_reload_waveform_dialog(ui, dialog, &mut msgs);
        }

        if let Some(dialog) = self.user.show_open_sibling_state_file_suggestion {
            draw_open_sibling_state_file_dialog(ui, dialog, &mut msgs);
        }

        if self.user.show_performance {
            #[cfg(feature = "performance_plot")]
            self.draw_performance_graph(ui, &mut msgs);
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
            self.draw_load_url(ui, &mut msgs);
        }

        if self.user.show_server_file_window {
            self.draw_surver_file_window(ui, &mut msgs);
        }

        if self.show_statusbar() {
            self.add_statusbar_panel(ui, self.user.waves.as_ref(), &mut msgs);
        }

        if let Some(waves) = self.user.waveform_read()
            && self.show_overview()
            && !waves.items.items_tree.is_empty()
        {
            self.add_overview_panel(ui, &waves, &mut msgs);
        }

        if self.show_hierarchy() {
            Panel::left("variable select left panel")
                .default_size(300.)
                .size_range(100.0..=max_width)
                .frame(Frame {
                    fill: self.user.config.theme.primary_ui_color.background,
                    ..Default::default()
                })
                .show(ui, |ui| {
                    self.user.sidepanel_width = Some(ui.clip_rect().width());
                    match self.hierarchy_style() {
                        HierarchyStyle::Separate => self.separate(ui, &mut msgs),
                        HierarchyStyle::Tree => self.tree(ui, &mut msgs),
                        HierarchyStyle::Variables => self.variable_list(ui, &mut msgs),
                    }
                });
        }

        if self.command_prompt.visible {
            show_command_prompt(self, ui, window_size, &mut msgs);
            if let Some(new_idx) = self.command_prompt.new_selection {
                self.command_prompt.selected = new_idx;
                self.command_prompt.new_selection = None;
            }
        }

        let show_welcome = self.user.workspace.tiles().is_empty()
            || (self.user.config.layout.hide_single_tab_bar
                && self.user.workspace.tiles().len() == 1
                && self
                    .user
                    .waveform_read()
                    .is_some_and(|waves| !waves.items.any_displayed()));
        if !show_welcome {
            let focus_ids = self.command_prompt.visible
                && expand_command(
                    &self.command_prompt_text.borrow(),
                    get_parser(self, self.command_prompt.target),
                )
                .expanded
                .starts_with("item_focus");
            let mut adapter = self.layout_adapter.take().unwrap_or_else(|| {
                crate::tiles::render::LayoutAdapter::new(ui.id().with("workspace"))
            });
            let pass = CentralPanel::default()
                .frame(Frame::NONE)
                .show(ui, |ui| {
                    adapter.draw(
                        ui,
                        self.user.workspace.layout(),
                        &self.workspace_runtime,
                        &crate::tiles::kind::ApplicationPanes::new(self, focus_ids),
                        self.user.config.layout.hide_single_tab_bar,
                    )
                })
                .inner;
            self.layout_adapter = Some(adapter);
            match pass {
                Ok(pass) => {
                    let _ = self.user.workspace.set_geometry(pass.revision, pass.rects);
                    if let Some(edit) = pass.edit {
                        msgs.push(Message::ApplyLayoutProposal(edit));
                    }
                    for event in pass.events {
                        use crate::tiles::{commands::WorkspaceCommand, render::PaneEvent};
                        msgs.push(match event {
                            PaneEvent::Focus(id) => {
                                Message::Workspace(WorkspaceCommand::FocusTile(id))
                            }
                            PaneEvent::Close(id) => {
                                Message::Workspace(WorkspaceCommand::CloseTile(id))
                            }
                            PaneEvent::Command(command) => command,
                        });
                    }
                }
                Err(error) => tracing::warn!("Unable to render workspace: {error}"),
            }
        }

        if show_welcome {
            CentralPanel::default()
                .frame(Frame::NONE.fill(self.user.config.theme.canvas_colors.background))
                .show(ui, |ui| {
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

        ui.input(|i| {
            i.raw.dropped_files.iter().for_each(|file| {
                info!("Got dropped file");
                msgs.push(Message::FileDropped(file.clone()));
            });
        });

        // If some dialogs are open, skip decoding keypresses
        // if !self.user.show_url_entry && self.user.show_reload_suggestion.is_none() {
        //     self.handle_pressed_keys(ctx, &mut msgs);
        // }

        // If egui want keyboard inputs, skip decoding keypresses
        if !self.user.show_url_entry
            && self.user.show_reload_suggestion.is_none()
            && !ui.egui_wants_keyboard_input()
        {
            self.handle_pressed_keys(ui, &mut msgs);
        }

        if let Some(crate::wave_container::WaveContainer::Wellen(waves)) = self
            .user
            .waves
            .as_mut()
            .and_then(|w| w.inner.as_waves_mut())
        {
            waves.finish_native_frame();
        }
        self.reconcile_native_signals();
        msgs
    }

    fn draw_load_url(&self, ui: &mut Ui, msgs: &mut Vec<Message>) {
        let mut open = true;
        egui::Window::new("Load URL")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .show(ui, |ui| {
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

    /// Recursively unfolds compound fields up to `levels` deep (0 = fold everything), for
    /// `Message::ExpandDrawnItem`.
    pub(crate) fn expand_levels_to_unfolded_fields(
        info: &VariableInfo,
        levels: usize,
    ) -> AHashSet<Vec<String>> {
        let mut out = AHashSet::new();
        Self::collect_unfolded_fields(info, &mut Vec::new(), levels, &mut out);
        out
    }

    fn collect_unfolded_fields(
        info: &VariableInfo,
        field: &mut Vec<String>,
        levels: usize,
        out: &mut AHashSet<Vec<String>>,
    ) {
        if levels == 0 {
            return;
        }
        if let VariableInfo::Compound { subfields } = info {
            out.insert(field.clone());
            for (name, child_info) in subfields {
                field.push(name.clone());
                Self::collect_unfolded_fields(child_info, field, levels - 1, out);
                field.pop();
            }
        }
    }
}

impl crate::tile_kinds::waveform_services::WaveformReadServices<'_> {
    pub(crate) fn draw_item_focus_list(&self, waves: &ItemListView<'_>, ui: &mut Ui) {
        let alignment = self.get_name_alignment();
        ui.with_layout(
            Layout::top_down(alignment).with_cross_justify(false),
            |ui| {
                let start_y = ui.cursor().top();
                let row_layout = if alignment == Align::LEFT {
                    Layout::left_to_right(Align::Center)
                } else {
                    Layout::right_to_left(Align::Center)
                };
                let clip = ui.clip_rect();
                let visible_top = clip.top() - start_y;
                let visible_bottom = clip.bottom() - start_y;
                // drawing_infos accounts for height_scaling_factor
                for drawing_info in waves
                    .items
                    .visible_drawing_infos(visible_top, visible_bottom)
                    .iter()
                {
                    let next_y = ui.cursor().top();
                    // Align with the corresponding row in other panels
                    if next_y < drawing_info.top_at(start_y) {
                        ui.add_space(drawing_info.top_at(start_y) - next_y);
                    }
                    let vidx = drawing_info.vidx();
                    let row_rect = Rect::from_min_max(
                        Pos2::new(ui.max_rect().left(), drawing_info.top_at(start_y)),
                        Pos2::new(ui.max_rect().right(), drawing_info.bottom_at(start_y)),
                    );
                    let mut row_ui =
                        ui.new_child(UiBuilder::new().max_rect(row_rect).layout(row_layout));
                    crate::tile_kinds::waveform_services::WaveformReadServices::enforce_stable_row_widget_expansion(&mut row_ui);
                    row_ui.style_mut().visuals.selection.bg_fill =
                        self.config.theme.accent_warn.background;
                    row_ui.style_mut().visuals.override_text_color =
                        Some(self.config.theme.accent_warn.foreground);
                    let _ = row_ui.selectable_label(true, get_alpha_focus_id(vidx, waves.items));
                }
                crate::tile_kinds::waveform_services::WaveformReadServices::add_padding_for_last_bottom(
                    ui,
                    waves.items.drawing_bottom_at(start_y),
                    self.config.layout.waveforms_line_height,
                );
            },
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

                    let max_from_before = (extra_chars as f32 * 0.5).ceil() as usize;
                    let max_from_after = (extra_chars as f32 * 0.5).floor() as usize;

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
                    valign: Align::Center,
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
                    valign: Align::Center,
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
                    valign: Align::Center,
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
                    valign: Align::Center,
                    ..Default::default()
                },
            );
        }
    }
}

impl crate::tile_kinds::waveform_services::WaveformReadServices<'_> {
    fn finish_item_drop(
        waves: &ItemListView<'_>,
        ui: &Ui,
        msgs: &mut Vec<Message>,
        position: crate::displayed_item_tree::TargetPosition,
    ) {
        use crate::tile_kinds::waveform::WaveformDrag;
        if !ui.input(|input| input.pointer.any_released()) {
            return;
        }
        let Some(payload) = egui::DragAndDrop::payload::<WaveformDrag>(ui.ctx()) else {
            return;
        };
        if !payload.accepts(waves.tile_id) {
            return;
        }
        egui::DragAndDrop::take_payload::<WaveformDrag>(ui.ctx());
        msgs.push(match payload.as_ref() {
            WaveformDrag::Rows { items, .. } => Message::MoveDraggedItems {
                tile_id: waves.tile_id,
                items: items.clone(),
                position,
            },
            WaveformDrag::Variables(variables) => Message::AddDraggedVariables {
                tile_id: waves.tile_id,
                variables: variables.clone(),
                position,
            },
        });
    }
    pub(crate) fn desired_item_row_height(&self, displayed_item: &DisplayedItem) -> f32 {
        let base_row_height =
            self.config.layout.waveforms_line_height + 2.0 * self.config.layout.waveforms_gap;
        match displayed_item {
            DisplayedItem::Variable(_) | DisplayedItem::Placeholder(_) => {
                self.config.layout.waveforms_line_height * displayed_item.height_scaling_factor()
                    + 2.0 * self.config.layout.waveforms_gap
            }
            DisplayedItem::Stream(stream) => {
                self.config.layout.transactions_line_height * stream.rows as f32
            }
            DisplayedItem::Divider(_)
            | DisplayedItem::Marker(_)
            | DisplayedItem::TimeLine(_)
            | DisplayedItem::Group(_) => base_row_height,
        }
    }

    /// Lists the rows a variable (and its expanded subfields, if any) occupies, in
    /// top-to-bottom order. Pure function of the variable's shape and its persisted fold
    /// state (`unfolded_fields`); see `flattened_variable_rows` for the cached wrapper used
    /// during drawing.
    pub(crate) fn flatten_variable_rows(
        unfolded_fields: &AHashSet<Vec<String>>,
        field: &FieldRef,
        info: &VariableInfo,
        depth: u32,
        out: &mut Vec<VariableFieldRow>,
    ) {
        match info {
            VariableInfo::Compound { subfields } => {
                let unfolded = unfolded_fields.contains(&field.field);
                out.push(VariableFieldRow {
                    field: field.clone(),
                    depth,
                    is_compound: true,
                    has_children: !subfields.is_empty(),
                    unfolded,
                });
                if unfolded {
                    for (name, child_info) in subfields {
                        let mut child_field = field.clone();
                        child_field.field.push(name.clone());
                        Self::flatten_variable_rows(
                            unfolded_fields,
                            &child_field,
                            child_info,
                            depth + 1,
                            out,
                        );
                    }
                }
            }
            VariableInfo::Bool
            | VariableInfo::Bits
            | VariableInfo::Clock
            | VariableInfo::String
            | VariableInfo::Event
            | VariableInfo::Real => out.push(VariableFieldRow {
                field: field.clone(),
                depth,
                is_compound: false,
                has_children: false,
                unfolded: false,
            }),
        }
    }

    pub(crate) fn unfolded_fields_signature(unfolded_fields: &AHashSet<Vec<String>>) -> u64 {
        let mut items: Vec<&Vec<String>> = unfolded_fields.iter().collect();
        items.sort();
        let mut hasher = AHasher::default();
        items.hash(&mut hasher);
        hasher.finish()
    }

    /// Cache expanded rows within the owning list. Both translation structure and
    /// fold state matter; the same local item ID may exist in independent lists.
    pub(crate) fn flattened_variable_rows(
        &self,
        items: &crate::item_list::ItemList,
        item_ref: DisplayedItemRef,
        field: &FieldRef,
        info: &VariableInfo,
        unfolded_fields: &AHashSet<Vec<String>>,
    ) -> Arc<Vec<VariableFieldRow>> {
        let mut hasher = AHasher::default();
        Self::unfolded_fields_signature(unfolded_fields).hash(&mut hasher);
        info.hash(&mut hasher);
        field.hash(&mut hasher);
        let signature = hasher.finish();
        if let Some((cached_signature, rows)) = items.flattened_rows_cache.borrow().get(&item_ref)
            && *cached_signature == signature
        {
            return Arc::clone(rows);
        }
        let mut rows = Vec::new();
        Self::flatten_variable_rows(unfolded_fields, field, info, 0, &mut rows);
        let rows = Arc::new(rows);
        items
            .flattened_rows_cache
            .borrow_mut()
            .insert(item_ref, (signature, Arc::clone(&rows)));
        rows
    }

    /// Computes top/bottom for every row (including compound-variable subfields), as if the
    /// first row started at y = 0. Pure function of Surfer state (item tree, displayed
    /// items, fold state) and layout config - no `ui`/egui state involved - so the result
    /// only needs to change when items are added/removed/reordered or a group/compound is
    /// folded/unfolded; see `item_layout_signature` and its use in `draw_item_list`.
    pub(crate) fn compute_item_drawing_infos(
        &self,
        items: &crate::item_list::ItemList,
    ) -> Vec<ItemDrawingInfo> {
        let mut out = Vec::new();
        let mut y = 0.0f32;
        for info in items.items_tree.iter_visible_extra() {
            let item_ref = info.node.item_ref;
            let vidx = info.vidx;
            let Some(displayed_item) = items.displayed_items.get(&item_ref) else {
                continue;
            };
            let row_height = self.desired_item_row_height(displayed_item);
            match displayed_item {
                DisplayedItem::Variable(displayed_variable) => {
                    let field = FieldRef::without_fields(displayed_variable.variable_ref.clone());
                    let rows = self.flattened_variable_rows(
                        items,
                        item_ref,
                        &field,
                        &displayed_variable.info,
                        &displayed_variable.unfolded_fields,
                    );
                    for row in rows.iter() {
                        let top = y;
                        let bottom = y + row_height;
                        out.push(ItemDrawingInfo::Variable(VariableDrawingInfo {
                            displayed_field_ref: DisplayedFieldRef {
                                item: item_ref,
                                field: row.field.field.clone(),
                            },
                            field_ref: row.field.clone(),
                            vidx,
                            top,
                            bottom,
                        }));
                        y = bottom;
                    }
                }
                DisplayedItem::Divider(_) => {
                    out.push(ItemDrawingInfo::Divider(DividerDrawingInfo {
                        vidx,
                        top: y,
                        bottom: y + row_height,
                    }));
                    y += row_height;
                }
                DisplayedItem::Marker(cursor) => {
                    out.push(ItemDrawingInfo::Marker(MarkerDrawingInfo {
                        vidx,
                        top: y,
                        bottom: y + row_height,
                        idx: cursor.idx,
                    }));
                    y += row_height;
                }
                DisplayedItem::TimeLine(_) => {
                    out.push(ItemDrawingInfo::TimeLine(TimeLineDrawingInfo {
                        vidx,
                        top: y,
                        bottom: y + row_height,
                    }));
                    y += row_height;
                }
                DisplayedItem::Stream(stream) => {
                    out.push(ItemDrawingInfo::Stream(StreamDrawingInfo {
                        transaction_stream_ref: stream.transaction_stream_ref.clone(),
                        vidx,
                        top: y,
                        bottom: y + row_height,
                    }));
                    y += row_height;
                }
                DisplayedItem::Group(_) => {
                    out.push(ItemDrawingInfo::Group(GroupDrawingInfo {
                        vidx,
                        top: y,
                        bottom: y + row_height,
                    }));
                    y += row_height;
                }
                DisplayedItem::Placeholder(_) => {
                    out.push(ItemDrawingInfo::Placeholder(PlaceholderDrawingInfo {
                        vidx,
                        top: y,
                        bottom: y + row_height,
                    }));
                    y += row_height;
                }
            }
        }
        out
    }

    /// Cheap structural signature of everything `compute_item_drawing_infos` depends on;
    /// `draw_item_list` only rebuilds the cached layout when this changes.
    pub(crate) fn item_layout_signature(&self, items: &crate::item_list::ItemList) -> u64 {
        let mut hasher = AHasher::default();
        let layout = &self.config.layout;
        layout.waveforms_line_height.to_bits().hash(&mut hasher);
        layout.waveforms_gap.to_bits().hash(&mut hasher);
        layout.transactions_line_height.to_bits().hash(&mut hasher);
        self.translator_generation.hash(&mut hasher);
        for info in items.items_tree.iter_visible_extra() {
            info.node.item_ref.0.hash(&mut hasher);
            info.node.level.hash(&mut hasher);
            info.node.unfolded.hash(&mut hasher);
            let item = items.displayed_items.get(&info.node.item_ref);
            item.map(std::mem::discriminant).hash(&mut hasher);
            match item {
                Some(DisplayedItem::Variable(v)) => {
                    v.info.hash(&mut hasher);
                    v.variable_ref.hash(&mut hasher);
                    v.height_scaling_factor.map(f32::to_bits).hash(&mut hasher);
                    Self::unfolded_fields_signature(&v.unfolded_fields).hash(&mut hasher);
                }
                Some(DisplayedItem::Placeholder(p)) => {
                    p.height_scaling_factor.map(f32::to_bits).hash(&mut hasher);
                }
                Some(DisplayedItem::Stream(s)) => {
                    s.rows.hash(&mut hasher);
                }
                _ => {}
            }
        }
        hasher.finish()
    }

    /// Refresh content-space row positions when the item list or layout settings change.
    /// Linked views reuse this cache regardless of their scroll positions and widths.
    pub(crate) fn ensure_drawing_infos_cached(&self, items: &crate::item_list::ItemList) {
        let signature = self.item_layout_signature(items);
        if items.layout_cache.borrow().signature == Some(signature) {
            return;
        }
        let infos = self.compute_item_drawing_infos(items);
        *items.layout_cache.borrow_mut() = crate::item_list::ItemLayoutCache {
            total_height: infos.last().map_or(0.0, ItemDrawingInfo::bottom),
            infos,
            signature: Some(signature),
        };
    }

    pub fn draw_background(
        &self,
        drawing_info: &ItemDrawingInfo,
        y_offset: f32,
        ctx: &DrawingContext<'_>,
        background_color: Color32,
    ) {
        let row_top = drawing_info.top_at(y_offset);
        let row_bottom = drawing_info.bottom_at(y_offset);
        let left = (ctx.to_screen)(0.0, 0.0).x;
        let right = (ctx.to_screen)(ctx.cfg.canvas_size.x, 0.0).x;
        let min = Pos2::new(left, row_top);
        let max = Pos2::new(right, row_bottom);
        ctx.painter
            .rect_filled(Rect { min, max }, CornerRadius::ZERO, background_color);
    }

    pub fn get_background_color(
        &self,
        items: &crate::item_list::ItemList,
        focused_item: Option<VisibleItemIndex>,
        vidx: VisibleItemIndex,
        item_count: usize,
    ) -> Color32 {
        if let Some(focused) = focused_item
            && matches!(self.focus_highlight, FocusHighlight::Background)
            && focused == vidx
        {
            return self.config.theme.highlight_background;
        }
        items
            .items_tree
            .get_visible(vidx)
            .and_then(|visible| items.displayed_items.get(&visible.item_ref))
            .and_then(super::displayed_item::DisplayedItem::background_color)
            .and_then(|color| self.config.theme.get_color(color))
            .unwrap_or_else(|| self.get_default_alternating_background_color(item_count))
    }

    pub(crate) fn get_default_alternating_background_color(&self, item_count: usize) -> Color32 {
        // Set background color
        if self.config.theme.alt_frequency != 0
            && (item_count / self.config.theme.alt_frequency) % 2 == 1
        {
            self.config.theme.canvas_colors.alt_background
        } else {
            Color32::TRANSPARENT
        }
    }

    /// Draw the default timeline at the top of the canvas
    pub fn draw_default_timeline(
        &self,
        waves: &WaveData,
        ctx: &DrawingContext,
        viewport: &crate::viewport::Viewport,
    ) {
        let ticks = self.get_ticks_for_viewport(waves, viewport, ctx.cfg);
        let wave_top_padding = self.config.layout.waveforms_gap;

        ctx.draw_ticks(
            self.config.theme.foreground,
            &ticks,
            wave_top_padding,
            emath::Align2::CENTER_TOP,
        );
    }
}

impl crate::tile_kinds::waveform_services::WaveformReadServices<'_> {
    /// Add bottom padding so the last item isn’t clipped or covered by the scrollbar.
    /// `last_bottom` must already be in this `ui`'s coordinate space (see
    /// `ItemDrawingInfo::bottom_at`).
    pub(crate) fn add_padding_for_last_bottom(
        ui: &mut Ui,
        last_bottom: Option<f32>,
        line_height: f32,
    ) {
        if let Some(bottom) = last_bottom {
            let target_bottom = bottom + line_height;
            let next_y = ui.cursor().top();
            if next_y < target_bottom {
                ui.add_space(target_bottom - next_y);
            }
        }
    }

    pub(crate) fn item_text_margin(ui: &Ui) -> Vec2 {
        ui.spacing().item_spacing
    }

    pub(crate) fn enforce_stable_row_widget_expansion(ui: &mut Ui) {
        let visuals = &mut ui.style_mut().visuals.widgets;
        visuals.inactive.expansion = 0.0;
        visuals.hovered.expansion = 0.0;
        visuals.active.expansion = 0.0;
        visuals.open.expansion = 0.0;
    }

    pub(crate) fn hierarchy_icon(
        &self,
        ui: &mut Ui,
        has_children: bool,
        unfolded: bool,
        alignment: Align,
    ) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(
            Vec2::splat(self.config.layout.waveforms_text_size),
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
            -std::f32::consts::PI * 0.5
        } else {
            std::f32::consts::PI * 0.5
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

    pub(crate) fn draw_item_list(
        &self,
        waves: &ItemListView<'_>,
        msgs: &mut Vec<Message>,
        ui: &mut Ui,
    ) {
        let any_groups = waves.items.items_tree.iter().any(|node| node.level > 0);
        let alignment = self.get_name_alignment();
        let text_margin = Self::item_text_margin(ui);

        // Refresh the cache purely based on Surfer state (items/fold/drag); ui-derived
        // values like the panel's current position never influence this decision.
        self.ensure_drawing_infos_cached(waves.items);

        ui.with_layout(Layout::top_down(alignment).with_cross_justify(true), |ui| {
            let background_rect = ui.max_rect();
            let painter = ui.painter().clone();

            // Add default horizontal margin for text while keeping the vertical start
            // exactly at `background_rect.min.y`, matching the value column and canvas.
            let rect_with_margin = Rect {
                min: background_rect.min + Vec2::new(text_margin.x, 0.0),
                max: background_rect.max + Vec2::new(0.0, 40.0),
            };

            let builder = UiBuilder::new().max_rect(rect_with_margin);
            ui.scope_builder(builder, |ui| {
                // No item_spacing between rows: gaps come from the explicit wave padding below.
                ui.spacing_mut().item_spacing.y = 0.0;
                let content_rect = ui.available_rect_before_wrap();
                // `waves.items.drawing_infos` is offset-free (canonical); translate the clip rect
                // into that same space instead, so visibility can be checked against the raw
                // cached values, and `start_y` only gets added when a row is actually drawn.
                let start_y = ui.cursor().top();
                let clip = ui.clip_rect();
                let clip_top = clip.top() - start_y;
                let clip_bottom = clip.bottom() - start_y;

                let total_bottom = waves.items.drawing_bottom().unwrap_or(0.0);
                let row_layout = waves.items.layout_cache.borrow();
                let mut row_iter = row_layout.infos.iter().peekable();

                for (item_count, info) in waves.items.items_tree.iter_visible_extra().enumerate() {
                    let item_ref = info.node.item_ref;
                    let vidx = info.vidx;
                    let Some(displayed_item) = waves.items.displayed_items.get(&item_ref) else {
                        continue;
                    };

                    // Pull this item's precomputed rows out of the cache.
                    let mut item_rows: Vec<&ItemDrawingInfo> = Vec::new();
                    while row_iter.peek().is_some_and(|r| r.vidx() == vidx) {
                        item_rows.push(row_iter.next().unwrap());
                    }
                    let Some(&first_row) = item_rows.first() else {
                        continue;
                    };
                    let last_row = item_rows.last().unwrap();

                    let background_color = self.get_background_color(
                        waves.items,
                        waves.focused_item,
                        vidx,
                        item_count,
                    );

                    let is_visible =
                        last_row.bottom() >= clip_top && first_row.top() <= clip_bottom;
                    if !is_visible {
                        // Position is already known from the cache; nothing else to do.
                        continue;
                    }

                    let row_top = first_row.top_at(start_y);
                    let row_bottom = last_row.bottom_at(start_y);

                    let min = Pos2::new(background_rect.left(), row_top);
                    let max = Pos2::new(background_rect.right(), row_bottom);
                    painter.rect_filled(Rect { min, max }, CornerRadius::ZERO, background_color);

                    // Center-align cross-axis so the (smaller) group/fold triangle icon is
                    // vertically centered in the row instead of stuck to its top.
                    let row_layout = if alignment == Align::LEFT {
                        Layout::left_to_right(Align::Center)
                    } else {
                        Layout::right_to_left(Align::Center)
                    };
                    // Content starts at `content_rect.left()` (margin-adjusted), unlike the
                    // background fill above which spans the full un-margined row width.
                    let row_rect = Rect {
                        min: Pos2::new(content_rect.left(), row_top),
                        max,
                    };
                    let mut row_ui =
                        ui.new_child(UiBuilder::new().max_rect(row_rect).layout(row_layout));
                    let row_ui = &mut row_ui;

                    row_ui.add_space(10.0 * f32::from(info.node.level));
                    if any_groups {
                        let response = self.hierarchy_icon(
                            row_ui,
                            info.has_children,
                            info.node.unfolded,
                            alignment,
                        );
                        if response.clicked() {
                            if info.node.unfolded {
                                msgs.push(Message::GroupFold(Some(item_ref)));
                            } else {
                                msgs.push(Message::GroupUnfold(Some(item_ref)));
                            }
                        }
                    }

                    match displayed_item {
                        DisplayedItem::Variable(displayed_variable) => self.draw_variable(
                            waves,
                            msgs,
                            vidx,
                            displayed_item,
                            item_ref,
                            &FieldRef::without_fields(displayed_variable.variable_ref.clone()),
                            &displayed_variable.info,
                            &displayed_variable.unfolded_fields,
                            &item_rows,
                            start_y,
                            row_ui,
                            alignment,
                            background_color,
                        ),
                        DisplayedItem::Divider(_)
                        | DisplayedItem::Marker(_)
                        | DisplayedItem::Placeholder(_)
                        | DisplayedItem::TimeLine(_)
                        | DisplayedItem::Stream(_)
                        | DisplayedItem::Group(_) => {
                            row_ui.with_layout(
                                row_ui
                                    .layout()
                                    .with_main_justify(true)
                                    .with_main_align(alignment),
                                |ui| {
                                    self.draw_plain_item(
                                        waves,
                                        msgs,
                                        vidx,
                                        item_ref,
                                        displayed_item,
                                        ui,
                                        background_color,
                                    );
                                },
                            );
                        }
                    }

                    // expand to the left, but not over the icon size
                    let mut expanded_rect = Rect::from_min_max(
                        Pos2::new(row_rect.min.x, row_top),
                        Pos2::new(row_rect.max.x, first_row.bottom_at(start_y)),
                    );
                    expanded_rect.set_left(
                        content_rect.left()
                            + self.config.layout.waveforms_text_size
                            + text_margin.x,
                    );
                    expanded_rect.set_right(content_rect.right());
                    self.draw_drag_target(
                        waves,
                        msgs,
                        vidx,
                        expanded_rect,
                        content_rect,
                        row_ui,
                        info.last,
                    );
                }

                // Reserve the full content height once, instead of accumulating per-row space.
                let target_bottom =
                    total_bottom + start_y + self.config.layout.waveforms_line_height;
                let next_y = ui.cursor().top();
                if next_y < target_bottom {
                    ui.add_space(target_bottom - next_y);
                }
            });
        });

        // Context menu for the unused part
        let response = ui.allocate_response(ui.available_size(), Sense::click());
        if response.contains_pointer() {
            Self::finish_item_drop(waves, ui, msgs, waves.items.end_insert_position());
        }
        generic_context_menu(msgs, &response);
    }

    pub(crate) fn get_name_alignment(&self) -> Align {
        if self.align_names_right {
            Align::RIGHT
        } else {
            Align::LEFT
        }
    }

    pub(crate) fn draw_drag_source(
        &self,
        waves: &ItemListView<'_>,
        msgs: &mut Vec<Message>,
        vidx: VisibleItemIndex,
        item_response: &egui::Response,
        modifiers: egui::Modifiers,
    ) {
        if item_response.drag_started_by(egui::PointerButton::Primary) {
            let Some(node) = waves.items.items_tree.get_visible(vidx) else {
                return;
            };
            let mut items = if modifiers.ctrl || node.selected {
                waves
                    .items
                    .items_tree
                    .iter_visible_selected()
                    .map(|node| node.item_ref)
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            if !items.contains(&node.item_ref) {
                items.push(node.item_ref);
            }
            egui::DragAndDrop::set_payload(
                &item_response.ctx,
                crate::tile_kinds::waveform::WaveformDrag::Rows {
                    tile_id: waves.tile_id,
                    items,
                },
            );
            if !modifiers.ctrl && !node.selected {
                msgs.push(waves.focus(Some(node.item_ref)));
                msgs.push(waves.selection(crate::item_list::ItemSelection::Clear));
            }
            msgs.push(waves.selection(crate::item_list::ItemSelection::Set {
                item: node.item_ref,
                selected: true,
            }));
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_variable_label(
        &self,
        waves: &ItemListView<'_>,
        vidx: VisibleItemIndex,
        displayed_item: &DisplayedItem,
        displayed_id: DisplayedItemRef,
        field: &FieldRef,
        msgs: &mut Vec<Message>,
        ui: &mut Ui,
        meta: Option<&VariableMeta>,
        background_color: Color32,
    ) -> egui::Response {
        let mut variable_label = self.draw_item_label(
            waves,
            vidx,
            displayed_id,
            displayed_item,
            Some(field),
            msgs,
            ui,
            meta,
            background_color,
        );

        if self.show_tooltip {
            variable_label = variable_label.on_hover_ui(|ui| {
                let tooltip = {
                    if field.field.is_empty() {
                        if meta.is_some() {
                            variable_tooltip_text(meta, &field.root)
                        } else {
                            let wave_container = waves.inner.as_waves().unwrap();
                            let meta = wave_container.variable_meta(&field.root).ok();
                            variable_tooltip_text(meta.as_ref(), &field.root)
                        }
                    } else {
                        "From translator".to_string()
                    }
                };
                ui.set_max_width(ui.spacing().tooltip_width);
                ui.add(egui::Label::new(tooltip));
            });
        }

        variable_label
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_variable(
        &self,
        waves: &ItemListView<'_>,
        msgs: &mut Vec<Message>,
        vidx: VisibleItemIndex,
        displayed_item: &DisplayedItem,
        displayed_id: DisplayedItemRef,
        field: &FieldRef,
        info: &VariableInfo,
        unfolded_fields: &AHashSet<Vec<String>>,
        item_rows: &[&ItemDrawingInfo],
        start_y: f32,
        ui: &mut Ui,
        alignment: Align,
        background_color: Color32,
    ) {
        let wave_top_padding = self.config.layout.waveforms_gap;
        // Row positions are already known (see `compute_item_drawing_infos`); only the
        // per-row shape (depth/is_compound/...) is looked up here, from the same cache.
        let rows =
            self.flattened_variable_rows(waves.items, displayed_id, field, info, unfolded_fields);
        // `item_rows` holds offset-free positions; translate the clip rect into that same
        // space instead, so visibility is checked against the raw cached values.
        let clip = ui.clip_rect();
        let clip_top = clip.top() - start_y;
        let clip_bottom = clip.bottom() - start_y;

        ui.with_layout(Layout::top_down(alignment).with_cross_justify(true), |ui| {
            for (row, position) in rows.iter().zip(item_rows.iter()) {
                // Sub-fields deep inside a large expanded compound can be scrolled out of
                // view; their position is already known, so skip the widget/text layout.
                if !position.overlaps(clip_top, clip_bottom) {
                    continue;
                }

                let row_top = position.top_at(start_y);
                let row_bottom = position.bottom_at(start_y);

                let row_layout = if alignment == Align::LEFT {
                    Layout::left_to_right(Align::Center)
                } else {
                    Layout::right_to_left(Align::Center)
                };
                let row_rect = Rect::from_min_max(
                    Pos2::new(ui.max_rect().left(), row_top),
                    Pos2::new(ui.max_rect().right(), row_bottom),
                );
                let mut row_ui =
                    ui.new_child(UiBuilder::new().max_rect(row_rect).layout(row_layout));
                Self::enforce_stable_row_widget_expansion(&mut row_ui);
                row_ui.add_space(10.0 * row.depth as f32);

                if row.is_compound {
                    let icon_response =
                        self.hierarchy_icon(&mut row_ui, row.has_children, row.unfolded, alignment);
                    if icon_response.clicked() {
                        msgs.push(Message::ToggleVariableFieldFold(
                            displayed_id,
                            row.field.field.clone(),
                        ));
                    }
                }

                let label_response = row_ui
                    .with_layout(Layout::top_down(alignment).with_cross_justify(true), |ui| {
                        ui.add_space(wave_top_padding);
                        self.draw_variable_label(
                            waves,
                            vidx,
                            displayed_item,
                            displayed_id,
                            &row.field,
                            msgs,
                            ui,
                            None,
                            background_color,
                        )
                    })
                    .inner;

                // Compound header rows aren't draggable variables themselves.
                if !row.is_compound {
                    self.draw_drag_source(
                        waves,
                        msgs,
                        vidx,
                        &label_response,
                        ui.input(|e| e.modifiers),
                    );
                }
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_drag_target(
        &self,
        waves: &ItemListView<'_>,
        msgs: &mut Vec<Message>,
        vidx: VisibleItemIndex,
        expanded_rect: Rect,
        content_rect: Rect,
        ui: &mut Ui,
        last: bool,
    ) {
        let Some(payload) =
            egui::DragAndDrop::payload::<crate::tile_kinds::waveform::WaveformDrag>(ui.ctx())
        else {
            return;
        };
        if !payload.accepts(waves.tile_id) {
            return;
        }

        // expanded_rect is just for the label, leaving us with gaps between lines
        // expand to counter that
        let rect_with_margin = expanded_rect.expand2(ui.spacing().item_spacing / 2f32);

        // collision check rect need to be
        // - limited to half the height of the item text
        // - extended to cover the empty space to the left
        // - for the last element, expanded till the bottom
        let before_rect = rect_with_margin
            .with_max_y(rect_with_margin.left_center().y)
            .with_min_x(content_rect.left())
            .round_to_pixels(ui.painter().pixels_per_point());
        let after_rect = if last {
            rect_with_margin.with_max_y(ui.max_rect().max.y)
        } else {
            rect_with_margin
        }
        .with_min_y(rect_with_margin.left_center().y)
        .with_min_x(content_rect.left())
        .round_to_pixels(ui.painter().pixels_per_point());

        let (insert_vidx, line_y) = if ui.rect_contains_pointer(before_rect) {
            (vidx, rect_with_margin.top())
        } else if ui.rect_contains_pointer(after_rect) {
            (VisibleItemIndex(vidx.0 + 1), rect_with_margin.bottom())
        } else {
            return;
        };

        let level_range = waves
            .items
            .items_tree
            .valid_levels_visible(insert_vidx, |node| {
                matches!(
                    waves.items.displayed_items.get(&node.item_ref),
                    Some(DisplayedItem::Group(..))
                )
            });

        let left_x = |level: u8| -> f32 { rect_with_margin.left() + f32::from(level) * 10.0 };
        let Some(insert_level) = level_range.find_or_last(|&level| {
            let mut rect = expanded_rect.with_min_x(left_x(level));
            rect.set_width(10.0);
            if level == 0 {
                rect.set_left(content_rect.left());
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
                self.config.theme.linewidth,
                self.config.theme.drag_hint_color,
            ),
        );
        let position = crate::displayed_item_tree::TargetPosition {
            before: ItemIndex(
                waves
                    .items
                    .items_tree
                    .to_displayed(insert_vidx)
                    .map_or_else(|| waves.items.items_tree.len(), |index| index.0),
            ),
            level: insert_level,
        };
        Self::finish_item_drop(waves, ui, msgs, position);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_item_label(
        &self,
        waves: &ItemListView<'_>,
        vidx: VisibleItemIndex,
        displayed_id: DisplayedItemRef,
        displayed_item: &DisplayedItem,
        field: Option<&FieldRef>,
        msgs: &mut Vec<Message>,
        ui: &mut Ui,
        meta: Option<&VariableMeta>,
        background_color: Color32,
    ) -> egui::Response {
        let color_pair = {
            if waves.focused_item == Some(vidx) {
                &self.config.theme.accent_info
            } else if waves.item_is_selected(displayed_id) {
                &self.config.theme.selected_elements_colors
            } else if matches!(
                displayed_item,
                DisplayedItem::Variable(_) | DisplayedItem::Placeholder(_)
            ) {
                &ThemeColorPair {
                    background: background_color,
                    foreground: self.config.theme.get_best_text_color(background_color),
                }
            } else {
                &ThemeColorPair {
                    background: self.config.theme.primary_ui_color.background,
                    foreground: self.get_item_text_color(displayed_item),
                }
            }
        };
        {
            let style = ui.style_mut();
            style.visuals.selection.bg_fill = color_pair.background;
        }

        let mut layout_job = LayoutJob::default();
        match displayed_item {
            DisplayedItem::Variable(var) if field.is_some() => {
                let field = field.unwrap();
                let base_line_height = self.config.layout.waveforms_line_height;
                layout_job.first_row_min_height =
                    base_line_height * displayed_item.height_scaling_factor();
                if field.field.is_empty() {
                    let name_info = self.get_variable_name_info(&var.variable_ref, meta);

                    if let Some(true_name) = name_info.and_then(|info| info.true_name) {
                        let monospace_font =
                            ui.style().text_styles.get(&TextStyle::Monospace).unwrap();
                        let monospace_width = {
                            ui.fonts_mut(|fonts| {
                                fonts
                                    .layout_no_wrap(
                                        " ".to_string(),
                                        monospace_font.clone(),
                                        Color32::BLACK,
                                    )
                                    .size()
                                    .x
                            })
                        };
                        let available_width = ui.available_width();

                        draw_true_name(
                            &true_name,
                            &mut layout_job,
                            monospace_font,
                            color_pair.foreground,
                            monospace_width,
                            available_width,
                            base_line_height,
                        );
                    } else {
                        displayed_item.add_to_layout_job(
                            color_pair.foreground,
                            ui.style(),
                            &mut layout_job,
                            Some(field),
                            self.config,
                        );
                    }
                } else {
                    RichText::new(field.field.last().unwrap().clone())
                        .color(color_pair.foreground)
                        .line_height(Some(base_line_height))
                        .append_to(
                            &mut layout_job,
                            ui.style(),
                            FontSelection::Default,
                            Align::Center,
                        );
                }
            }
            _ => displayed_item.add_to_layout_job(
                color_pair.foreground,
                ui.style(),
                &mut layout_job,
                field,
                self.config,
            ),
        }

        let item_label = ui
            .scope(|ui| {
                // Keep row geometry stable across interaction states so hover does not
                // change vertical spacing when custom line-height multipliers are used.
                Self::enforce_stable_row_widget_expansion(ui);
                ui.selectable_label(
                    waves.item_is_selected(displayed_id) || (waves.focused_item == Some(vidx)),
                    WidgetText::LayoutJob(layout_job.into()),
                )
                .interact(Sense::drag())
            })
            .inner;

        // click can select and deselect, depending on previous selection state & modifiers
        // with the rules:
        // - a primary click on the single selected item will deselect it (so that there is a
        //   way to deselect and get rid of the selection highlight)
        // - a primary/secondary click otherwise will select just the clicked item
        // - a secondary click on the selection will not change the selection
        // - a click with shift added will select all items between focused and clicked
        // - a click with control added will toggle the selection of the item
        // - shift + control does not have special meaning
        //
        // We do not implement more complex behavior like the selection toggling
        // that the windows explorer had in the past (with combined ctrl+shift)
        if item_label.clicked() || item_label.secondary_clicked() {
            let focused_item = waves.focused_item;
            let is_focused = focused_item == Some(vidx);
            let is_selected = waves.item_is_selected(displayed_id);
            let single_selected = waves.items.items_tree.iter_visible_selected().count() == 1;

            let modifiers = ui.input(|i| i.modifiers);
            tracing::trace!(focused_item=?focused_item, is_focused=?is_focused, is_selected=?is_selected, single_selected=?single_selected, modifiers=?modifiers);

            // allow us to deselect, but only do so if this is the only selected item
            if item_label.clicked() && is_selected && single_selected {
                msgs.push(Message::Batch(vec![
                    waves.selection(crate::item_list::ItemSelection::Clear),
                    waves.focus(None),
                ]));
                return item_label;
            }

            match (item_label.clicked(), modifiers.command, modifiers.shift) {
                (false, false, false) if is_selected => {}
                (_, false, false) => {
                    msgs.push(Message::Batch(vec![
                        waves.selection(crate::item_list::ItemSelection::Clear),
                        waves.selection(crate::item_list::ItemSelection::Set {
                            item: displayed_id,
                            selected: true,
                        }),
                        waves.focus(Some(displayed_id)),
                    ]));
                }
                (_, _, true) => msgs.push(Message::Batch(vec![
                    waves.select_range(displayed_id),
                    waves.focus(Some(displayed_id)),
                ])),
                (_, true, false) => {
                    if !is_selected {
                        msgs.push(Message::Batch(vec![
                            waves.selection(crate::item_list::ItemSelection::Set {
                                item: displayed_id,
                                selected: true,
                            }),
                            waves.focus(Some(displayed_id)),
                        ]));
                    } else if item_label.clicked() {
                        msgs.push(Message::Batch(vec![
                            waves.selection(crate::item_list::ItemSelection::Set {
                                item: displayed_id,
                                selected: false,
                            }),
                            waves.focus(None),
                        ]));
                    }
                }
            }
        }

        item_label.context_menu(|ui| {
            self.item_context_menu(
                waves,
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

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_plain_item(
        &self,
        waves: &ItemListView<'_>,
        msgs: &mut Vec<Message>,
        vidx: VisibleItemIndex,
        displayed_id: DisplayedItemRef,
        displayed_item: &DisplayedItem,
        ui: &mut Ui,
        background_color: Color32,
    ) {
        let wave_top_padding = self.config.layout.waveforms_gap;
        let row = ui.allocate_ui_with_layout(
            ui.available_size(),
            Layout::top_down(self.get_name_alignment()).with_cross_justify(true),
            |ui| {
                ui.add_space(wave_top_padding);
                self.draw_item_label(
                    waves,
                    vidx,
                    displayed_id,
                    displayed_item,
                    None,
                    msgs,
                    ui,
                    None,
                    background_color,
                )
            },
        );

        self.draw_drag_source(waves, msgs, vidx, &row.inner, ui.input(|e| e.modifiers));
    }

    pub(crate) fn draw_var_values(
        &self,
        waves: &ItemListView<'_>,
        ui: &mut Ui,
        msgs: &mut Vec<Message>,
    ) {
        let response = ui.allocate_response(ui.available_size(), Sense::click());
        generic_context_menu(msgs, &response);

        let mut painter = ui.painter().clone();
        let rect = response.rect;
        let container_rect = Rect::from_min_size(Pos2::ZERO, rect.size());
        let to_screen = RectTransform::from_to(container_rect, rect);
        let cfg = DrawConfig::new(
            rect.size(),
            self.config.layout.waveforms_line_height,
            self.config.layout.waveforms_text_size,
        );

        let ctx = DrawingContext {
            painter: &mut painter,
            cfg: &cfg,
            to_screen: &|x, y| to_screen.transform_pos(Pos2::new(x, y)),
            theme: &self.config.theme,
        };

        let ucursor = waves.cursor.as_ref().and_then(num::BigInt::to_biguint);

        // Add default horizontal margin as it was removed when creating the frame; keep the
        // vertical start exactly at `rect.min.y`, matching the name column and canvas.
        let rect_with_margin = Rect {
            min: rect.min + Vec2::new(ui.spacing().item_spacing.x, 0.0),
            max: rect.max + Vec2::new(0.0, 40.0),
        };

        let builder = UiBuilder::new().max_rect(rect_with_margin);
        ui.scope_builder(builder, |ui| {
            let text_style = TextStyle::Monospace;
            ui.style_mut().override_text_style = Some(text_style);
            ui.spacing_mut().item_spacing.y = 0.0;
            let start_y = ui.cursor().top();
            let clip = ui.clip_rect();
            let clip_top = clip.top() - start_y;
            let clip_bottom = clip.bottom() - start_y;
            // `drawing_infos` is offset-free and already top-to-bottom sorted by
            // construction (see `WaveformReadServices::compute_item_drawing_infos`), so no re-sort
            // is needed here; `start_y` is only added when a row is actually drawn.
            for drawing_info in waves
                .items
                .visible_drawing_infos(clip_top, clip_bottom)
                .iter()
            {
                let next_y = ui.cursor().top();
                // In order to align the text in this view with the variable tree,
                // we need to keep track of how far away from the expected offset we are,
                // and compensate for it
                if next_y < drawing_info.top_at(start_y) {
                    ui.add_space(drawing_info.top_at(start_y) - next_y);
                }

                let backgroundcolor = self.get_background_color(
                    waves.items,
                    waves.focused_item,
                    drawing_info.vidx(),
                    drawing_info.vidx().0,
                );
                self.draw_background(drawing_info, start_y, &ctx, backgroundcolor);
                match drawing_info {
                    ItemDrawingInfo::Variable(variable_info) => {
                        if ucursor.as_ref().is_none() {
                            ui.label("");
                            continue;
                        }

                        let v = self.get_variable_value(
                            waves.document,
                            waves.items,
                            &variable_info.displayed_field_ref,
                            ucursor.as_ref(),
                        );
                        if let Some(v) = v {
                            let waveforms_gap = self.config.layout.waveforms_gap;
                            let waveform_height =
                                (drawing_info.height() - 2.0 * waveforms_gap).max(1.0);

                            ui.add_space(waveforms_gap);
                            // Reserve the full row height on the job but keep the glyph's own
                            // line_height natural, so `Align::Center` valign can center the text.
                            let mut value_layout_job = LayoutJob {
                                first_row_min_height: waveform_height,
                                ..Default::default()
                            };
                            RichText::new(v)
                                .color(self.config.theme.get_best_text_color(backgroundcolor))
                                .line_height(Some(self.config.layout.waveforms_line_height))
                                .append_to(
                                    &mut value_layout_job,
                                    ui.style(),
                                    FontSelection::Default,
                                    Align::Center,
                                );
                            ui.label(WidgetText::LayoutJob(value_layout_job.into()))
                                .context_menu(|ui| {
                                    self.item_context_menu(
                                        waves,
                                        Some(&variable_info.field_ref),
                                        msgs,
                                        ui,
                                        variable_info.vidx,
                                        true,
                                        crate::message::MessageTarget::CurrentSelection,
                                    );
                                });
                        }
                    }

                    ItemDrawingInfo::Marker(numbered_cursor) => {
                        if let Some(cursor) = &waves.cursor {
                            let delta = time_string(
                                &(waves.numbered_marker_time(numbered_cursor.idx) - cursor),
                                &waves.inner.metadata().timescale,
                                &self.wanted_timeunit,
                                &self.time_format,
                            );

                            ui.add_space(self.config.layout.waveforms_gap);
                            ui.label(
                                RichText::new(format!("Δ: {delta}"))
                                    .color(self.config.theme.get_best_text_color(backgroundcolor))
                                    .line_height(Some(
                                        self.config.layout.waveforms_line_height.max(1.0),
                                    )),
                            )
                            .context_menu(|ui| {
                                self.item_context_menu(
                                    waves,
                                    None,
                                    msgs,
                                    ui,
                                    drawing_info.vidx(),
                                    true,
                                    crate::message::MessageTarget::CurrentSelection,
                                );
                            });
                        } else {
                            ui.label("");
                        }
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
            Self::add_padding_for_last_bottom(
                ui,
                waves.items.drawing_bottom_at(start_y),
                self.config.layout.waveforms_line_height + 2.0 * self.config.layout.waveforms_gap,
            );
        });
    }

    pub fn get_variable_value(
        &self,
        waves: &WaveData,
        items: &crate::item_list::ItemList,
        displayed_field_ref: &DisplayedFieldRef,
        ucursor: Option<&num::BigUint>,
    ) -> Option<String> {
        let ucursor = ucursor?;

        let DisplayedItem::Variable(displayed_variable) =
            items.displayed_items.get(&displayed_field_ref.item)?
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
        let translator = items.variable_translator_with_meta(
            &displayed_field_ref.without_field(),
            self.translators,
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
            &meta,
            &val,
        );

        // If time doesn't match cursor, i.e., we are not at a transition or the cursor is at zero
        // or we want the next value after the transition, return current
        if time != *ucursor
            || (*ucursor).is_zero()
            || self.transition_value == TransitionValue::Next
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
            &meta,
            &prev_val,
        );

        match self.transition_value {
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

    pub(crate) fn translate_query_result(
        &self,
        displayed_field_ref: &DisplayedFieldRef,
        displayed_variable: &DisplayedVariable,
        translator: &dyn Translator<VarId, ScopeId, Message>,
        meta: &VariableMeta,
        val: &VariableValue,
    ) -> Option<String> {
        let translated = translator.translate(meta, val).ok()?;
        let fields = translated.format_flat(
            &displayed_variable.format,
            &displayed_variable.field_formats,
            self.translators,
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
}
#[cfg(test)]
mod tile_row_cache_tests {
    use super::*;
    use crate::item_list::ItemList;
    use crate::wave_container::{VariableRef, VariableRefExt};

    #[test]
    fn row_click_keeps_its_list_when_focus_changes_before_dispatch() {
        use crate::tile_kinds::waveform::WaveformMessage;
        use crate::tiles::{commands::WorkspaceCommand, kind::TileMessage, layout::Placement};
        let mut state = SystemState::new_default_config().unwrap();
        let mut ids = Vec::new();
        for name in ["First row", "Second row"] {
            let placement = ids
                .last()
                .copied()
                .map_or(Placement::Root, Placement::TabAfter);
            state
                .update(Message::Workspace(WorkspaceCommand::CreateTile {
                    kind: "waveform".into(),
                    placement,
                    focus: true,
                }))
                .unwrap();
            let id = state.user.workspace.layout().focused().unwrap();
            let position = state
                .user
                .workspace
                .waveform_resources(id)
                .unwrap()
                .0
                .end_insert_position();
            state
                .update(Message::ToTile(
                    id,
                    TileMessage::Waveform(WaveformMessage::AddDivider {
                        name: Some(name.into()),
                        position,
                    }),
                ))
                .unwrap();
            ids.push(id);
        }
        let document = WaveData {
            inner: crate::data_container::DataContainer::Empty,
            source: crate::wave_source::WaveSource::Data,
            format: crate::wave_source::WaveFormat::Vcd,
            active_scope: None,
            cursor: None,
            markers: Default::default(),
            display_variable_indices: false,
            old_max_timestamp: None,
            cache_generation: 0,
            inflight_caches: Default::default(),
            cached_time_range: Default::default(),
        };
        let ctx = egui::Context::default();
        let frame = |events: Vec<egui::Event>, state: &SystemState| {
            let mut messages = Vec::new();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 250.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    for (index, id) in ids.iter().enumerate() {
                        let (items, view) = state.user.workspace.waveform_resources(*id).unwrap();
                        let rect = Rect::from_min_size(
                            Pos2::new(index as f32 * 300.0, 0.0),
                            Vec2::new(280.0, 200.0),
                        );
                        let mut pane = ui.new_child(UiBuilder::new().id_salt(id).max_rect(rect));
                        pane.set_clip_rect(rect);
                        state.waveform_services().draw_item_list(
                            &ItemListView {
                                document: &document,
                                items,
                                focused_item: view.focused_index(items),
                                tile_id: *id,
                            },
                            &mut messages,
                            &mut pane,
                        );
                    }
                },
            );
            output.textures_delta.clear();
            (messages, output)
        };
        let (_, output) = frame(Vec::new(), &state);
        fn label_position(shape: &Shape) -> Option<Pos2> {
            match shape {
                Shape::Text(text) if text.galley.text() == "First row" => {
                    Some(text.pos + Vec2::new(10.0, 5.0))
                }
                Shape::Vec(shapes) => shapes.iter().find_map(label_position),
                _ => None,
            }
        }
        let pos = output
            .shapes
            .iter()
            .find_map(|shape| label_position(&shape.shape))
            .unwrap();
        let click = |pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ]
        };
        frame(click(true), &state);
        let (messages, _) = frame(click(false), &state);
        assert!(!messages.is_empty());
        state
            .update(Message::Workspace(WorkspaceCommand::FocusTile(ids[1])))
            .unwrap();
        for message in messages {
            state.update(message);
        }
        let (first_list, first_view) = state.user.workspace.waveform_resources(ids[0]).unwrap();
        let (second_list, second_view) = state.user.workspace.waveform_resources(ids[1]).unwrap();
        let first = first_list.items_tree.iter().next().unwrap();
        let second = second_list.items_tree.iter().next().unwrap();
        assert_eq!(
            first.item_ref, second.item_ref,
            "independent lists deliberately reuse row numbers"
        );
        assert!(first.selected);
        assert_eq!(first_view.focused_item, Some(first.item_ref));
        assert!(!second.selected);
        assert_eq!(second_view.focused_item, None);
        assert_eq!(state.user.workspace.layout().focused(), Some(ids[1]));
    }

    #[test]
    fn name_columns_render_independent_lists_without_a_global_waveform() {
        let state = SystemState::new_default_config().unwrap();
        assert!(state.user.waves.is_none());
        let document = WaveData {
            inner: crate::data_container::DataContainer::Empty,
            source: crate::wave_source::WaveSource::Data,
            format: crate::wave_source::WaveFormat::Vcd,
            active_scope: None,
            cursor: None,
            markers: Default::default(),
            display_variable_indices: false,
            old_max_timestamp: None,
            cache_generation: 0,
            inflight_caches: Default::default(),
            cached_time_range: Default::default(),
        };
        let make_list = |name: &str| {
            let mut items = ItemList::default();
            let id = items.next_displayed_item_ref();
            items.displayed_items.insert(
                id,
                DisplayedItem::Divider(crate::displayed_item::DisplayedDivider {
                    color: None,
                    background_color: None,
                    name: Some(name.into()),
                }),
            );
            items
                .items_tree
                .insert_item(id, items.end_insert_position())
                .unwrap();
            items
        };
        let lists = [make_list("First view"), make_list("Second view")];
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            for (index, items) in lists.iter().enumerate() {
                let rect = Rect::from_min_size(
                    Pos2::new(index as f32 * 300.0, 0.0),
                    Vec2::new(280.0, 200.0),
                );
                let mut pane = ui.new_child(UiBuilder::new().id_salt(index).max_rect(rect));
                pane.set_clip_rect(rect);
                state.waveform_services().draw_item_list(
                    &ItemListView {
                        document: &document,
                        items,
                        focused_item: Some(VisibleItemIndex(0)),
                        tile_id: crate::tiles::TileId(index as u64 + 1),
                    },
                    &mut Vec::new(),
                    &mut pane,
                );
            }
        });
        output.textures_delta.clear();
        fn texts(shape: &Shape, result: &mut Vec<String>) {
            match shape {
                Shape::Text(text) => result.push(text.galley.text().into()),
                Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, result)),
                _ => {}
            }
        }
        let mut rendered = Vec::new();
        for shape in output.shapes {
            texts(&shape.shape, &mut rendered);
        }
        assert!(
            rendered.iter().any(|text| text == "First view"),
            "{rendered:?}"
        );
        assert!(
            rendered.iter().any(|text| text == "Second view"),
            "{rendered:?}"
        );
        for items in &lists {
            assert_eq!(items.layout_cache.borrow().infos.len(), 1);
        }
    }

    #[test]
    fn independent_lists_with_the_same_item_id_keep_separate_row_caches() {
        let state = SystemState::new_default_config().unwrap();
        let first = ItemList::default();
        let second = ItemList::default();
        let field = FieldRef::without_fields(VariableRef::from_hierarchy_string("top.bus"));
        let unfolded = [Vec::new()].into_iter().collect();
        let compound = |name: &str| VariableInfo::Compound {
            subfields: vec![(name.into(), VariableInfo::Bits)],
        };
        let first_info = compound("first");
        let second_info = compound("second");
        let a = state.waveform_services().flattened_variable_rows(
            &first,
            DisplayedItemRef(1),
            &field,
            &first_info,
            &unfolded,
        );
        let b = state.waveform_services().flattened_variable_rows(
            &second,
            DisplayedItemRef(1),
            &field,
            &second_info,
            &unfolded,
        );
        assert_eq!(a[1].field.field, ["first"]);
        assert_eq!(b[1].field.field, ["second"]);
        let a_again = state.waveform_services().flattened_variable_rows(
            &first,
            DisplayedItemRef(1),
            &field,
            &first_info,
            &unfolded,
        );
        assert!(Arc::ptr_eq(&a, &a_again));
        assert!(
            first
                .copy_content()
                .flattened_rows_cache
                .borrow()
                .is_empty()
        );
    }

    #[test]
    fn changed_translator_structure_rebuilds_rows_without_a_fold_change() {
        let state = SystemState::new_default_config().unwrap();
        let list = ItemList::default();
        let field = FieldRef::without_fields(VariableRef::from_hierarchy_string("top.bus"));
        let unfolded = [Vec::new()].into_iter().collect();
        let before = state.waveform_services().flattened_variable_rows(
            &list,
            DisplayedItemRef(1),
            &field,
            &VariableInfo::Bits,
            &unfolded,
        );
        let info = VariableInfo::Compound {
            subfields: vec![
                ("a".into(), VariableInfo::Bits),
                ("b".into(), VariableInfo::Bits),
            ],
        };
        let after = state.waveform_services().flattened_variable_rows(
            &list,
            DisplayedItemRef(1),
            &field,
            &info,
            &unfolded,
        );
        assert_eq!(before.len(), 1);
        assert_eq!(after.len(), 3);
        assert!(!Arc::ptr_eq(&before, &after));
    }
}
