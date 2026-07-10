use egui::{Frame, Layout, Margin, Panel, Ui};
use emath::Align;
use web_time::{Duration, Instant};

use crate::time::{time_string, timeunit_menu};
use crate::wave_source::draw_progress_information;
use crate::{SystemState, displayed_item::DisplayedItem, message::Message, wave_data::WaveData};

/// Debounce duration for progress information display (in milliseconds)
/// Progress is only shown after this duration to avoid flicker on fast operations
const PROGRESS_DEBOUNCE_MS: u64 = 100;

impl SystemState {
    pub(crate) fn add_statusbar_panel(
        &self,
        ui: &mut Ui,
        waves: Option<&WaveData>,
        msgs: &mut Vec<Message>,
    ) {
        Panel::bottom("statusbar")
            .frame(Frame {
                fill: self.user.config.theme.primary_ui_color.background,
                inner_margin: Margin {
                    left: 5,
                    right: 5,
                    top: 0,
                    bottom: 5,
                },
                ..Default::default()
            })
            .show(ui, |ui| {
                self.draw_statusbar(ui, waves, msgs);
            });
    }

    fn draw_statusbar(&self, ui: &mut Ui, waves: Option<&WaveData>, msgs: &mut Vec<Message>) {
        ui.visuals_mut().override_text_color =
            Some(self.user.config.theme.primary_ui_color.foreground);
        ui.with_layout(Layout::left_to_right(Align::RIGHT), |ui| {
            self.draw_statusbar_left(ui, waves, msgs);
            self.draw_statusbar_right(ui, waves, msgs);
        });
    }

    /// Draw left-aligned status bar elements: wave source and generation date
    fn draw_statusbar_left(&self, ui: &mut Ui, waves: Option<&WaveData>, msgs: &mut Vec<Message>) {
        if let Some(waves) = waves {
            ui.label(waves.source.to_string());
            if let Some(idx) = self.user.selected_server_file_index
                && let Some(infos) = self.user.surver_file_infos.as_ref()
                && let Some(file) = infos.get(idx)
            {
                ui.separator();
                ui.label(&file.filename);
            }
            if let Some(datetime) = waves.inner.metadata().date {
                ui.separator();
                ui.label(format!("Generated: {datetime}"));
            }
        }

        if let Some(state_file) = &self.user.state_file {
            ui.separator();
            ui.label(state_file.to_string_lossy());
        }

        if !self.user.dismissed_konata_hint
            && self.user.konata_tiles.is_empty()
            && let Some(waves) = waves
            && let Some(stream) = waves.displayed_items.values().find_map(|item| {
                let DisplayedItem::Stream(stream) = item else {
                    return None;
                };
                let generator = stream.transaction_stream_ref.gen_id?;
                waves
                    .transactions_for_source(stream.source)?
                    .event_index()
                    .events_generator_of(generator)?;
                Some(stream)
            })
        {
            ui.separator();
            if ui
                .small_button("Pipeline trace detected — open in Konata view?")
                .clicked()
            {
                msgs.push(Message::OpenKonataView {
                    source: stream.source,
                    generator: stream.transaction_stream_ref.clone(),
                });
            }
            if ui.small_button("×").on_hover_text("Dismiss").clicked() {
                msgs.push(Message::DismissKonataSuggestion);
            }
        }

        if let Some(progress_data) = &self.progress_tracker
            && Instant::now().duration_since(progress_data.started)
                > Duration::from_millis(PROGRESS_DEBOUNCE_MS)
        {
            ui.separator();
            draw_progress_information(ui, progress_data);
        }

        // Show analog cache building status
        if let Some(waves) = waves {
            let in_progress_count = waves.inflight_caches.len();
            if in_progress_count > 0 {
                ui.separator();
                ui.spinner();
                if in_progress_count == 1 {
                    ui.label("Building analog cache…");
                } else {
                    ui.label(format!("Building {in_progress_count} analog caches…"));
                }
            }
        }
    }

    /// Draw right-aligned status bar elements: cursor time, undo info, count, and RAM usage
    fn draw_statusbar_right(&self, ui: &mut Ui, waves: Option<&WaveData>, msgs: &mut Vec<Message>) {
        ui.with_layout(Layout::right_to_left(Align::RIGHT), |ui| {
            if let Some(usage) = memory_stats::memory_stats() {
                ui.label(format!("RAM: {}", format_byte_size(usage.physical_mem)));
                ui.separator();
            }
            if let Some(waves) = waves {
                if let Some(time) = &waves.num_timestamps() {
                    ui.label(format!(
                        " ({})",
                        time_string(
                            time,
                            &waves.inner.metadata().timescale,
                            &self.user.wanted_timeunit,
                            &self.get_time_format(),
                        )
                    ))
                    .context_menu(|ui| timeunit_menu(ui, msgs, &self.user.wanted_timeunit));
                }
                if let Some(time) = &waves.cursor {
                    ui.label(time_string(
                        time,
                        &waves.inner.metadata().timescale,
                        &self.user.wanted_timeunit,
                        &self.get_time_format(),
                    ))
                    .context_menu(|ui| timeunit_menu(ui, msgs, &self.user.wanted_timeunit));
                }
                if let Some(undo_op) = &self.undo_stack.last() {
                    ui.separator();
                    ui.label(format!("Undo: {}", undo_op.message));
                }
                if let Some(count) = &self.user.count {
                    ui.separator();
                    ui.label(format!("Count: {count}"));
                }
            }
        });
    }
}

fn format_byte_size(bytes: usize) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b < MB {
        format!("{:.1} KB", b / KB)
    } else if b < GB {
        format!("{:.1} MB", b / MB)
    } else {
        format!("{:.2} GB", b / GB)
    }
}
