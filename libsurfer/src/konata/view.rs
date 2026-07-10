use std::{collections::HashMap, sync::atomic::Ordering};

use egui::{
    Align2, Color32, FontId, PointerButton, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui,
    Vec2,
};
use ftr_parser::types::TransactionId;
use num::{BigInt, ToPrimitive as _};

use crate::time::{time_string, timeunit_menu};
use crate::{
    SystemState, message::Message, source::SourceTransactionRef,
    transaction_container::TransactionRef,
};

use super::{
    FlushState, KonataAlignmentMode, KonataArrowStyle, KonataColorScheme, KonataDependency,
    KonataInstructionClassifier, KonataLaneMode, KonataModel, KonataStage, KonataTileId,
    KonataTileState, RowFlags, StageFlags,
};

const RULER_HEIGHT: f32 = 28.0;
const STATUS_HEIGHT: f32 = 22.0;
const FIND_HEIGHT: f32 = 34.0;
const SPLITTER_WIDTH: f32 = 7.0;
const LABEL_PADDING: f32 = 7.0;
const MINIMAP_WIDTH: f32 = 76.0;
const MAX_DETAILED_STAGES: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq)]
enum ScrollGesture {
    Pan(Vec2),
    ZoomX(f64),
    ZoomBoth(f64),
}

pub fn draw_konata_tile(
    state: &mut SystemState,
    _ctx: &egui::Context,
    ui: &mut Ui,
    msgs: &mut Vec<Message>,
    tile_id: KonataTileId,
    tiles: &mut HashMap<KonataTileId, KonataTileState>,
) {
    let overlay_candidates = tiles
        .iter()
        .filter(|(candidate_id, _)| **candidate_id != tile_id)
        .map(|(candidate_id, candidate)| {
            (
                *candidate_id,
                candidate.spec.clone(),
                candidate.title.clone(),
            )
        })
        .collect::<Vec<_>>();
    let configured_overlay = tiles.get(&tile_id).map(|tile| {
        (
            tile.config.overlay_tile,
            tile.config.overlay.as_ref().cloned(),
        )
    });
    let overlay_view = configured_overlay.and_then(|(target_id, spec)| {
        overlay_candidates
            .iter()
            .find(|(candidate_id, candidate, _)| {
                target_id == Some(*candidate_id)
                    || target_id.is_none() && spec.as_ref() == Some(candidate)
            })
            .and_then(|(candidate_id, _, _)| {
                let model = state
                    .konata_runtime
                    .get(candidate_id)?
                    .entry
                    .as_ref()?
                    .model()?;
                Some((*candidate_id, model, tiles.get(candidate_id)?.clone()))
            })
    });
    let Some(tile) = tiles.get_mut(&tile_id) else {
        ui.centered_and_justified(|ui| ui.label("Konata tile state is unavailable"));
        return;
    };

    let generation = state
        .user
        .waves
        .as_ref()
        .and_then(|waves| waves.cache_generation_for_source(tile.spec.source))
        .unwrap_or_default();
    let runtime = state.konata_runtime.entry(tile_id).or_default();
    let entry_matches = runtime.entry.as_ref().is_some_and(|entry| {
        entry.key.source == tile.spec.source
            && entry.key.parent_generator == tile.spec.generator.gen_id.unwrap_or_default()
            && entry.key.generation == generation
    });
    if !entry_matches && runtime.requested_generation != Some(generation) {
        runtime.requested_generation = Some(generation);
        msgs.push(Message::BuildKonataModel { tile_id });
    }
    let entry = runtime.entry.clone();

    let Some(entry) = entry else {
        ui.centered_and_justified(|ui| {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Building pipeline projection…");
            });
        });
        return;
    };
    if let Some(error) = entry.error() {
        ui.centered_and_justified(|ui| {
            ui.colored_label(ui.visuals().error_fg_color, error.as_ref());
        });
        return;
    }
    let Some(model) = entry.model() else {
        let (progress, phase) = state
            .konata_runtime
            .get(&tile_id)
            .map_or((0.0, "Building pipeline projection…"), |runtime| {
                (runtime.model_progress, runtime.model_phase.as_str())
            });
        ui.centered_and_justified(|ui| {
            ui.vertical_centered(|ui| {
                ui.label(phase);
                ui.add(
                    egui::ProgressBar::new(progress.clamp(0.0, 1.0))
                        .desired_width(280.0)
                        .show_percentage(),
                );
            });
        });
        return;
    };

    advance_viewport_motion(state, ui, tile_id, tile);
    synchronize_external_focus(state, tile_id, tile, &model);

    draw_find_bar(state, ui, msgs, tile_id);

    if entry.is_complete() && model.stage_count() == 0 {
        ui.centered_and_justified(|ui| {
            ui.vertical_centered(|ui| {
                ui.heading("No pipeline stages found");
                ui.label("The paired event generator has no usable parent-linked stages.");
                if ui.button("Open pipeline event table").clicked() {
                    msgs.push(Message::OpenKonataEventTable {
                        tile_id,
                        parent_tx: None,
                    });
                }
                if ui.button("Open instruction table").clicked() {
                    msgs.push(Message::OpenTransactionTable {
                        source: tile.spec.source,
                        generator: tile.spec.generator.clone(),
                    });
                }
                ui.label(format!(
                    "Quality summary: {} orphan, {} multiple-parent, {} unnamed, {} out-of-range events",
                    model.quality.orphans,
                    model.quality.multiple_parents,
                    model.quality.unnamed_stages,
                    model.quality.out_of_range,
                ));
            });
        });
        return;
    }

    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    response.clone().widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Other,
            true,
            format!("Pipeline canvas with {} instructions", model.row_count()),
        )
    });
    let split_x =
        (rect.left() + tile.config.splitter_px).clamp(rect.left() + 100.0, rect.right() - 100.0);
    let splitter = Rect::from_min_max(
        Pos2::new(split_x - SPLITTER_WIDTH * 0.5, rect.top()),
        Pos2::new(
            split_x + SPLITTER_WIDTH * 0.5,
            rect.bottom() - STATUS_HEIGHT,
        ),
    );
    let label_rect = Rect::from_min_max(
        Pos2::new(rect.left(), rect.top() + RULER_HEIGHT),
        Pos2::new(splitter.left(), rect.bottom() - STATUS_HEIGHT),
    );
    let minimap_rect = tile.config.show_minimap.then(|| {
        Rect::from_min_max(
            Pos2::new(rect.right() - MINIMAP_WIDTH, rect.top()),
            Pos2::new(rect.right(), rect.bottom() - STATUS_HEIGHT),
        )
    });
    let canvas_right = minimap_rect.map_or(rect.right(), |minimap| minimap.left() - 3.0);
    let ruler_rect = Rect::from_min_max(
        Pos2::new(splitter.right(), rect.top()),
        Pos2::new(canvas_right, rect.top() + RULER_HEIGHT),
    );
    let canvas_rect = Rect::from_min_max(
        ruler_rect.left_bottom(),
        Pos2::new(canvas_right, rect.bottom() - STATUS_HEIGHT),
    );
    let status_rect = Rect::from_min_max(
        Pos2::new(rect.left(), rect.bottom() - STATUS_HEIGHT),
        rect.right_bottom(),
    );
    state.konata_runtime.entry(tile_id).or_default().canvas_size = canvas_rect.size();

    let splitter_response = ui.interact(
        splitter,
        ui.id().with(("konata_splitter", tile_id.0)),
        Sense::drag(),
    );
    if splitter_response.dragged() {
        tile.config.splitter_px = (tile.config.splitter_px + splitter_response.drag_delta().x)
            .clamp(100.0, rect.width() - 100.0);
    }
    if splitter_response.double_clicked() {
        tile.config.splitter_px = 310.0;
    }
    if splitter_response.hovered() || splitter_response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    let ruler_response = ui.interact(
        ruler_rect,
        ui.id().with(("konata_ruler", tile_id.0)),
        Sense::click_and_drag(),
    );
    ruler_response.context_menu(|ui| {
        if tile.config.clock_period_ticks.is_some() {
            ui.checkbox(&mut tile.config.ruler_cycles, "Show pipeline cycles");
        }
        ui.separator();
        timeunit_menu(ui, msgs, &state.user.wanted_timeunit);
    });

    if let Some(minimap_rect) = minimap_rect {
        let minimap_response = ui.interact(
            minimap_rect,
            ui.id().with(("konata_minimap", tile_id.0)),
            Sense::click_and_drag(),
        );
        interact_minimap(tile, &model, canvas_rect, minimap_rect, &minimap_response);
    }

    #[cfg(feature = "performance_plot")]
    state.timing.borrow_mut().start("Konata command generation");
    interact(
        state,
        ui,
        msgs,
        tile_id,
        tile,
        &model,
        &response,
        overlay_view
            .as_ref()
            .map(|(target_id, overlay_model, _)| (*target_id, overlay_model.as_ref())),
        label_rect,
        ruler_rect,
        canvas_rect,
    );
    register_accessibility(state, ui, tile_id, tile, &model, label_rect, canvas_rect);
    #[cfg(feature = "performance_plot")]
    state.timing.borrow_mut().end("Konata command generation");
    #[cfg(feature = "performance_plot")]
    state.timing.borrow_mut().start("Konata painting");
    paint(
        state,
        ui,
        tile_id,
        tile,
        &model,
        overlay_view
            .as_ref()
            .map(|(target_id, overlay_model, overlay_tile)| {
                (*target_id, overlay_model.as_ref(), overlay_tile)
            }),
        label_rect,
        ruler_rect,
        canvas_rect,
        status_rect,
        splitter,
    );
    if let Some(range) = state
        .konata_runtime
        .get(&tile_id)
        .and_then(|runtime| runtime.range_selection)
    {
        paint_range_selection(ui.painter(), tile, ruler_rect, canvas_rect, range);
    }
    if let Some(minimap_rect) = minimap_rect {
        paint_minimap(ui.painter(), tile, &model, canvas_rect, minimap_rect);
    }
    #[cfg(feature = "performance_plot")]
    state.timing.borrow_mut().end("Konata painting");
    draw_options_header(
        ui,
        msgs,
        tile_id,
        tile,
        &model,
        &overlay_candidates,
        overlay_view
            .as_ref()
            .map(|(target_id, model, tile)| (*target_id, model.as_ref(), tile.title.as_str())),
        Rect::from_min_max(
            rect.left_top(),
            Pos2::new(splitter.left(), label_rect.top()),
        ),
    );
    show_tooltip(ui, tile, &model, &response, label_rect, canvas_rect);
    let range_selection = state
        .konata_runtime
        .get(&tile_id)
        .and_then(|runtime| runtime.range_selection);
    let focused = focused_row(state, tile, &model);
    let producer_chain_active = state
        .konata_runtime
        .get(&tile_id)
        .is_some_and(|runtime| runtime.producer_chain_root == focused);
    let bookmark_labels = (0..10u8)
        .map(|slot| {
            let bookmark = state
                .user
                .konata_bookmarks
                .get(&tile.spec)
                .and_then(|bookmarks| bookmarks.get(slot as usize))
                .copied()
                .flatten();
            let label = bookmark.map_or_else(
                || format!("{slot} — empty"),
                |bookmark| {
                    model.row_for_transaction(bookmark.tx_id).map_or_else(
                        || format!("{slot} — tx#{} @ {}", bookmark.tx_id, bookmark.tick),
                        |row| format!("{slot} — ID {row} @ {}", bookmark.tick),
                    )
                },
            );
            (slot, label, bookmark.is_some())
        })
        .collect::<Vec<_>>();
    show_context_menu(
        msgs,
        tile_id,
        tile,
        &model,
        &response,
        canvas_rect,
        range_selection,
        focused,
        producer_chain_active,
        &bookmark_labels,
    );
    draw_find_result_card(state, ui, msgs, tile_id, tile, &model, canvas_rect);
    draw_pinned_tooltip(state, ui, tile_id, tile, &model, canvas_rect);
    if synchronization_group(tile).is_some() {
        synchronize_tiles(state, tiles, tile_id);
    }
}

fn draw_find_bar(
    state: &mut SystemState,
    ui: &mut Ui,
    msgs: &mut Vec<Message>,
    tile_id: KonataTileId,
) {
    let runtime = state.konata_runtime.entry(tile_id).or_default();
    if !runtime.find_open {
        return;
    }

    let field_id = ui.id().with(("konata_find", tile_id.0));
    let mut submit = false;
    let mut cancel = false;
    let mut close = false;
    let mut next = None;
    let mut to_table = false;
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), FIND_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.label("Find");
            let response = ui.add(
                egui::TextEdit::singleline(&mut runtime.find_query)
                    .id(field_id)
                    .desired_width((ui.available_width() - 390.0).max(120.0))
                    .hint_text("regular expression"),
            );
            if runtime.find_focus_requested {
                response.request_focus();
                runtime.find_focus_requested = false;
            }
            submit = response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if ui.button("Find").clicked() {
                submit = true;
            }
            if ui
                .add_enabled(runtime.find_hits.is_some(), egui::Button::new("Prev"))
                .on_hover_text("Previous match (Shift+F3)")
                .clicked()
            {
                next = Some(true);
            }
            if ui
                .add_enabled(runtime.find_hits.is_some(), egui::Button::new("Next"))
                .on_hover_text("Next match (F3)")
                .clicked()
            {
                next = Some(false);
            }
            if runtime.find_searching {
                cancel = ui.button("Cancel").clicked();
            }
            to_table = ui
                .add_enabled(
                    runtime.find_valid_pattern.as_deref() == Some(runtime.find_query.as_str()),
                    egui::Button::new("to table"),
                )
                .on_hover_text("Open all matches in a pipeline instruction table")
                .clicked();
            close = ui.button("×").on_hover_text("Close find").clicked();
        },
    );

    if runtime.find_searching {
        let fraction = if runtime.find_total == 0 {
            0.0
        } else {
            runtime.find_processed.load(Ordering::Relaxed) as f32 / runtime.find_total as f32
        };
        ui.add(egui::ProgressBar::new(fraction.clamp(0.0, 1.0)).show_percentage());
    }
    if let Some(error) = &runtime.find_error {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }

    let modifiers = ui.input(|input| input.modifiers);
    if ui.input(|input| input.key_pressed(egui::Key::F3)) {
        next = Some(modifiers.shift);
    }
    if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        if runtime.find_searching {
            cancel = true;
        } else if runtime.find_active_row.take().is_none() {
            close = true;
        }
    }

    if submit && !runtime.find_query.is_empty() {
        msgs.push(Message::StartKonataFind {
            tile_id,
            pattern: runtime.find_query.clone(),
        });
    }
    if let Some(reverse) = next {
        msgs.push(Message::KonataFindNext { tile_id, reverse });
    }
    if to_table {
        msgs.push(Message::OpenKonataFindTable {
            tile_id,
            pattern: runtime.find_query.clone(),
        });
    }
    if cancel || close && runtime.find_searching {
        msgs.push(Message::CancelKonataFind { tile_id });
    }
    if close {
        runtime.find_open = false;
        runtime.find_active_row = None;
    }
}

fn advance_viewport_motion(
    state: &mut SystemState,
    ui: &Ui,
    tile_id: KonataTileId,
    tile: &mut KonataTileState,
) {
    let animations_enabled = state.animation_enabled();
    let Some(runtime) = state.konata_runtime.get_mut(&tile_id) else {
        return;
    };
    let Some(mut motion) = runtime.viewport_motion else {
        return;
    };
    if !animations_enabled {
        tile.viewport = motion.target;
        runtime.viewport_motion = None;
        return;
    }
    motion.elapsed += ui.input(|input| input.stable_dt).max(0.0);
    let linear = (motion.elapsed / motion.duration.max(f32::EPSILON)).clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - linear).powi(3);
    tile.viewport =
        super::KonataViewport::interpolate(motion.start, motion.target, f64::from(eased));
    if linear >= 1.0 {
        tile.viewport = motion.target;
        runtime.viewport_motion = None;
    } else {
        runtime.viewport_motion = Some(motion);
        ui.ctx().request_repaint();
    }
}

fn draw_find_result_card(
    state: &mut SystemState,
    ui: &Ui,
    msgs: &mut Vec<Message>,
    tile_id: KonataTileId,
    tile: &KonataTileState,
    model: &KonataModel,
    canvas_rect: Rect,
) {
    let Some(runtime) = state.konata_runtime.get(&tile_id) else {
        return;
    };
    let partial = runtime.find_searching && runtime.find_partial_first.is_some();
    let (row, pattern, header, navigation) = if partial {
        let Some(row) = runtime.find_partial_first else {
            return;
        };
        (
            row,
            runtime.find_query.clone(),
            format!(
                "First match — {} found so far — ID {row}",
                runtime.find_partial_count
            ),
            false,
        )
    } else {
        let Some(row) = runtime.find_active_row else {
            return;
        };
        let Some(hits) = runtime.find_hits.as_ref() else {
            return;
        };
        if !hits.contains(row) {
            return;
        }
        let Some(pattern) = runtime.find_valid_pattern.clone() else {
            return;
        };
        (
            row,
            pattern,
            format!(
                "Match {} of {} — ID {row}",
                hits.ordinal(row).unwrap_or_default(),
                hits.count()
            ),
            true,
        )
    };
    let Ok(regex) = regex::Regex::new(&pattern) else {
        return;
    };
    let mut text = String::new();
    model.write_search_text(row, &mut text);
    let matching_lines = text
        .lines()
        .filter(|line| regex.is_match(line))
        .take(8)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let hidden = tile.config.hide_flushed && model.rows.flushed[row] == FlushState::True;
    let mut close = false;
    let mut next = None;
    let width = canvas_rect.width().clamp(240.0, 460.0);
    egui::Area::new(ui.id().with(("konata_find_result", tile_id.0)))
        .order(egui::Order::Foreground)
        .fixed_pos(Pos2::new(
            canvas_rect.right() - width - 12.0,
            canvas_rect.top() + 12.0,
        ))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.strong(header);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = ui.button("×").clicked();
                        if navigation
                            && ui.button("Next").on_hover_text("Next match (F3)").clicked()
                        {
                            next = Some(false);
                        }
                        if navigation
                            && ui
                                .button("Prev")
                                .on_hover_text("Previous match (Shift+F3)")
                                .clicked()
                        {
                            next = Some(true);
                        }
                    });
                });
                if hidden {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        "This match is hidden because flushed operations are hidden.",
                    );
                }
                for line in matching_lines {
                    ui.label(highlighted_match(&line, &regex, ui));
                }
            });
        });
    if close {
        if partial {
            msgs.push(Message::CancelKonataFind { tile_id });
        } else {
            state
                .konata_runtime
                .entry(tile_id)
                .or_default()
                .find_active_row = None;
        }
    }
    if let Some(reverse) = next {
        msgs.push(Message::KonataFindNext { tile_id, reverse });
    }
}

fn highlighted_match(line: &str, regex: &regex::Regex, ui: &Ui) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let base = egui::TextFormat {
        font_id: FontId::monospace(12.0),
        color: ui.visuals().text_color(),
        ..Default::default()
    };
    let highlight = egui::TextFormat {
        background: Color32::from_rgb(150, 105, 20),
        color: Color32::WHITE,
        ..base.clone()
    };
    let mut end = 0;
    for matched in regex.find_iter(line) {
        job.append(&line[end..matched.start()], 0.0, base.clone());
        job.append(matched.as_str(), 0.0, highlight.clone());
        end = matched.end();
    }
    job.append(&line[end..], 0.0, base);
    job
}

fn draw_pinned_tooltip(
    state: &mut SystemState,
    ui: &Ui,
    tile_id: KonataTileId,
    tile: &KonataTileState,
    model: &KonataModel,
    canvas_rect: Rect,
) {
    let Some((tx_id, event_tx)) = state
        .konata_runtime
        .get(&tile_id)
        .and_then(|runtime| runtime.pinned_tooltip)
    else {
        return;
    };
    let Some(row) = model.row_for_transaction(tx_id) else {
        state
            .konata_runtime
            .entry(tile_id)
            .or_default()
            .pinned_tooltip = None;
        return;
    };
    let text = instruction_tooltip_text(tile, model, row, event_tx);
    let mut close = false;
    egui::Area::new(ui.id().with(("konata_pinned_tooltip", tile_id.0)))
        .order(egui::Order::Foreground)
        .fixed_pos(canvas_rect.left_top() + Vec2::splat(12.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(canvas_rect.width().min(470.0));
                ui.horizontal(|ui| {
                    ui.strong("Pinned instruction");
                    if ui.button("Copy").clicked() {
                        ui.ctx().copy_text(text.clone());
                    }
                    close = ui.button("×").clicked();
                });
                ui.monospace(&text);
            });
        });
    if close {
        state
            .konata_runtime
            .entry(tile_id)
            .or_default()
            .pinned_tooltip = None;
    }
}

fn instruction_tooltip_text(
    tile: &KonataTileState,
    model: &KonataModel,
    row: usize,
    event_tx: Option<u64>,
) -> String {
    use std::fmt::Write as _;

    let mut text = row_label(model, row);
    if let Some(detail) = model.rows.detail(row) {
        write!(text, "\n{}", model.string(detail)).ok();
    }
    write!(text, "\nFTR transaction #{}", model.rows.tx_id[row]).ok();
    if model.rows.flushed[row] == FlushState::True {
        text.push_str("\nThis operation was flushed");
    }
    if model.rows.flags[row].contains(RowFlags::BEGIN_REGRESSION) {
        text.push_str("\nWarning: fetch timestamp moved backward; recorded ID order preserved");
    }
    if model.rows.flags[row].contains(RowFlags::MISSING_RID) {
        text.push_str("\nWarning: operation is unretired or retirement metadata is unknown");
    }
    if model.rows.flags[row].contains(RowFlags::DUPLICATE_RID) {
        text.push_str("\nWarning: duplicate retire ID");
    }
    let stages = model.stages_for_row(row);
    if let Some(stage) =
        event_tx.and_then(|event| stages.iter().find(|stage| stage.event_tx == event))
    {
        let duration = stage.end.saturating_sub(stage.start);
        write!(
            text,
            "\n{} [{}] lane {}",
            model.stage_name(stage),
            tile.config
                .clock_period_ticks
                .filter(|period| *period > 0)
                .map_or_else(
                    || duration.to_string(),
                    |period| format_cycle_duration(duration, period)
                ),
            model.lane_name(stage.lane)
        )
        .ok();
        for annotation in model.annotations_for_stage(stage) {
            write!(text, "\n{}", model.annotation_text(&annotation)).ok();
        }
    } else {
        write!(text, "\n{} stages", stages.len()).ok();
    }
    text
}

fn stage_tooltip_text(
    tile: &KonataTileState,
    model: &KonataModel,
    row: usize,
    tick: f64,
    stages: &[&KonataStage],
) -> String {
    use std::fmt::Write as _;

    let mut text = format!("[{}, ID {row}]", pointer_position(tile, tick));
    for stage in stages {
        let duration = stage.end.saturating_sub(stage.start);
        let duration = tile
            .config
            .clock_period_ticks
            .filter(|period| *period > 0)
            .map_or_else(
                || duration.to_string(),
                |period| format_cycle_duration(duration, period),
            );
        write!(
            text,
            "\n{}[{duration}]  lane {}",
            model.stage_name(stage),
            model.lane_name(stage.lane)
        )
        .ok();
        for annotation in model.annotations_for_stage(stage) {
            write!(text, "\n{}", model.annotation_text(&annotation)).ok();
        }
        if stage.flags.0 != 0 {
            text.push_str("\nRecorded stage has data-quality warnings");
        }
    }
    text
}

#[allow(clippy::too_many_arguments)]
fn draw_options_header(
    ui: &mut Ui,
    msgs: &mut Vec<Message>,
    tile_id: KonataTileId,
    tile: &mut KonataTileState,
    model: &KonataModel,
    overlay_candidates: &[(KonataTileId, super::KonataModelSpec, String)],
    active_overlay: Option<(KonataTileId, &KonataModel, &str)>,
    rect: Rect,
) {
    egui::Area::new(ui.id().with(("konata_options_header", tile_id.0)))
        .order(egui::Order::Foreground)
        .fixed_pos(rect.min)
        .constrain(false)
        .show(ui.ctx(), |ui| {
            ui.set_max_width(rect.width());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Pipeline").strong());
                if let Some((_, overlay_model, title)) = active_overlay {
                    let anchor = tile.viewport.top_visible_row.round().max(0.0) as usize;
                    let anchor = physical_row(tile, model, anchor);
                    let aligned = anchor.and_then(|row| {
                        aligned_row(model, overlay_model, row, tile.config.alignment_mode)
                    });
                    let duplicate_count =
                        model.quality.duplicate_rid + overlay_model.quality.duplicate_rid;
                    let status = if aligned.is_some() {
                        "matched"
                    } else {
                        "unmatched"
                    };
                    let summary = format!(
                        "overlay: {title} · {} · {status} · {duplicate_count} duplicate keys",
                        alignment_label(tile.config.alignment_mode)
                    );
                    ui.add_sized(
                        [(rect.width() - 150.0).max(24.0), 18.0],
                        egui::Label::new(
                            egui::RichText::new(&summary).color(Color32::from_rgb(190, 150, 240)),
                        )
                        .truncate(),
                    )
                    .on_hover_text(summary);
                }
                ui.menu_button("⚙", |ui| {
                    display_options(ui, tile, model);
                    if !overlay_candidates.is_empty() {
                        ui.menu_button("Compare as overlay", |ui| {
                            if ui
                                .selectable_label(tile.config.overlay.is_none(), "None")
                                .clicked()
                            {
                                tile.config.overlay = None;
                                tile.config.overlay_tile = None;
                                ui.close();
                            }
                            for (candidate_id, spec, title) in overlay_candidates {
                                if ui
                                    .selectable_label(
                                        tile.config.overlay_tile == Some(*candidate_id)
                                            || tile.config.overlay_tile.is_none()
                                                && tile.config.overlay.as_ref() == Some(spec),
                                        title,
                                    )
                                    .clicked()
                                {
                                    tile.config.overlay = Some(spec.clone());
                                    tile.config.overlay_tile = Some(*candidate_id);
                                    ui.close();
                                }
                            }
                        });
                    }
                    ui.separator();
                    if ui.button("Pipeline statistics…").clicked() {
                        msgs.push(Message::OpenKonataStatistics { tile_id });
                        ui.close();
                    }
                });
                if model.quality.total() > 0 {
                    ui.menu_button(
                        egui::RichText::new(format!("⚠ {}", model.quality.total()))
                            .color(ui.visuals().warn_fg_color),
                        |ui| {
                            ui.label(format!(
                                "{} timestamp regressions",
                                model.quality.begin_regressions
                            ));
                            ui.label(format!("{} missing retire IDs", model.quality.missing_rid));
                            ui.label(format!(
                                "{} duplicate retire IDs",
                                model.quality.duplicate_rid
                            ));
                            ui.label(format!("{} orphan events", model.quality.orphans));
                            ui.label(format!(
                                "{} multiple-parent events",
                                model.quality.multiple_parents
                            ));
                            ui.label(format!("{} unnamed stages", model.quality.unnamed_stages));
                            ui.label(format!(
                                "{} out-of-range stages",
                                model.quality.out_of_range
                            ));
                            ui.label(format!(
                                "{} end-before-start stages",
                                model.quality.end_before_start
                            ));
                            ui.label(format!("{} unknown lanes", model.quality.unknown_lanes));
                            ui.separator();
                            if ui.button("Open raw event table").clicked() {
                                msgs.push(Message::OpenEventTable {
                                    source: tile.spec.source,
                                    generator: tile.spec.generator.clone(),
                                });
                                ui.close();
                            }
                            if ui.button("Open instruction table").clicked() {
                                msgs.push(Message::OpenTransactionTable {
                                    source: tile.spec.source,
                                    generator: tile.spec.generator.clone(),
                                });
                                ui.close();
                            }
                        },
                    );
                }
            });
        });
}

fn display_options(ui: &mut Ui, tile: &mut KonataTileState, model: &KonataModel) {
    let anchor = tile.viewport.top_visible_row.floor().max(0.0) as usize;
    let anchor_fraction = tile.viewport.top_visible_row - anchor as f64;
    let anchor_row = physical_row(tile, model, anchor);
    ui.add(
        egui::TextEdit::singleline(&mut tile.config.options_filter).hint_text("Search settings…"),
    );
    let filter = tile.config.options_filter.to_ascii_lowercase();
    if option_matches(&filter, &["hide", "flush"])
        && ui
            .checkbox(&mut tile.config.hide_flushed, "Hide flushed ops")
            .changed()
        && let Some(row) = anchor_row
    {
        tile.viewport.top_visible_row = visible_index_of(tile, model, row) as f64 + anchor_fraction;
    }
    if option_matches(&filter, &["color", "scheme", "palette"]) {
        ui.menu_button("Color scheme", |ui| {
            for (scheme, label) in [
                (KonataColorScheme::Auto, "Auto"),
                (KonataColorScheme::Unique, "Unique"),
                (KonataColorScheme::Thread, "Thread ID"),
                (KonataColorScheme::Orange, "Orange"),
                (KonataColorScheme::RoyalBlue, "RoyalBlue"),
                (KonataColorScheme::ColorBlindSafe, "Color-vision-safe"),
                (KonataColorScheme::Custom, "Custom RGB overrides"),
            ] {
                ui.selectable_value(&mut tile.config.color_scheme, scheme, label);
            }
            let names = tile
                .config
                .custom_color_schemes
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            if !names.is_empty() {
                ui.separator();
                for name in names {
                    let selected = tile.config.color_scheme == KonataColorScheme::Custom
                        && tile.config.custom_color_scheme == name;
                    if ui.selectable_label(selected, &name).clicked() {
                        tile.config.color_scheme = KonataColorScheme::Custom;
                        tile.config.custom_color_scheme = name;
                    }
                }
            }
        });
    }
    if option_matches(&filter, &["lane", "split"]) {
        ui.menu_button("Lanes", |ui| {
            ui.selectable_value(&mut tile.config.lane_mode, KonataLaneMode::Merged, "Merged");
            ui.selectable_value(
                &mut tile.config.lane_mode,
                KonataLaneMode::SplitFixed,
                "Split, fixed op height",
            );
            ui.selectable_value(
                &mut tile.config.lane_mode,
                KonataLaneMode::SplitNatural,
                "Split, natural height",
            );
        });
    }
    if option_matches(&filter, &["pipeline", "clock", "cycle", "ruler"]) {
        ui.menu_button("Pipeline clock", |ui| {
            let mut enabled = tile.config.clock_period_ticks.is_some();
            if ui.checkbox(&mut enabled, "Use pipeline clock").changed() {
                tile.config.clock_period_ticks = enabled.then_some(1);
            }
            if let Some(period) = &mut tile.config.clock_period_ticks {
                ui.horizontal(|ui| {
                    ui.label("Period (ticks)");
                    ui.add(egui::DragValue::new(period).range(1..=u64::MAX));
                });
                ui.horizontal(|ui| {
                    ui.label("Origin (tick)");
                    ui.add(egui::DragValue::new(&mut tile.config.clock_origin_tick));
                });
                ui.checkbox(&mut tile.config.ruler_cycles, "Show cycles on ruler");
            } else {
                ui.label("Trace-time mode");
            }
        });
    }
    if option_matches(&filter, &["dependency", "arrow", "wakeup"]) {
        ui.menu_button("Dependency arrows", |ui| {
            ui.selectable_value(
                &mut tile.config.arrow_style,
                KonataArrowStyle::Inside,
                "Inside-line",
            );
            ui.selectable_value(
                &mut tile.config.arrow_style,
                KonataArrowStyle::LeftCurve,
                "Left-side curve",
            );
            ui.selectable_value(
                &mut tile.config.arrow_style,
                KonataArrowStyle::Hidden,
                "Hidden",
            );
        });
    }
    if option_matches(&filter, &["minimap", "overview"]) {
        ui.checkbox(&mut tile.config.show_minimap, "Show minimap");
    }
    if option_matches(&filter, &["synchronize", "sync", "compare"]) {
        if ui
            .checkbox(&mut tile.config.synchronize_scroll, "Synchronize scroll")
            .changed()
        {
            tile.config.sync_group = tile.config.synchronize_scroll.then_some(0);
        }
        ui.menu_button("Comparison alignment", |ui| {
            ui.selectable_value(
                &mut tile.config.alignment_mode,
                KonataAlignmentMode::ThreadRid,
                "Thread + retire ID",
            );
            ui.selectable_value(
                &mut tile.config.alignment_mode,
                KonataAlignmentMode::FetchId,
                "Stable fetch ID",
            );
            ui.selectable_value(
                &mut tile.config.alignment_mode,
                KonataAlignmentMode::Timestamp,
                "Timestamp",
            );
        });
    }
    if option_matches(&filter, &["level", "detail", "lod", "threshold", "zoom"]) {
        ui.collapsing("Level of detail", |ui| {
            ui.add(egui::Slider::new(&mut tile.config.text_lod_px, 4.0..=24.0).text("Text"));
            ui.add(egui::Slider::new(&mut tile.config.frame_lod_px, 1.0..=12.0).text("Borders"));
            ui.add(egui::Slider::new(&mut tile.config.color_lod_px, 0.1..=4.0).text("Colors"));
            ui.add(egui::Slider::new(&mut tile.config.arrow_lod_px, 0.5..=12.0).text("Arrows"));
        });
        ui.add(egui::Slider::new(&mut tile.config.zoom_step, 1.05..=2.0).text("Zoom step"));
    }
    if option_matches(&filter, &["stage", "class", "execute", "stall"]) {
        ui.collapsing("Stage classification", |ui| {
            ui.horizontal(|ui| {
                ui.label("Execute");
                ui.text_edit_singleline(&mut tile.config.execution_stages);
            });
            ui.horizontal(|ui| {
                ui.label("Stalls");
                ui.text_edit_singleline(&mut tile.config.stall_stages);
            });
            ui.checkbox(
                &mut tile.config.stall_case_sensitive,
                "Case-sensitive stalls",
            );
        });
        ui.collapsing("Instruction classification", |ui| {
            ui.selectable_value(
                &mut tile.config.instruction_classifier,
                KonataInstructionClassifier::Generic,
                "Generic",
            );
            ui.selectable_value(
                &mut tile.config.instruction_classifier,
                KonataInstructionClassifier::X86Gem5,
                "x86 / gem5",
            );
            ui.checkbox(
                &mut tile.config.include_estimated_flush_rates,
                "Include estimates in rates",
            );
        });
    }
    if option_matches(&filter, &["custom", "stage", "color"]) {
        ui.collapsing("Custom stage colors", |ui| {
            for (name, color) in &mut tile.config.custom_stage_colors {
                ui.horizontal(|ui| {
                    ui.color_edit_button_srgb(color);
                    ui.label(name);
                });
            }
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut tile.config.custom_color_name);
                ui.color_edit_button_srgb(&mut tile.config.custom_color_rgb);
                if ui.button("Add").clicked() && !tile.config.custom_color_name.trim().is_empty() {
                    tile.config.custom_stage_colors.insert(
                        tile.config.custom_color_name.trim().to_string(),
                        tile.config.custom_color_rgb,
                    );
                    tile.config.custom_color_name.clear();
                }
            });
        });
    }
    if option_matches(&filter, &["cache", "memory", "page", "telemetry"]) {
        let telemetry = model.detail_cache_telemetry();
        ui.collapsing("Detail cache", |ui| {
            ui.monospace(format!(
                "{} / {} MiB decoded · {} MiB encoded",
                telemetry.resident_bytes / (1024 * 1024),
                telemetry.budget_bytes / (1024 * 1024),
                telemetry.encoded_bytes / (1024 * 1024),
            ));
            ui.monospace(format!(
                "{} / {} resident pages · {} pending",
                telemetry.resident_pages, telemetry.page_count, telemetry.pending_pages,
            ));
            ui.monospace(format!(
                "{} hits · {} misses · {} evictions · {} errors",
                telemetry.hits, telemetry.misses, telemetry.evictions, telemetry.decode_errors,
            ));
        });
    }
}

fn option_matches(filter: &str, keywords: &[&str]) -> bool {
    filter.is_empty()
        || keywords
            .iter()
            .any(|keyword| keyword.contains(filter) || filter.contains(keyword))
}

#[allow(clippy::too_many_arguments)]
fn interact(
    state: &mut SystemState,
    ui: &Ui,
    msgs: &mut Vec<Message>,
    tile_id: KonataTileId,
    tile: &mut KonataTileState,
    model: &KonataModel,
    response: &Response,
    overlay: Option<(KonataTileId, &KonataModel)>,
    label_rect: Rect,
    ruler_rect: Rect,
    canvas_rect: Rect,
) {
    let pointer = ui.input(|input| input.pointer.hover_pos());
    let canvas_hovered = pointer.is_some_and(|pointer| canvas_rect.contains(pointer));
    let label_hovered = pointer.is_some_and(|pointer| label_rect.contains(pointer));
    let ruler_hovered = pointer.is_some_and(|pointer| ruler_rect.contains(pointer));
    if response.clicked() {
        response.request_focus();
        state.active_konata_tile = Some(tile_id);
    }

    if canvas_hovered || label_hovered {
        let (scroll, zoom, modifiers) = ui.input(|input| {
            (
                input.smooth_scroll_delta,
                input.zoom_delta(),
                input.modifiers,
            )
        });
        let scroll_zoom_speed = ui
            .ctx()
            .options(|options| options.input_options.scroll_zoom_speed);
        if let Some(gesture) = resolve_scroll_gesture(scroll, zoom, modifiers, scroll_zoom_speed) {
            match gesture {
                ScrollGesture::ZoomX(factor) => {
                    let pointer = pointer.unwrap_or(canvas_rect.center());
                    tile.viewport
                        .zoom_x_at(factor, f64::from(pointer.x - canvas_rect.left()));
                }
                ScrollGesture::ZoomBoth(factor) => {
                    let pointer = pointer.unwrap_or(canvas_rect.center());
                    zoom_at(
                        tile,
                        model,
                        factor,
                        f64::from(pointer.x - canvas_rect.left()),
                        f64::from(pointer.y - canvas_rect.top()),
                    );
                }
                ScrollGesture::Pan(scroll) => {
                    if scroll.x != 0.0 {
                        tile.viewport.pan_pixels(f64::from(scroll.x), 0.0);
                    }
                    if scroll.y != 0.0 {
                        let rows = -f64::from(scroll.y) / row_pitch(tile, model);
                        scroll_rows_with_diagonal(tile, model, rows, !modifiers.shift);
                    }
                }
            }
        }
    }

    let current_focused_tx = focused_transaction_id(state, tile);
    let runtime = state.konata_runtime.entry(tile_id).or_default();
    let primary_down = ui.input(|input| input.pointer.primary_down());
    let direct_navigation = ui.input(|input| {
        input.smooth_scroll_delta != Vec2::ZERO
            || input.zoom_delta() != 1.0
            || [
                egui::Key::ArrowUp,
                egui::Key::ArrowDown,
                egui::Key::ArrowLeft,
                egui::Key::ArrowRight,
                egui::Key::PageUp,
                egui::Key::PageDown,
                egui::Key::Home,
                egui::Key::End,
                egui::Key::Plus,
                egui::Key::Minus,
            ]
            .into_iter()
            .any(|key| input.key_pressed(key))
    });
    if primary_down || direct_navigation {
        runtime.viewport_motion = None;
    }
    runtime.overlay_emphasis = if primary_down && canvas_hovered {
        overlay.and_then(|(target_id, overlay_model)| {
            pointer.map(|pointer| {
                if overlay_hit_is_closer(tile, model, overlay_model, pointer, canvas_rect) {
                    target_id
                } else {
                    tile_id
                }
            })
        })
    } else {
        None
    };
    if response.drag_started_by(PointerButton::Primary)
        && let Some(pointer) = response.interact_pointer_pos()
        && ruler_rect.contains(pointer)
    {
        let tick = tile
            .viewport
            .x_to_tick(pointer.x, canvas_rect.left())
            .max(0.0)
            .round() as u64;
        runtime.range_drag_start = Some(tick);
        runtime.range_selection = Some((tick, tick));
    }
    if let Some(start) = runtime.range_drag_start
        && response.dragged_by(PointerButton::Primary)
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let tick = tile
            .viewport
            .x_to_tick(pointer.x, canvas_rect.left())
            .max(0.0)
            .round() as u64;
        runtime.range_selection = Some((start.min(tick), start.max(tick)));
    } else if response.dragged_by(PointerButton::Primary) {
        let pitch = row_pitch(tile, model);
        pan_viewport_by_drag_delta(&mut tile.viewport, response.drag_delta(), pitch);
    } else if response.drag_stopped_by(PointerButton::Primary) {
        runtime.range_drag_start = None;
    }

    if response.double_clicked_by(PointerButton::Primary)
        && let Some(pointer) = response.interact_pointer_pos()
        && canvas_rect.contains(pointer)
    {
        let shift = ui.input(|input| input.modifiers.shift);
        zoom_at(
            tile,
            model,
            if shift {
                1.0 / tile.config.zoom_step
            } else {
                tile.config.zoom_step
            },
            f64::from(pointer.x - canvas_rect.left()),
            f64::from(pointer.y - canvas_rect.top()),
        );
    }

    if response.clicked_by(PointerButton::Primary)
        && let Some(pointer) = response.interact_pointer_pos()
    {
        if ruler_hovered {
            let tick = tile
                .viewport
                .x_to_tick(pointer.x, canvas_rect.left())
                .max(0.0)
                .round() as u64;
            msgs.push(Message::CursorSet(BigInt::from(tick)));
        } else if canvas_rect.contains(pointer)
            && effective_detail(tile) < f64::from(tile.config.color_lod_px)
        {
            zoom_density_pixel(tile, model, pointer, canvas_rect);
        } else if let Some(row) = row_at_pointer(tile, model, pointer, canvas_rect) {
            let (tx_id, stage_event) = if canvas_rect.contains(pointer) {
                let tick = tile.viewport.x_to_tick(pointer.x, canvas_rect.left());
                let stages = stages_at_tick(tile, model, row, tick).collect::<Vec<_>>();
                if stages.is_empty() {
                    (model.rows.tx_id[row], None)
                } else {
                    let key = (row, pointer.x.round() as i64);
                    if runtime.overlap_click_key == Some(key) {
                        runtime.overlap_click_index =
                            (runtime.overlap_click_index + 1) % stages.len();
                    } else {
                        runtime.overlap_click_key = Some(key);
                        runtime.overlap_click_index = 0;
                    }
                    let event = stages[runtime.overlap_click_index].event_tx;
                    (event, Some(event))
                }
            } else {
                (model.rows.tx_id[row], None)
            };
            runtime.keyboard_row = Some(row);
            runtime.keyboard_stage_event = stage_event;
            runtime.suppress_focus_scroll = Some(tx_id);
            focus_transaction(msgs, tile, tx_id);
            if label_rect.contains(pointer) {
                let visible = visible_index_of(tile, model, row);
                tile.viewport.align_row(model.rows.begin[row], visible);
            }
        } else if canvas_rect.contains(pointer) {
            msgs.push(Message::FocusTransactionFromSource(None, None));
        }
    }

    if response.has_focus() || runtime.keyboard_region_focused {
        let modifiers = ui.input(|input| input.modifiers);
        if ui.input(|input| input.key_pressed(egui::Key::F))
            && (!modifiers.any() || modifiers.command)
        {
            runtime.find_open = true;
            runtime.find_focus_requested = true;
        }
        if ui.input(|input| input.key_pressed(egui::Key::F3)) {
            msgs.push(Message::KonataFindNext {
                tile_id,
                reverse: modifiers.shift,
            });
        }
        if !modifiers.ctrl && ui.input(|input| input.key_pressed(egui::Key::ArrowDown)) {
            scroll_rows_with_diagonal(tile, model, 1.0, !modifiers.shift);
            runtime.keyboard_row = advance_keyboard_row(tile, model, runtime.keyboard_row, 1);
            runtime.keyboard_stage_event = None;
        }
        if !modifiers.ctrl && ui.input(|input| input.key_pressed(egui::Key::ArrowUp)) {
            scroll_rows_with_diagonal(tile, model, -1.0, !modifiers.shift);
            runtime.keyboard_row = advance_keyboard_row(tile, model, runtime.keyboard_row, -1);
            runtime.keyboard_stage_event = None;
        }
        if !modifiers.alt && ui.input(|input| input.key_pressed(egui::Key::ArrowLeft)) {
            tile.viewport.pan_pixels(64.0, 0.0);
        }
        if !modifiers.alt && ui.input(|input| input.key_pressed(egui::Key::ArrowRight)) {
            tile.viewport.pan_pixels(-64.0, 0.0);
        }
        if modifiers.alt
            && ui.input(|input| input.key_pressed(egui::Key::ArrowLeft))
            && let Some(row) = current_focused_tx.and_then(|tx| {
                model
                    .row_for_transaction(tx)
                    .or_else(|| model.row_for_event(tx))
            })
            && let Some(producer) = dependency_walk_target(tile, model, runtime, row, false)
        {
            focus_transaction(msgs, tile, model.rows.tx_id[producer]);
        }
        if modifiers.alt
            && ui.input(|input| input.key_pressed(egui::Key::ArrowRight))
            && let Some(row) = current_focused_tx.and_then(|tx| {
                model
                    .row_for_transaction(tx)
                    .or_else(|| model.row_for_event(tx))
            })
            && let Some(consumer) = dependency_walk_target(tile, model, runtime, row, true)
        {
            focus_transaction(msgs, tile, model.rows.tx_id[consumer]);
        }
        if ui.input(|input| input.key_pressed(egui::Key::Home)) {
            let row = physical_row(tile, model, 0).unwrap_or(0);
            tile.viewport.align_row(model.rows.begin[row], 0);
        }
        if ui.input(|input| input.key_pressed(egui::Key::End)) {
            let count = visible_row_count(tile, model);
            if count > 0 {
                let visible = count - 1;
                let row = physical_row(tile, model, visible).unwrap_or(visible);
                tile.viewport.align_row(model.rows.begin[row], visible);
            }
        }
        if ui.input(|input| input.key_pressed(egui::Key::PageDown)) {
            let rows = (f64::from(canvas_rect.height()) / row_pitch(tile, model) - 1.0).max(1.0);
            scroll_rows_with_diagonal(tile, model, rows, !modifiers.shift);
            runtime.keyboard_row = advance_keyboard_row(
                tile,
                model,
                runtime.keyboard_row,
                rows.floor().min(isize::MAX as f64) as isize,
            );
            runtime.keyboard_stage_event = None;
        }
        if ui.input(|input| input.key_pressed(egui::Key::PageUp)) {
            let rows = (f64::from(canvas_rect.height()) / row_pitch(tile, model) - 1.0).max(1.0);
            scroll_rows_with_diagonal(tile, model, -rows, !modifiers.shift);
            runtime.keyboard_row = advance_keyboard_row(
                tile,
                model,
                runtime.keyboard_row,
                -(rows.floor().min(isize::MAX as f64) as isize),
            );
            runtime.keyboard_stage_event = None;
        }
        if ui.input(|input| input.key_pressed(egui::Key::Plus))
            || modifiers.ctrl && ui.input(|input| input.key_pressed(egui::Key::ArrowUp))
        {
            zoom_at(
                tile,
                model,
                tile.config.zoom_step,
                f64::from(canvas_rect.width()) * 0.5,
                f64::from(canvas_rect.height()) * 0.5,
            );
        }
        if ui.input(|input| input.key_pressed(egui::Key::Minus))
            || modifiers.ctrl && ui.input(|input| input.key_pressed(egui::Key::ArrowDown))
        {
            zoom_at(
                tile,
                model,
                1.0 / tile.config.zoom_step,
                f64::from(canvas_rect.width()) * 0.5,
                f64::from(canvas_rect.height()) * 0.5,
            );
        }
        if ui.input(|input| input.key_pressed(egui::Key::Space))
            && !modifiers.shift
            && let Some(row) =
                keyboard_or_center_row(tile, model, runtime.keyboard_row, canvas_rect.height())
        {
            let tx_id = model.rows.tx_id[row];
            runtime.keyboard_row = Some(row);
            runtime.suppress_focus_scroll = Some(tx_id);
            if current_focused_tx == Some(tx_id) {
                msgs.push(Message::FocusTransactionFromSource(None, None));
            } else {
                focus_transaction(msgs, tile, tx_id);
            }
        }
        if ui.input(|input| input.key_pressed(egui::Key::Space))
            && modifiers.shift
            && let Some(row) =
                keyboard_or_center_row(tile, model, runtime.keyboard_row, canvas_rect.height())
        {
            runtime.keyboard_row = Some(row);
            let pinned = (model.rows.tx_id[row], runtime.keyboard_stage_event);
            runtime.pinned_tooltip = (runtime.pinned_tooltip != Some(pinned)).then_some(pinned);
        }
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            if runtime.find_searching {
                msgs.push(Message::CancelKonataFind { tile_id });
            } else if runtime.find_active_row.take().is_none() {
                if runtime.pinned_tooltip.is_some() {
                    runtime.pinned_tooltip = None;
                } else if runtime.find_open {
                    runtime.find_open = false;
                } else {
                    msgs.push(Message::FocusTransactionFromSource(None, None));
                }
            }
        }
        if ui.input(|input| input.key_pressed(egui::Key::Enter))
            && runtime.find_active_row.is_some()
        {
            runtime.find_active_row = None;
        }
        if ui.input(|input| input.key_pressed(egui::Key::N)) {
            tile.config.lane_mode = match tile.config.lane_mode {
                KonataLaneMode::Merged => KonataLaneMode::SplitFixed,
                KonataLaneMode::SplitFixed | KonataLaneMode::SplitNatural => KonataLaneMode::Merged,
            };
        }
        for (key, slot) in [
            (egui::Key::Num0, 0),
            (egui::Key::Num1, 1),
            (egui::Key::Num2, 2),
            (egui::Key::Num3, 3),
            (egui::Key::Num4, 4),
            (egui::Key::Num5, 5),
            (egui::Key::Num6, 6),
            (egui::Key::Num7, 7),
            (egui::Key::Num8, 8),
            (egui::Key::Num9, 9),
        ] {
            if ui.input(|input| input.key_pressed(key)) {
                msgs.push(if modifiers.ctrl {
                    Message::KonataBookmarkSet(slot)
                } else {
                    Message::KonataBookmarkGoto(slot)
                });
            }
        }
    }
}

fn resolve_scroll_gesture(
    scroll: Vec2,
    zoom: f32,
    modifiers: egui::Modifiers,
    scroll_zoom_speed: f32,
) -> Option<ScrollGesture> {
    if modifiers.alt && scroll != Vec2::ZERO {
        let factor = (f64::from(scroll.x + scroll.y) * f64::from(scroll_zoom_speed)).exp();
        Some(ScrollGesture::ZoomBoth(factor))
    } else if zoom != 1.0 {
        let factor = f64::from(zoom);
        Some(if modifiers.command {
            ScrollGesture::ZoomX(factor)
        } else {
            ScrollGesture::ZoomBoth(factor)
        })
    } else if scroll != Vec2::ZERO {
        Some(ScrollGesture::Pan(scroll))
    } else {
        None
    }
}

fn pan_viewport_by_drag_delta(viewport: &mut super::KonataViewport, delta: Vec2, row_pitch: f64) {
    viewport.pan_pixels(f64::from(delta.x), 0.0);
    viewport.top_visible_row -= f64::from(delta.y) / row_pitch;
}

fn scroll_rows_with_diagonal(
    tile: &mut KonataTileState,
    model: &KonataModel,
    rows: f64,
    compensate: bool,
) {
    let count = visible_row_count(tile, model);
    if count == 0 {
        return;
    }
    let from_visible = tile
        .viewport
        .top_visible_row
        .round()
        .clamp(0.0, (count - 1) as f64) as usize;
    let to_visible = (from_visible as f64 + rows)
        .round()
        .clamp(0.0, (count - 1) as f64) as usize;
    let horizontal = if compensate {
        physical_row(tile, model, from_visible)
            .zip(physical_row(tile, model, to_visible))
            .map_or(0.0, |(from, to)| {
                model.rows.begin[to] as f64 - model.rows.begin[from] as f64
            })
    } else {
        0.0
    };
    tile.viewport.scroll_rows(rows, horizontal);
    tile.viewport.top_visible_row = tile
        .viewport
        .top_visible_row
        .clamp(-2.0, count.saturating_sub(1) as f64 + 2.0);
}

fn dependency_walk_target(
    tile: &KonataTileState,
    model: &KonataModel,
    runtime: &mut super::KonataRuntimeState,
    focused_row: usize,
    forward: bool,
) -> Option<usize> {
    let continuing_tie = runtime.dependency_walk_forward == forward
        && runtime.dependency_walk_target == Some(focused_row)
        && runtime.dependency_walk_origin.is_some();
    let origin = if continuing_tie {
        runtime.dependency_walk_origin?
    } else {
        focused_row
    };
    let mut candidates = if forward {
        model
            .outgoing_dependencies(origin)
            .map(|dependency| {
                let row = dependency.consumer_row as usize;
                let tick = dependency
                    .consumer_tick
                    .or_else(|| execution_tick(tile, model, row))
                    .unwrap_or(model.rows.begin[row]);
                (tick, row)
            })
            .collect::<Vec<_>>()
    } else {
        model
            .incoming_dependencies(origin)
            .map(|dependency| {
                let row = dependency.producer_row as usize;
                let tick = dependency
                    .producer_tick
                    .or_else(|| execution_tick(tile, model, row))
                    .unwrap_or(model.rows.begin[row]);
                (tick, row)
            })
            .collect::<Vec<_>>()
    };
    let best_tick = if forward {
        candidates.iter().map(|(tick, _)| *tick).min()
    } else {
        candidates.iter().map(|(tick, _)| *tick).max()
    }?;
    candidates.retain(|(tick, _)| *tick == best_tick);
    candidates.sort_unstable_by_key(|(_, row)| *row);
    candidates.dedup_by_key(|(_, row)| *row);
    let index = if continuing_tie {
        (runtime.dependency_walk_index + 1) % candidates.len()
    } else {
        0
    };
    let target = candidates[index].1;
    if candidates.len() > 1 {
        runtime.dependency_walk_origin = Some(origin);
        runtime.dependency_walk_target = Some(target);
        runtime.dependency_walk_forward = forward;
        runtime.dependency_walk_index = index;
    } else {
        runtime.dependency_walk_origin = None;
        runtime.dependency_walk_target = None;
        runtime.dependency_walk_index = 0;
    }
    Some(target)
}

fn keyboard_or_center_row(
    tile: &KonataTileState,
    model: &KonataModel,
    keyboard_row: Option<usize>,
    canvas_height: f32,
) -> Option<usize> {
    keyboard_row
        .filter(|row| {
            *row < model.row_count()
                && (!tile.config.hide_flushed || model.visibility.is_visible(*row))
        })
        .or_else(|| {
            let visible = (tile.viewport.top_visible_row
                + f64::from(canvas_height) / row_pitch(tile, model) * 0.5)
                .round()
                .clamp(0.0, visible_row_count(tile, model).saturating_sub(1) as f64)
                as usize;
            physical_row(tile, model, visible)
        })
}

fn advance_keyboard_row(
    tile: &KonataTileState,
    model: &KonataModel,
    keyboard_row: Option<usize>,
    delta: isize,
) -> Option<usize> {
    let count = visible_row_count(tile, model);
    if count == 0 {
        return None;
    }
    let current = keyboard_row
        .filter(|row| {
            *row < model.row_count()
                && (!tile.config.hide_flushed || model.visibility.is_visible(*row))
        })
        .map_or_else(
            || {
                tile.viewport
                    .top_visible_row
                    .round()
                    .clamp(0.0, count.saturating_sub(1) as f64) as usize
            },
            |row| visible_index_of(tile, model, row),
        );
    let next = current
        .saturating_add_signed(delta)
        .min(count.saturating_sub(1));
    physical_row(tile, model, next)
}

fn zoom_density_pixel(
    tile: &mut KonataTileState,
    model: &KonataModel,
    pointer: Pos2,
    canvas_rect: Rect,
) {
    let from = y_to_visible_row(tile, model, pointer.y.floor(), canvas_rect.top())
        .floor()
        .max(0.0) as usize;
    let to = y_to_visible_row(tile, model, pointer.y.floor() + 1.0, canvas_rect.top())
        .ceil()
        .min(visible_row_count(tile, model) as f64) as usize;
    let Some((start, end, _, _)) = model.range_extent(from, to, tile.config.hide_flushed) else {
        return;
    };
    let span = end.saturating_sub(start).max(1) as f64;
    tile.viewport.top_visible_row = from as f64;
    tile.viewport.row_height_px = tile
        .viewport
        .row_height_px
        .max(f64::from(tile.config.text_lod_px));
    tile.viewport.px_per_tick = (f64::from(canvas_rect.width()) * 0.9 / span).max(0.000_001);
    let margin = span * 0.05;
    tile.viewport.set_left_tick(
        (start as f64 - margin)
            .floor()
            .clamp(i64::MIN as f64, i64::MAX as f64) as i64,
    );
}

fn interact_minimap(
    tile: &mut KonataTileState,
    model: &KonataModel,
    canvas_rect: Rect,
    minimap_rect: Rect,
    response: &Response,
) {
    response
        .clone()
        .on_hover_text("Pipeline overview — click or drag to reposition the viewport");
    if !(response.clicked() || response.dragged()) {
        return;
    }
    let Some(pointer) = response.interact_pointer_pos() else {
        return;
    };
    let count = visible_row_count(tile, model);
    let Some((first_tick, last_tick, _, _)) =
        model.range_extent(0, count, tile.config.hide_flushed)
    else {
        return;
    };
    let row_fraction =
        f64::from(pointer.y - minimap_rect.top()) / f64::from(minimap_rect.height().max(1.0));
    let visible_rows = f64::from(canvas_rect.height()) / row_pitch(tile, model);
    tile.viewport.top_visible_row = (row_fraction * count as f64 - visible_rows * 0.5)
        .clamp(-2.0, count.saturating_sub(1) as f64 + 2.0);

    let time_fraction =
        f64::from(pointer.x - minimap_rect.left()) / f64::from(minimap_rect.width().max(1.0));
    let tick = first_tick as f64 + time_fraction * last_tick.saturating_sub(first_tick) as f64;
    let visible_ticks = f64::from(canvas_rect.width()) / tile.viewport.px_per_tick;
    tile.viewport.set_left_tick(
        (tick - visible_ticks * 0.5)
            .round()
            .clamp(i64::MIN as f64, i64::MAX as f64) as i64,
    );
}

fn paint_minimap(
    painter: &egui::Painter,
    tile: &KonataTileState,
    model: &KonataModel,
    canvas_rect: Rect,
    rect: Rect,
) {
    let painter = painter.with_clip_rect(rect);
    painter.rect_filled(rect, 2.0, Color32::from_rgb(11, 16, 20));
    let count = visible_row_count(tile, model);
    let Some((first_tick, last_tick, _, _)) =
        model.range_extent(0, count, tile.config.hide_flushed)
    else {
        return;
    };
    let time_span = last_tick.saturating_sub(first_tick).max(1) as f64;
    let map_x = |tick: u64| {
        rect.left()
            + (tick.saturating_sub(first_tick) as f64 / time_span * f64::from(rect.width())) as f32
    };
    let pixel_count = rect.height().ceil().max(1.0) as usize;
    for pixel in 0..pixel_count {
        let start = pixel * count / pixel_count;
        let end = ((pixel + 1) * count / pixel_count)
            .max(start + 1)
            .min(count);
        let Some((min_begin, max_end, flushed, rows)) =
            model.range_extent(start, end, tile.config.hide_flushed)
        else {
            continue;
        };
        let y = rect.top() + pixel as f32;
        let flush_ratio = flushed as f32 / rows.max(1) as f32;
        let color = Color32::from_rgb(
            (75.0 + 80.0 * flush_ratio) as u8,
            (165.0 - 95.0 * flush_ratio) as u8,
            (205.0 - 120.0 * flush_ratio) as u8,
        );
        painter.line_segment(
            [Pos2::new(map_x(min_begin), y), Pos2::new(map_x(max_end), y)],
            Stroke::new(1.0, color),
        );
    }

    let visible_rows = f64::from(canvas_rect.height()) / row_pitch(tile, model);
    let lens_top = rect.top()
        + (tile.viewport.top_visible_row.max(0.0) / count.max(1) as f64 * f64::from(rect.height()))
            as f32;
    let lens_bottom = rect.top()
        + ((tile.viewport.top_visible_row + visible_rows).max(0.0) / count.max(1) as f64
            * f64::from(rect.height())) as f32;
    let left_tick = tile.viewport.left_tick as f64 + tile.viewport.left_frac;
    let right_tick = left_tick + f64::from(canvas_rect.width()) / tile.viewport.px_per_tick;
    let map_view_x = |tick: f64| {
        rect.left() + ((tick - first_tick as f64) / time_span * f64::from(rect.width())) as f32
    };
    let raw_left = map_view_x(left_tick);
    let raw_right = map_view_x(right_tick);
    let lens_center = Pos2::new(
        ((raw_left + raw_right) * 0.5).clamp(rect.left(), rect.right()),
        ((lens_top + lens_bottom) * 0.5).clamp(rect.top(), rect.bottom()),
    );
    let lens = Rect::from_center_size(
        lens_center,
        Vec2::new(
            (raw_right - raw_left).abs().max(8.0),
            (lens_bottom - lens_top).abs().max(6.0),
        ),
    )
    .intersect(rect);
    painter.rect_filled(
        lens,
        1.0,
        Color32::from_rgba_unmultiplied(220, 230, 240, 22),
    );
    painter.rect_stroke(
        lens,
        1.0,
        Stroke::new(1.3, Color32::from_rgb(225, 235, 245)),
        StrokeKind::Inside,
    );
}

pub(crate) fn synchronize_tiles(
    state: &SystemState,
    tiles: &mut HashMap<KonataTileId, KonataTileState>,
    source_id: KonataTileId,
) {
    let Some(source_tile) = tiles.get(&source_id).cloned() else {
        return;
    };
    let Some(source_group) = synchronization_group(&source_tile) else {
        return;
    };
    let Some(source_model) = state
        .konata_runtime
        .get(&source_id)
        .and_then(|runtime| runtime.entry.as_ref())
        .and_then(|entry| entry.model())
    else {
        return;
    };
    let source_visible = source_tile.viewport.top_visible_row.round().max(0.0) as usize;
    let source_row = if source_tile.config.hide_flushed {
        source_model.visibility.select(source_visible)
    } else {
        (source_visible < source_model.row_count()).then_some(source_visible)
    };
    let Some(source_row) = source_row else {
        return;
    };
    let tick_offset = source_tile.viewport.left_tick as f64 + source_tile.viewport.left_frac
        - source_model.rows.begin[source_row] as f64;

    for (target_id, target_tile) in tiles.iter_mut().filter(|(target_id, target)| {
        **target_id != source_id && synchronization_group(target) == Some(source_group)
    }) {
        target_tile.config.splitter_px = source_tile.config.splitter_px;
        target_tile.viewport.px_per_tick = source_tile.viewport.px_per_tick;
        target_tile.viewport.row_height_px = source_tile.viewport.row_height_px;
        target_tile.config.alignment_mode = source_tile.config.alignment_mode;
        let Some(target_model) = state
            .konata_runtime
            .get(target_id)
            .and_then(|runtime| runtime.entry.as_ref())
            .and_then(|entry| entry.model())
        else {
            continue;
        };
        let Some(target_row) = aligned_row(
            &source_model,
            &target_model,
            source_row,
            source_tile.config.alignment_mode,
        ) else {
            continue;
        };
        target_tile.viewport.top_visible_row = if target_tile.config.hide_flushed {
            target_model.visibility.rank(target_row) as f64
        } else {
            target_row as f64
        };
        let left = target_model.rows.begin[target_row] as f64 + tick_offset;
        let integral = left.floor().clamp(i64::MIN as f64, i64::MAX as f64);
        target_tile.viewport.set_left_tick(integral as i64);
        target_tile.viewport.left_frac = left - integral;
    }
}

fn synchronization_group(tile: &KonataTileState) -> Option<u64> {
    tile.config
        .sync_group
        .or(tile.config.synchronize_scroll.then_some(0))
}

fn alignment_label(mode: KonataAlignmentMode) -> &'static str {
    match mode {
        KonataAlignmentMode::ThreadRid => "thread + RID",
        KonataAlignmentMode::FetchId => "fetch ID",
        KonataAlignmentMode::Timestamp => "timestamp",
    }
}

fn aligned_row(
    source: &KonataModel,
    target: &KonataModel,
    source_row: usize,
    mode: KonataAlignmentMode,
) -> Option<usize> {
    match mode {
        KonataAlignmentMode::ThreadRid => {
            let rid = source.rows.rid(source_row)?;
            let thread = source
                .rows
                .tid(source_row)
                .map(|tid| source.thread_name(tid));
            target.row_for_thread_rid(thread, rid)
        }
        KonataAlignmentMode::FetchId => (source_row < target.row_count()).then_some(source_row),
        KonataAlignmentMode::Timestamp => {
            target.nearest_row_for_tick(source.rows.begin[source_row])
        }
    }
}

fn overlay_hit_is_closer(
    tile: &KonataTileState,
    model: &KonataModel,
    overlay: &KonataModel,
    pointer: Pos2,
    canvas_rect: Rect,
) -> bool {
    let Some(row) = row_at_pointer(tile, model, pointer, canvas_rect) else {
        return false;
    };
    let Some(overlay_row) = aligned_row(model, overlay, row, tile.config.alignment_mode) else {
        return false;
    };
    let displayed_tick = tile.viewport.x_to_tick(pointer.x, canvas_rect.left());
    let raw_overlay_tick =
        displayed_tick + overlay.rows.begin[overlay_row] as f64 - model.rows.begin[row] as f64;
    let tolerance = 3.0 / tile.viewport.px_per_tick;
    let primary = closest_stage_distance(model, row, displayed_tick, tolerance);
    let front = closest_stage_distance(overlay, overlay_row, raw_overlay_tick, tolerance);
    matches!((primary, front), (None, Some(_)))
        || matches!((primary, front), (Some(back), Some(front)) if front < back)
}

fn closest_stage_distance(
    model: &KonataModel,
    row: usize,
    tick: f64,
    tolerance: f64,
) -> Option<f64> {
    model
        .stages_for_row(row)
        .iter()
        .filter_map(|stage| {
            let start = stage.start as f64;
            let end = stage.end.max(stage.start) as f64;
            let hit = if start == end {
                (tick - start).abs() <= tolerance
            } else {
                tick >= start && tick < end
            };
            hit.then(|| (tick - (start + end) * 0.5).abs())
        })
        .min_by(f64::total_cmp)
}

#[allow(clippy::too_many_arguments)]
fn paint(
    state: &SystemState,
    ui: &Ui,
    tile_id: KonataTileId,
    tile: &KonataTileState,
    model: &KonataModel,
    overlay: Option<(KonataTileId, &KonataModel, &KonataTileState)>,
    label_rect: Rect,
    ruler_rect: Rect,
    canvas_rect: Rect,
    status_rect: Rect,
    splitter: Rect,
) {
    let theme = &state.user.config.theme;
    let painter = ui.painter();
    let background = theme.primary_ui_color.background;
    let chrome = theme.secondary_ui_color.background;
    let foreground = theme.foreground;
    let body_rect = label_rect
        .union(ruler_rect)
        .union(canvas_rect)
        .union(status_rect)
        .union(splitter);
    painter.rect_filled(body_rect, 0.0, background);
    painter.rect_filled(label_rect, 0.0, chrome);
    painter.rect_filled(ruler_rect, 0.0, chrome);
    painter.rect_filled(status_rect, 0.0, chrome);
    painter.rect_filled(canvas_rect, 0.0, multiply(background, 0.72));
    painter.rect_filled(splitter, 0.0, multiply(foreground, 0.24));

    paint_ruler(state, painter, tile, ruler_rect, canvas_rect, foreground);
    paint_rows(
        state,
        painter,
        tile_id,
        tile,
        model,
        overlay,
        label_rect,
        canvas_rect,
        foreground,
    );
    paint_cursor_and_markers(state, painter, tile, ruler_rect, canvas_rect);
    paint_status(
        state,
        ui,
        tile_id,
        tile,
        model,
        status_rect,
        canvas_rect,
        foreground,
    );
}

fn register_accessibility(
    state: &mut SystemState,
    ui: &Ui,
    tile_id: KonataTileId,
    tile: &KonataTileState,
    model: &KonataModel,
    label_rect: Rect,
    canvas_rect: Rect,
) {
    let count = visible_row_count(tile, model);
    if count == 0 {
        state
            .konata_runtime
            .entry(tile_id)
            .or_default()
            .keyboard_region_focused = false;
        return;
    }
    let pitch = row_pitch(tile, model);
    let start = tile.viewport.top_visible_row.floor().max(0.0) as usize;
    let end = (tile.viewport.top_visible_row + f64::from(canvas_rect.height()) / pitch)
        .ceil()
        .max(0.0) as usize;
    let end = end.min(count);
    if start < end {
        let first = physical_row(tile, model, start).unwrap_or_default();
        let last = physical_row(tile, model, end - 1).unwrap_or(first);
        model.prefetch_detail_rows(
            first.saturating_sub(super::KONATA_DETAIL_PAGE_ROWS)
                ..(last + super::KONATA_DETAIL_PAGE_ROWS + 1).min(model.row_count()),
        );
    }
    let keyboard_row = state
        .konata_runtime
        .get(&tile_id)
        .and_then(|runtime| runtime.keyboard_row)
        .or_else(|| focused_row(state, tile, model));
    let mut newly_focused = None;
    let mut region_focused = false;

    // Below one pixel per row, exposing every mathematically visible row would
    // make the accessibility tree trace-sized. Keep one keyboard anchor and
    // let arrow navigation scroll it into a newly bounded visible subtree.
    let accessible_range = if pitch < 1.0 {
        let center = (start + end.saturating_sub(1)) / 2;
        let anchor = keyboard_row
            .filter(|row| !tile.config.hide_flushed || model.visibility.is_visible(*row))
            .map(|row| visible_index_of(tile, model, row))
            .filter(|visible| *visible >= start && *visible < end)
            .unwrap_or(center);
        anchor..(anchor + 1).min(end)
    } else {
        start..end
    };
    for visible in accessible_range {
        let Some(row) = physical_row(tile, model, visible) else {
            continue;
        };
        let y = visible_row_to_y(tile, model, visible as f64, canvas_rect.top());
        let rect = Rect::from_min_max(
            Pos2::new(label_rect.left(), y),
            Pos2::new(label_rect.right(), y + pitch.max(1.0) as f32),
        )
        .intersect(label_rect);
        if !rect.is_positive() {
            continue;
        }
        let response = ui.interact(
            rect,
            ui.id().with(("konata_accessible_row", tile_id.0, row)),
            Sense::focusable_noninteractive(),
        );
        let accessible_name = accessible_row_name(model, row);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, accessible_name.clone())
        });
        region_focused |= response.has_focus();
        if response.gained_focus() {
            newly_focused = Some((row, None));
        }
    }

    let stage_row = newly_focused
        .map(|(row, _)| row)
        .or(keyboard_row)
        .filter(|row| !tile.config.hide_flushed || model.visibility.is_visible(*row));
    if let Some(row) = stage_row {
        let visible = visible_index_of(tile, model, row);
        if visible >= start && visible < end {
            let y = visible_row_to_y(tile, model, visible as f64, canvas_rect.top());
            for stage in model.stages_for_row(row).iter() {
                let left = tile.viewport.tick_to_x(stage.start, canvas_rect.left());
                let right = if stage.end <= stage.start {
                    left + 6.0
                } else {
                    tile.viewport.tick_to_x(stage.end, canvas_rect.left())
                };
                let rect = Rect::from_min_max(
                    Pos2::new(left.min(right), y),
                    Pos2::new(left.max(right), y + pitch.max(1.0) as f32),
                )
                .intersect(canvas_rect);
                if !rect.is_positive() {
                    continue;
                }
                let response = ui.interact(
                    rect,
                    ui.id()
                        .with(("konata_accessible_stage", tile_id.0, stage.event_tx)),
                    Sense::focusable_noninteractive(),
                );
                let accessible_name = accessible_stage_name(model, row, stage);
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Label,
                        true,
                        accessible_name.clone(),
                    )
                });
                region_focused |= response.has_focus();
                if response.gained_focus() {
                    newly_focused = Some((row, Some(stage.event_tx)));
                }
            }
        }
    }
    let runtime = state.konata_runtime.entry(tile_id).or_default();
    runtime.keyboard_region_focused = region_focused;
    if let Some((row, stage)) = newly_focused {
        runtime.keyboard_row = Some(row);
        runtime.keyboard_stage_event = stage;
    }
}

fn accessible_row_name(model: &KonataModel, row: usize) -> String {
    let mut name = format!("Instruction ID {row}");
    if let Some(sid) = model.rows.sid(row) {
        name.push_str(&format!(", SID {sid}"));
    }
    if let Some(tid) = model.rows.tid(row) {
        name.push_str(&format!(", thread {}", model.thread_name(tid)));
    }
    if let Some(rid) = model.rows.rid(row) {
        name.push_str(&format!(", RID {rid}"));
    }
    if let Some(label) = model.rows.label(row) {
        name.push_str(&format!(", {}", model.string(label)));
    }
    match model.rows.flushed[row] {
        FlushState::True => name.push_str(", flushed"),
        FlushState::Unknown => name.push_str(", flush state unknown"),
        FlushState::False => {}
    }
    if model.rows.flags[row].0 != 0 {
        name.push_str(", data-quality warning");
    }
    name.push_str(&format!(", {} stages", model.stages_for_row(row).len()));
    name
}

fn accessible_stage_name(model: &KonataModel, row: usize, stage: &KonataStage) -> String {
    let duration = stage.end.saturating_sub(stage.start);
    let warning = if stage.flags.0 != 0 { ", warning" } else { "" };
    format!(
        "Stage {}, start {}, end {}, duration {}, lane {}, parent instruction ID {}{}",
        model.stage_name(stage),
        stage.start,
        stage.end,
        duration,
        model.lane_name(stage.lane),
        row,
        warning,
    )
}

fn paint_ruler(
    state: &SystemState,
    painter: &egui::Painter,
    tile: &KonataTileState,
    ruler_rect: Rect,
    canvas_rect: Rect,
    foreground: Color32,
) {
    let left = tile
        .viewport
        .x_to_tick(canvas_rect.left(), canvas_rect.left());
    let right = tile
        .viewport
        .x_to_tick(canvas_rect.right(), canvas_rect.left());
    let font = FontId::monospace(10.0);
    let grid = Color32::from_rgba_unmultiplied(foreground.r(), foreground.g(), foreground.b(), 35);
    let (mut value, end, step, to_tick): (f64, f64, f64, Box<dyn Fn(f64) -> f64>) =
        if tile.config.ruler_cycles
            && let Some(period) = tile.config.clock_period_ticks.filter(|period| *period > 0)
        {
            let origin = tile.config.clock_origin_tick as f64;
            let period = period as f64;
            let left_cycle = (left - origin) / period;
            let right_cycle = (right - origin) / period;
            let step = nice_tick_step(90.0 / (tile.viewport.px_per_tick * period)).max(1.0);
            (
                (left_cycle / step).floor() * step,
                right_cycle,
                step,
                Box::new(move |cycle| origin + cycle * period),
            )
        } else {
            let step = nice_tick_step(90.0 / tile.viewport.px_per_tick).max(1.0);
            (
                (left / step).floor() * step,
                right,
                step,
                Box::new(|tick| tick),
            )
        };
    let mut budget = 0;
    while value <= end && budget < 2048 {
        let tick = to_tick(value);
        if tick >= 0.0 && tick <= u64::MAX as f64 {
            let x = tile.viewport.tick_to_x(tick as u64, canvas_rect.left());
            painter.line_segment(
                [
                    Pos2::new(x, ruler_rect.bottom()),
                    Pos2::new(x, canvas_rect.bottom()),
                ],
                Stroke::new(1.0, grid),
            );
            let label = if tile.config.ruler_cycles
                && tile
                    .config
                    .clock_period_ticks
                    .is_some_and(|period| period > 0)
            {
                format!("{value:.0}")
            } else {
                format_trace_tick(state, tile, tick as u64)
            };
            painter.text(
                Pos2::new(x + 3.0, ruler_rect.center().y),
                Align2::LEFT_CENTER,
                label,
                font.clone(),
                foreground,
            );
        }
        value += step;
        budget += 1;
    }
}

fn nice_tick_step(raw: f64) -> f64 {
    if !raw.is_finite() || raw <= 1.0 {
        return 1.0;
    }
    let magnitude = 10.0_f64.powf(raw.log10().floor());
    let normalized = raw / magnitude;
    let factor = if normalized <= 1.0 {
        1.0
    } else if normalized <= 2.0 {
        2.0
    } else if normalized <= 5.0 {
        5.0
    } else {
        10.0
    };
    factor * magnitude
}

fn format_trace_tick(state: &SystemState, tile: &KonataTileState, tick: u64) -> String {
    state
        .user
        .waves
        .as_ref()
        .and_then(|waves| waves.data_container_for_source(tile.spec.source))
        .map_or_else(
            || tick.to_string(),
            |container| {
                time_string(
                    &BigInt::from(tick),
                    &container.metadata().timescale,
                    &state.user.wanted_timeunit,
                    &state.get_time_format(),
                )
            },
        )
}

#[allow(clippy::too_many_arguments)]
fn paint_rows(
    state: &SystemState,
    painter: &egui::Painter,
    tile_id: KonataTileId,
    tile: &KonataTileState,
    model: &KonataModel,
    overlay: Option<(KonataTileId, &KonataModel, &KonataTileState)>,
    label_rect: Rect,
    canvas_rect: Rect,
    foreground: Color32,
) {
    let pipeline_theme = &state.user.config.theme.konata;
    let canvas_background = multiply(state.user.config.theme.primary_ui_color.background, 0.72);
    let count = visible_row_count(tile, model);
    let start = tile.viewport.top_visible_row.floor().max(0.0) as usize;
    let end = (tile.viewport.top_visible_row
        + f64::from(canvas_rect.height()) / row_pitch(tile, model))
    .ceil()
    .max(0.0) as usize;
    let end = end.min(count);
    let valid_top = visible_row_to_y(tile, model, 0.0, canvas_rect.top());
    let valid_bottom = visible_row_to_y(tile, model, count as f64, canvas_rect.top());
    painter.with_clip_rect(canvas_rect).rect_filled(
        Rect::from_min_max(
            Pos2::new(canvas_rect.left(), valid_top),
            Pos2::new(canvas_rect.right(), valid_bottom),
        )
        .intersect(canvas_rect),
        0.0,
        Color32::from_rgba_unmultiplied(foreground.r(), foreground.g(), foreground.b(), 4),
    );
    if effective_detail(tile) < f64::from(tile.config.color_lod_px) {
        paint_density(painter, tile, model, canvas_rect, start, end);
        return;
    }

    let detailed_stage_count = (start..end)
        .filter_map(|visible| physical_row(tile, model, visible))
        .try_fold(0usize, |count, row| {
            count
                .checked_add(model.stages_for_row(row).len())
                .filter(|count| *count <= MAX_DETAILED_STAGES)
        });
    if detailed_stage_count.is_none() {
        paint_density(painter, tile, model, canvas_rect, start, end);
        return;
    }

    let label_painter = painter.with_clip_rect(label_rect);
    let canvas_painter = painter.with_clip_rect(canvas_rect);
    let font = FontId::monospace((tile.viewport.row_height_px * 0.48).clamp(8.0, 13.0) as f32);
    let focused = focused_row(state, tile, model);
    let producer_chain = state
        .konata_runtime
        .get(&tile_id)
        .and_then(|runtime| runtime.producer_chain.as_deref());
    let keyboard_row = state
        .konata_runtime
        .get(&tile_id)
        .and_then(|runtime| runtime.keyboard_row);
    let keyboard_stage = state
        .konata_runtime
        .get(&tile_id)
        .and_then(|runtime| runtime.keyboard_stage_event);
    let focused_tx = focused_transaction_id(state, tile);
    let overlay_emphasis = state
        .konata_runtime
        .get(&tile_id)
        .and_then(|runtime| runtime.overlay_emphasis);
    let overlay_target = overlay.map(|(target_id, _, _)| target_id);
    let primary_opacity = match overlay_emphasis {
        Some(emphasized) if Some(emphasized) == overlay_target => 0.20,
        _ => 1.0,
    };
    let overlay_opacity = match overlay_emphasis {
        Some(emphasized) if Some(emphasized) == overlay_target => 1.0,
        Some(_) => 0.18,
        None => 0.52,
    };
    let effective = effective_detail(tile);
    let visible_left = tile
        .viewport
        .x_to_tick(canvas_rect.left(), canvas_rect.left())
        .max(0.0) as u64;
    let visible_right = tile
        .viewport
        .x_to_tick(canvas_rect.right(), canvas_rect.left())
        .clamp(0.0, u64::MAX as f64) as u64;

    for visible in start..end {
        let Some(row) = physical_row(tile, model, visible) else {
            continue;
        };
        let y = visible_row_to_y(tile, model, visible as f64, canvas_rect.top());
        let height = row_pitch(tile, model) as f32;
        let row_rect = Rect::from_min_max(
            Pos2::new(canvas_rect.left(), y),
            Pos2::new(canvas_rect.right(), y + height),
        );
        if row % 2 == 1 {
            canvas_painter.rect_filled(
                row_rect,
                0.0,
                Color32::from_rgba_unmultiplied(foreground.r(), foreground.g(), foreground.b(), 10),
            );
        }
        if producer_chain.is_some_and(|chain| chain.contains(row)) {
            canvas_painter.rect_filled(
                row_rect,
                0.0,
                Color32::from_rgba_unmultiplied(225, 170, 45, 34),
            );
            label_painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(label_rect.left(), row_rect.top()),
                    Pos2::new(label_rect.right(), row_rect.bottom()),
                ),
                0.0,
                Color32::from_rgba_unmultiplied(225, 170, 45, 24),
            );
        }

        for stage in model.stages_for_row(row).iter().filter(|stage| {
            stage.start <= visible_right && stage.end.max(stage.start) >= visible_left
        }) {
            paint_stage(
                &canvas_painter,
                tile,
                model,
                row,
                stage,
                row_rect,
                canvas_rect,
                effective,
                focused == Some(row),
                focused_tx == Some(stage.event_tx) || keyboard_stage == Some(stage.event_tx),
                primary_opacity,
                pipeline_theme,
                canvas_background,
            );
        }

        if let Some((_, overlay_model, overlay_source_tile)) = overlay
            && let Some(overlay_row) =
                aligned_row(model, overlay_model, row, tile.config.alignment_mode)
        {
            let mut overlay_tile = overlay_source_tile.clone();
            overlay_tile.viewport = tile.viewport;
            let shift = i128::from(model.rows.begin[row])
                - i128::from(overlay_model.rows.begin[overlay_row]);
            for stage in overlay_model
                .stages_for_row(overlay_row)
                .iter()
                .filter(|stage| {
                    let start = shift_tick(stage.start, shift);
                    let end = shift_tick(stage.end, shift).max(start);
                    start <= visible_right && end >= visible_left
                })
            {
                let mut shifted = stage.clone();
                shifted.start = shift_tick(stage.start, shift);
                shifted.end = shift_tick(stage.end, shift);
                paint_stage(
                    &canvas_painter,
                    &overlay_tile,
                    overlay_model,
                    overlay_row,
                    &shifted,
                    row_rect,
                    canvas_rect,
                    effective,
                    false,
                    false,
                    overlay_opacity,
                    pipeline_theme,
                    canvas_background,
                );
            }
        }

        if model.rows.flushed[row] == FlushState::True {
            canvas_painter.rect_filled(
                row_rect,
                0.0,
                Color32::from_rgba_unmultiplied(
                    pipeline_theme.flush_overlay.r(),
                    pipeline_theme.flush_overlay.g(),
                    pipeline_theme.flush_overlay.b(),
                    145,
                ),
            );
        }
        if model.rows.flags[row].0 != 0 {
            canvas_painter.rect_stroke(
                row_rect.shrink(0.5),
                0.0,
                Stroke::new(1.0, pipeline_theme.warning),
                StrokeKind::Inside,
            );
        }
        if focused == Some(row) {
            canvas_painter.rect_stroke(
                row_rect.shrink(1.0),
                0.0,
                Stroke::new(2.0, pipeline_theme.focus),
                StrokeKind::Inside,
            );
        } else if keyboard_row == Some(row) {
            canvas_painter.rect_stroke(
                row_rect.shrink(1.0),
                0.0,
                Stroke::new(1.5, Color32::from_rgb(105, 205, 255)),
                StrokeKind::Inside,
            );
            label_painter.text(
                Pos2::new(label_rect.right() - 5.0, row_rect.center().y),
                Align2::RIGHT_CENTER,
                "◆",
                FontId::proportional(9.0),
                Color32::from_rgb(105, 205, 255),
            );
        }

        if tile.viewport.row_height_px >= f64::from(tile.config.text_lod_px) {
            let label = row_label(model, row);
            label_painter.text(
                Pos2::new(label_rect.left() + LABEL_PADDING, row_rect.center().y),
                Align2::LEFT_CENTER,
                label,
                font.clone(),
                foreground,
            );
        }
    }
    paint_dependencies(
        &canvas_painter,
        tile,
        model,
        canvas_rect,
        start,
        end,
        focused,
        producer_chain,
        effective,
        pipeline_theme,
    );
}

#[allow(clippy::too_many_arguments)]
fn paint_dependencies(
    painter: &egui::Painter,
    tile: &KonataTileState,
    model: &KonataModel,
    canvas_rect: Rect,
    start_visible: usize,
    end_visible: usize,
    focused_row: Option<usize>,
    producer_chain: Option<&super::KonataRowSet>,
    effective: f64,
    theme: &crate::config::KonataTheme,
) {
    if tile.config.arrow_style == KonataArrowStyle::Hidden
        || effective < f64::from(tile.config.arrow_lod_px)
        || start_visible >= end_visible
    {
        return;
    }
    let Some(first_row) = physical_row(tile, model, start_visible) else {
        return;
    };
    let Some(last_row) = physical_row(tile, model, end_visible - 1) else {
        return;
    };
    let row_start = first_row.min(last_row);
    let row_end = first_row.max(last_row) + 1;
    let mut drawn = 0usize;
    let mut suppressed = 0usize;
    for dependency in model
        .dependencies_in_row_window(row_start, row_end)
        .iter()
        .filter(|dependency| {
            let producer = dependency.producer_row as usize;
            let consumer = dependency.consumer_row as usize;
            producer >= row_start
                && producer < row_end
                && consumer >= row_start
                && consumer < row_end
                && (!tile.config.hide_flushed
                    || model.visibility.is_visible(producer)
                        && model.visibility.is_visible(consumer))
        })
    {
        if drawn < 4_000 {
            paint_dependency(
                painter,
                tile,
                model,
                dependency,
                canvas_rect,
                focused_row,
                producer_chain,
                theme,
            );
            drawn += 1;
        } else {
            suppressed += 1;
        }
    }
    if suppressed > 0 {
        painter.text(
            canvas_rect.right_top() + Vec2::new(-8.0, 8.0),
            Align2::RIGHT_TOP,
            format!("{suppressed} dependency edges suppressed"),
            FontId::monospace(10.0),
            Color32::from_rgb(245, 180, 80),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_dependency(
    painter: &egui::Painter,
    tile: &KonataTileState,
    model: &KonataModel,
    dependency: &KonataDependency,
    canvas_rect: Rect,
    focused_row: Option<usize>,
    producer_chain: Option<&super::KonataRowSet>,
    theme: &crate::config::KonataTheme,
) {
    let producer = dependency.producer_row as usize;
    let consumer = dependency.consumer_row as usize;
    let producer_visible = visible_index_of(tile, model, producer);
    let consumer_visible = visible_index_of(tile, model, consumer);
    let producer_y = visible_row_to_y(
        tile,
        model,
        producer_visible as f64 + 0.5,
        canvas_rect.top(),
    );
    let consumer_y = visible_row_to_y(
        tile,
        model,
        consumer_visible as f64 + 0.5,
        canvas_rect.top(),
    );
    let emphasized = producer_chain.map_or_else(
        || focused_row.is_none_or(|focused| focused == producer || focused == consumer),
        |chain| chain.contains(producer) && chain.contains(consumer),
    );
    let base = palette_color(
        &theme.stage_palette,
        dependency.name as usize + 7,
        theme.focus,
    );
    let color = Color32::from_rgba_unmultiplied(
        base.r(),
        base.g(),
        base.b(),
        if emphasized { 220 } else { 55 },
    );
    let stroke = Stroke::new(if emphasized { 1.8 } else { 1.0 }, color);
    let producer_anchor = dependency
        .producer_tick
        .or_else(|| execution_tick(tile, model, producer));
    let consumer_anchor = dependency
        .consumer_tick
        .or_else(|| execution_tick(tile, model, consumer));
    let producer_tick = producer_anchor.unwrap_or(model.rows.begin[producer]);
    let consumer_tick = consumer_anchor.unwrap_or(model.rows.begin[consumer]);
    let producer_point = Pos2::new(
        tile.viewport.tick_to_x(producer_tick, canvas_rect.left()),
        producer_y,
    );
    let consumer_point = Pos2::new(
        tile.viewport.tick_to_x(consumer_tick, canvas_rect.left()),
        consumer_y,
    );

    match tile.config.arrow_style {
        KonataArrowStyle::Inside => {
            painter.line_segment([producer_point, consumer_point], stroke);
        }
        KonataArrowStyle::LeftCurve => {
            let left = canvas_rect.left() + 6.0;
            let producer_fetch = Pos2::new(
                tile.viewport
                    .tick_to_x(model.rows.begin[producer], canvas_rect.left()),
                producer_y,
            );
            let consumer_fetch = Pos2::new(
                tile.viewport
                    .tick_to_x(model.rows.begin[consumer], canvas_rect.left()),
                consumer_y,
            );
            painter.line(
                vec![
                    producer_fetch,
                    Pos2::new(left, producer_y),
                    Pos2::new(left, consumer_y),
                    consumer_fetch,
                ],
                stroke,
            );
        }
        KonataArrowStyle::Hidden => return,
    }
    let target = if tile.config.arrow_style == KonataArrowStyle::Inside {
        consumer_point
    } else {
        Pos2::new(
            tile.viewport
                .tick_to_x(model.rows.begin[consumer], canvas_rect.left()),
            consumer_y,
        )
    };
    if producer_anchor.is_none() {
        painter.circle_stroke(producer_point, 3.0, stroke);
    }
    if consumer_anchor.is_none() {
        painter.circle_stroke(consumer_point, 3.0, stroke);
    }
    let previous = if tile.config.arrow_style == KonataArrowStyle::Inside {
        producer_point
    } else {
        Pos2::new(canvas_rect.left() + 6.0, consumer_y)
    };
    let direction = (target - previous).normalized();
    let normal = Vec2::new(-direction.y, direction.x);
    let base = target - direction * 6.0;
    painter.line_segment([target, base + normal * 3.5], stroke);
    painter.line_segment([target, base - normal * 3.5], stroke);
}

#[allow(clippy::too_many_arguments)]
fn paint_stage(
    painter: &egui::Painter,
    tile: &KonataTileState,
    model: &KonataModel,
    row: usize,
    stage: &KonataStage,
    row_rect: Rect,
    canvas_rect: Rect,
    effective: f64,
    focused: bool,
    focused_stage: bool,
    opacity: f32,
    theme: &crate::config::KonataTheme,
    canvas_background: Color32,
) {
    let x0 = tile.viewport.tick_to_x(stage.start, canvas_rect.left());
    let invalid = stage.end < stage.start;
    let x1 = tile.viewport.tick_to_x(
        if invalid { stage.start } else { stage.end },
        canvas_rect.left(),
    );
    let lane_count = model.lane_names.len().max(1) as f32;
    let (top, bottom) = match tile.config.lane_mode {
        KonataLaneMode::Merged => {
            let inset = f32::from(stage.lane).min(3.0) * 1.2;
            (
                row_rect.top() + 2.0 + inset,
                row_rect.bottom() - 2.0 - inset,
            )
        }
        KonataLaneMode::SplitFixed | KonataLaneMode::SplitNatural => {
            let lane_height = row_rect.height() / lane_count;
            let top = row_rect.top() + f32::from(stage.lane) * lane_height;
            (top + 1.0, top + lane_height - 1.0)
        }
    };
    let base_color = stage_color(tile, model, row, stage, theme, canvas_background);
    let color = Color32::from_rgba_unmultiplied(
        base_color.r(),
        base_color.g(),
        base_color.b(),
        (255.0 * opacity).round() as u8,
    );
    let warning = stage.flags.contains(StageFlags::OUT_OF_RANGE)
        || stage.flags.contains(StageFlags::END_BEFORE_START)
        || stage.flags.contains(StageFlags::UNNAMED)
        || stage.flags.contains(StageFlags::MULTIPLE_PARENTS);
    let stroke_color = if warning {
        Color32::from_rgba_unmultiplied(
            theme.warning.r(),
            theme.warning.g(),
            theme.warning.b(),
            (255.0 * opacity).round() as u8,
        )
    } else if focused {
        theme.focus
    } else {
        multiply(color, 0.55)
    };

    if stage.start == stage.end || invalid {
        let center = Pos2::new(x0, (top + bottom) * 0.5);
        let radius = ((bottom - top) * 0.35).clamp(2.5, 7.0);
        painter.add(egui::Shape::convex_polygon(
            vec![
                Pos2::new(center.x, center.y - radius),
                Pos2::new(center.x + radius, center.y),
                Pos2::new(center.x, center.y + radius),
                Pos2::new(center.x - radius, center.y),
            ],
            color,
            Stroke::new(
                if focused_stage {
                    3.0
                } else if warning {
                    2.0
                } else {
                    1.0
                },
                stroke_color,
            ),
        ));
        return;
    }

    let stage_rect = Rect::from_min_max(Pos2::new(x0, top), Pos2::new(x1.max(x0 + 1.0), bottom));
    painter.rect_filled(stage_rect, 1.5, color);
    let highlight = Color32::from_rgba_unmultiplied(255, 255, 255, (28.0 * opacity) as u8);
    painter.rect_filled(
        Rect::from_min_max(
            stage_rect.min,
            Pos2::new(stage_rect.right(), stage_rect.center().y),
        ),
        1.5,
        highlight,
    );
    if effective >= f64::from(tile.config.frame_lod_px) {
        painter.rect_stroke(
            stage_rect,
            1.5,
            Stroke::new(
                if focused_stage {
                    3.0
                } else if warning || focused {
                    2.0
                } else {
                    1.0
                },
                stroke_color,
            ),
            StrokeKind::Inside,
        );
    }
    if effective >= f64::from(tile.config.text_lod_px) && stage_rect.width() >= 12.0 {
        let first_cell_end = tile
            .config
            .clock_period_ticks
            .filter(|period| *period > 0)
            .map_or(stage.end, |period| {
                stage.start.saturating_add(period).min(stage.end)
            });
        let first_cell_right = tile.viewport.tick_to_x(first_cell_end, canvas_rect.left());
        painter.text(
            Pos2::new(
                (stage_rect.left() + first_cell_right.min(stage_rect.right())) * 0.5,
                stage_rect.center().y,
            ),
            Align2::CENTER_CENTER,
            model.stage_name(stage),
            FontId::monospace(((bottom - top) * 0.48).clamp(7.0, 12.0)),
            Color32::from_rgba_unmultiplied(255, 255, 255, (255.0 * opacity) as u8),
        );
        if let Some(period) = tile.config.clock_period_ticks.filter(|period| *period > 0) {
            let cells = stage.end.saturating_sub(stage.start).div_ceil(period);
            for cell in 1..cells.min(64) {
                let cell_start = stage.start.saturating_add(cell.saturating_mul(period));
                let cell_end = cell_start.saturating_add(period).min(stage.end);
                let left = tile.viewport.tick_to_x(cell_start, canvas_rect.left());
                let right = tile.viewport.tick_to_x(cell_end, canvas_rect.left());
                if right - left >= 9.0 {
                    painter.text(
                        Pos2::new((left + right) * 0.5, stage_rect.center().y),
                        Align2::CENTER_CENTER,
                        cell.to_string(),
                        FontId::monospace(((bottom - top) * 0.42).clamp(7.0, 11.0)),
                        Color32::from_rgba_unmultiplied(255, 255, 255, (255.0 * opacity) as u8),
                    );
                }
            }
        }
    }
}

fn paint_density(
    painter: &egui::Painter,
    tile: &KonataTileState,
    model: &KonataModel,
    canvas_rect: Rect,
    start: usize,
    end: usize,
) {
    let painter = painter.with_clip_rect(canvas_rect);
    let pixel_start = canvas_rect.top().floor() as i32;
    let pixel_end = canvas_rect.bottom().ceil() as i32;
    for pixel in pixel_start..pixel_end {
        let from = y_to_visible_row(tile, model, pixel as f32, canvas_rect.top())
            .floor()
            .max(start as f64) as usize;
        let to = y_to_visible_row(tile, model, pixel as f32 + 1.0, canvas_rect.top())
            .ceil()
            .min(end as f64) as usize;
        if from >= to {
            continue;
        }
        let Some((min_begin, max_end, flushed, count)) =
            model.range_extent(from, to, tile.config.hide_flushed)
        else {
            continue;
        };
        let x0 = tile.viewport.tick_to_x(min_begin, canvas_rect.left());
        let x1 = tile.viewport.tick_to_x(max_end, canvas_rect.left());
        let flush_ratio = flushed as f32 / count as f32;
        let color = Color32::from_rgb(
            (54.0 + 45.0 * flush_ratio) as u8,
            (125.0 - 70.0 * flush_ratio) as u8,
            (190.0 - 90.0 * flush_ratio) as u8,
        );
        painter.line_segment(
            [Pos2::new(x0, pixel as f32), Pos2::new(x1, pixel as f32)],
            Stroke::new(1.0, color),
        );
    }
}

fn paint_cursor_and_markers(
    state: &SystemState,
    painter: &egui::Painter,
    tile: &KonataTileState,
    ruler_rect: Rect,
    canvas_rect: Rect,
) {
    let Some(waves) = state.user.waves.as_ref() else {
        return;
    };
    let painter = painter.with_clip_rect(ruler_rect.union(canvas_rect));
    if let Some(cursor) = waves.cursor.as_ref().and_then(|cursor| cursor.to_u64()) {
        let x = tile.viewport.tick_to_x(cursor, canvas_rect.left());
        painter.line_segment(
            [
                Pos2::new(x, ruler_rect.top()),
                Pos2::new(x, canvas_rect.bottom()),
            ],
            Stroke::new(1.5, Color32::from_rgb(245, 210, 65)),
        );
    }
    let mut marker_stacks = HashMap::<u64, usize>::new();
    for (id, time) in &waves.markers {
        let Some(time) = time.to_u64() else {
            continue;
        };
        let x = tile.viewport.tick_to_x(time, canvas_rect.left());
        let stack = marker_stacks.entry(time).or_default();
        painter.line_segment(
            [
                Pos2::new(x, ruler_rect.bottom() - 8.0),
                Pos2::new(x, canvas_rect.bottom()),
            ],
            Stroke::new(1.0, Color32::from_rgb(220, 100, 190)),
        );
        painter.text(
            Pos2::new(x + 2.0, ruler_rect.top() + 2.0 + *stack as f32 * 9.0),
            Align2::LEFT_TOP,
            id.to_string(),
            FontId::monospace(9.0),
            Color32::from_rgb(240, 150, 220),
        );
        *stack += 1;
    }
}

fn paint_range_selection(
    painter: &egui::Painter,
    tile: &KonataTileState,
    ruler_rect: Rect,
    canvas_rect: Rect,
    range: (u64, u64),
) {
    if range.0 == range.1 {
        return;
    }
    let x0 = tile.viewport.tick_to_x(range.0, canvas_rect.left());
    let x1 = tile.viewport.tick_to_x(range.1, canvas_rect.left());
    let rect = Rect::from_min_max(
        Pos2::new(x0.min(x1), ruler_rect.top()),
        Pos2::new(x0.max(x1), canvas_rect.bottom()),
    )
    .intersect(ruler_rect.union(canvas_rect));
    painter.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(90, 165, 245, 28));
    painter.line_segment(
        [rect.left_top(), rect.left_bottom()],
        Stroke::new(1.2, Color32::from_rgb(110, 185, 255)),
    );
    painter.line_segment(
        [rect.right_top(), rect.right_bottom()],
        Stroke::new(1.2, Color32::from_rgb(110, 185, 255)),
    );
}

#[allow(clippy::too_many_arguments)]
fn paint_status(
    state: &SystemState,
    ui: &Ui,
    tile_id: KonataTileId,
    tile: &KonataTileState,
    model: &KonataModel,
    status_rect: Rect,
    canvas_rect: Rect,
    foreground: Color32,
) {
    let pointer = ui.input(|input| input.pointer.hover_pos());
    let position = pointer
        .filter(|pointer| canvas_rect.contains(*pointer))
        .map(|pointer| {
            let tick = tile.viewport.x_to_tick(pointer.x, canvas_rect.left());
            let visible = y_to_visible_row(tile, model, pointer.y, canvas_rect.top())
                .floor()
                .max(0.0) as usize;
            let position = if tile.config.ruler_cycles
                && tile
                    .config
                    .clock_period_ticks
                    .is_some_and(|period| period > 0)
            {
                pointer_position(tile, tick)
            } else {
                format_trace_tick(state, tile, tick.max(0.0).round() as u64)
            };
            physical_row(tile, model, visible).map_or_else(
                || format!("[{position}]"),
                |row| {
                    let stage = stage_at_pointer(tile, model, row, pointer, canvas_rect)
                        .map(|stage| {
                            let duration = stage.end.saturating_sub(stage.start);
                            let duration = tile
                                .config
                                .clock_period_ticks
                                .filter(|period| *period > 0)
                                .map_or_else(
                                    || duration.to_string(),
                                    |period| format_cycle_duration(duration, period),
                                );
                            format!("  {}[{duration}]", model.stage_name(&stage))
                        })
                        .unwrap_or_default();
                    format!("[{position}, ID {row}] {stage}")
                },
            )
        })
        .unwrap_or_default();
    let quality = if model.quality.total() > 0 {
        format!("  ⚠ {} data issues", model.quality.total())
    } else {
        String::new()
    };
    let focus = focused_row(state, tile, model).map_or_else(String::new, |row| {
        format!(
            "  focus ID {row} @ {}",
            if tile.config.ruler_cycles
                && tile
                    .config
                    .clock_period_ticks
                    .is_some_and(|period| period > 0)
            {
                pointer_position(tile, model.rows.begin[row] as f64)
            } else {
                format_trace_tick(state, tile, model.rows.begin[row])
            }
        )
    });
    let loading = state
        .konata_runtime
        .get(&tile_id)
        .filter(|runtime| {
            runtime
                .entry
                .as_ref()
                .is_some_and(|entry| !entry.is_complete())
        })
        .map_or_else(String::new, |runtime| {
            format!(
                "  loading {:.0}% (provisional)",
                runtime.model_progress * 100.0
            )
        });
    let status = format!(
        "{position}{focus}{loading}    zoom {:.2} px/tick × {:.2} px/row    {} ops ({} flushed){quality}",
        tile.viewport.px_per_tick,
        tile.viewport.row_height_px,
        model.row_count(),
        model.flushed_count,
    );
    ui.painter().text(
        Pos2::new(status_rect.left() + 7.0, status_rect.center().y),
        Align2::LEFT_CENTER,
        status,
        FontId::monospace(10.0),
        foreground,
    );
}

fn show_tooltip(
    ui: &Ui,
    tile: &KonataTileState,
    model: &KonataModel,
    response: &Response,
    label_rect: Rect,
    canvas_rect: Rect,
) {
    let Some(pointer) = ui.input(|input| input.pointer.hover_pos()) else {
        return;
    };
    if canvas_rect.contains(pointer) && effective_detail(tile) < f64::from(tile.config.color_lod_px)
    {
        let from = y_to_visible_row(tile, model, pointer.y.floor(), canvas_rect.top())
            .floor()
            .max(0.0) as usize;
        let to = y_to_visible_row(tile, model, pointer.y.floor() + 1.0, canvas_rect.top())
            .ceil()
            .min(visible_row_count(tile, model) as f64) as usize;
        if let Some((start, end, flushed, count)) =
            model.range_extent(from, to, tile.config.hide_flushed)
        {
            response.clone().on_hover_text(format!(
                "IDs {from}..{to}\ntime [{start}, {end})\n{count} operations, {flushed} flushed\nClick to zoom into this range"
            ));
        }
        return;
    }
    let Some(row) = row_at_pointer(tile, model, pointer, canvas_rect) else {
        return;
    };
    if label_rect.contains(pointer) {
        let text = instruction_tooltip_text(tile, model, row, None);
        response.clone().on_hover_ui(|ui| {
            if ui.small_button("Copy").clicked() {
                ui.ctx().copy_text(text.clone());
            }
            ui.monospace(&text);
        });
    } else if canvas_rect.contains(pointer) {
        let tick = tile.viewport.x_to_tick(pointer.x, canvas_rect.left());
        let detail = model.stages_for_row(row);
        let hits = detail
            .iter()
            .filter(|stage| {
                let tolerance = 3.0 / tile.viewport.px_per_tick;
                if stage.start == stage.end || stage.end < stage.start {
                    (tick - stage.start as f64).abs() <= tolerance
                } else {
                    tick >= stage.start as f64 && tick < stage.end as f64
                }
            })
            .collect::<Vec<_>>();
        let text = stage_tooltip_text(tile, model, row, tick, &hits);
        response.clone().on_hover_ui(|ui| {
            if ui.small_button("Copy").clicked() {
                ui.ctx().copy_text(text.clone());
            }
            ui.monospace(&text);
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn show_context_menu(
    msgs: &mut Vec<Message>,
    tile_id: KonataTileId,
    tile: &mut KonataTileState,
    model: &KonataModel,
    response: &Response,
    canvas_rect: Rect,
    range_selection: Option<(u64, u64)>,
    focused_row: Option<usize>,
    producer_chain_active: bool,
    bookmark_labels: &[(u8, String, bool)],
) {
    response.context_menu(|ui| {
        let pointer = response.interact_pointer_pos();
        let pointer_row =
            pointer.and_then(|pointer| row_at_pointer(tile, model, pointer, canvas_rect));
        let pointer_stages = pointer_row
            .zip(pointer)
            .map(|(row, pointer)| {
                let tick = tile.viewport.x_to_tick(pointer.x, canvas_rect.left());
                stages_at_tick(tile, model, row, tick).collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if ui.button("Adjust position").clicked() {
            let visible = tile
                .viewport
                .top_visible_row
                .round()
                .clamp(0.0, visible_row_count(tile, model).saturating_sub(1) as f64)
                as usize;
            if let Some(row) = physical_row(tile, model, visible) {
                tile.viewport.align_row(model.rows.begin[row], visible);
            }
            ui.close();
        }
        if ui.button("Fit whole trace").clicked() {
            let count = visible_row_count(tile, model);
            if let Some((start, end, _, _)) = model.range_extent(0, count, tile.config.hide_flushed)
            {
                tile.viewport.top_visible_row = 0.0;
                tile.viewport.row_height_px =
                    (f64::from(canvas_rect.height()) / count.max(1) as f64).max(0.000_001);
                tile.viewport.px_per_tick = (f64::from(canvas_rect.width())
                    / end.saturating_sub(start).max(1) as f64)
                    .max(0.000_001);
                tile.viewport
                    .set_left_tick(start.min(i64::MAX as u64) as i64);
            }
            ui.close();
        }
        if ui.button("Zoom in").clicked() {
            let pointer = pointer.unwrap_or(canvas_rect.center());
            zoom_at(
                tile,
                model,
                tile.config.zoom_step,
                f64::from(pointer.x - canvas_rect.left()),
                f64::from(pointer.y - canvas_rect.top()),
            );
            ui.close();
        }
        if ui.button("Zoom out").clicked() {
            let pointer = pointer.unwrap_or(canvas_rect.center());
            zoom_at(
                tile,
                model,
                1.0 / tile.config.zoom_step,
                f64::from(pointer.x - canvas_rect.left()),
                f64::from(pointer.y - canvas_rect.top()),
            );
            ui.close();
        }
        ui.menu_button("Go to bookmark", |ui| {
            for (slot, label, available) in bookmark_labels {
                if ui
                    .add_enabled(*available, egui::Button::new(label))
                    .clicked()
                {
                    msgs.push(Message::KonataBookmarkGoto(*slot));
                    ui.close();
                }
            }
        });
        ui.menu_button("Set bookmark", |ui| {
            for (slot, label, _) in bookmark_labels {
                if ui.button(label).clicked() {
                    msgs.push(Message::KonataBookmarkSet(*slot));
                    ui.close();
                }
            }
        });
        ui.separator();
        if let Some(row) = pointer_row {
            if ui.button("Focus instruction").clicked() {
                focus_transaction(msgs, tile, model.rows.tx_id[row]);
                ui.close();
            }
            if ui.button("Copy label").clicked() {
                let label = model.rows.label(row).map_or_else(
                    || format!("tx#{}", model.rows.tx_id[row]),
                    |label| model.string(label).to_string(),
                );
                ui.ctx().copy_text(label);
                ui.close();
            }
            if ui.button("Copy row as text").clicked() {
                ui.ctx().copy_text(row_label(model, row));
                ui.close();
            }
            if ui.button("Show in event table").clicked() {
                msgs.push(Message::OpenKonataEventTable {
                    tile_id,
                    parent_tx: Some(model.rows.tx_id[row]),
                });
                ui.close();
            }
            if ui.button("Show in waveform view").clicked() {
                focus_transaction(msgs, tile, model.rows.tx_id[row]);
                msgs.push(Message::CursorSet(BigInt::from(model.rows.begin[row])));
                ui.close();
            }
        }
        if !pointer_stages.is_empty() {
            ui.menu_button("Stages here", |ui| {
                for stage in pointer_stages {
                    ui.menu_button(
                        format!(
                            "{} [{}, {})",
                            model.stage_name(&stage),
                            stage.start,
                            stage.end
                        ),
                        |ui| {
                            if ui.button("Focus stage event").clicked() {
                                focus_transaction(msgs, tile, stage.event_tx);
                                ui.close();
                            }
                            if ui.button("Move cursor to start").clicked() {
                                msgs.push(Message::CursorSet(BigInt::from(stage.start)));
                                ui.close();
                            }
                            if ui.button("Move cursor to end").clicked() {
                                msgs.push(Message::CursorSet(BigInt::from(stage.end)));
                                ui.close();
                            }
                        },
                    );
                }
            });
        }
        if ui.button("Clear focus").clicked() {
            msgs.push(Message::FocusTransactionFromSource(None, None));
            ui.close();
        }
        if let Some(range) = range_selection.filter(|range| range.0 < range.1)
            && ui.button("Statistics for selection").clicked()
        {
            msgs.push(Message::OpenKonataRangeStatistics { tile_id, range });
            ui.close();
        }
        if let Some(row) = focused_row
            && ui
                .button(if producer_chain_active {
                    "Clear producer chain"
                } else {
                    "Highlight producer chain"
                })
                .clicked()
        {
            msgs.push(Message::ToggleKonataProducerChain { tile_id, row });
            ui.close();
        }
        ui.separator();
        display_options(ui, tile, model);
        if let Some(pointer) = pointer.filter(|pointer| canvas_rect.contains(*pointer)) {
            let tick = tile
                .viewport
                .x_to_tick(pointer.x, canvas_rect.left())
                .max(0.0)
                .round() as u64;
            if ui.button("Set marker here").clicked() {
                msgs.push(Message::AddMarker {
                    time: BigInt::from(tick),
                    name: None,
                    move_focus: true,
                });
                ui.close();
            }
        }
    });
}

fn focused_row(state: &SystemState, tile: &KonataTileState, model: &KonataModel) -> Option<usize> {
    let focused = focused_transaction_id(state, tile)?;
    model
        .row_for_transaction(focused)
        .or_else(|| model.row_for_event(focused))
}

fn focused_transaction_id(state: &SystemState, tile: &KonataTileState) -> Option<u64> {
    let focused = state.user.waves.as_ref()?.focused_transaction.0.as_ref()?;
    (focused.source == tile.spec.source).then_some(focused.inner.id.0)
}

fn focus_transaction(msgs: &mut Vec<Message>, tile: &KonataTileState, tx_id: u64) {
    let tx_ref = TransactionRef {
        id: TransactionId(tx_id),
    };
    msgs.push(Message::FocusTransactionFromSource(
        Some(SourceTransactionRef::new(tile.spec.source, tx_ref)),
        None,
    ));
}

fn synchronize_external_focus(
    state: &mut SystemState,
    tile_id: KonataTileId,
    tile: &mut KonataTileState,
    model: &KonataModel,
) {
    let focused = focused_transaction_id(state, tile);
    let animations_enabled = state.animation_enabled();
    let animation_duration = state.user.config.animation_time.clamp(0.08, 0.10);
    let runtime = state.konata_runtime.entry(tile_id).or_default();
    if focused == runtime.last_focused_tx {
        return;
    }
    let suppress = focused.is_some() && focused == runtime.suppress_focus_scroll;
    runtime.last_focused_tx = focused;
    runtime.suppress_focus_scroll = None;
    if suppress {
        return;
    }
    let Some(row) = focused.and_then(|tx| {
        model
            .row_for_transaction(tx)
            .or_else(|| model.row_for_event(tx))
    }) else {
        return;
    };
    let visible = visible_index_of(tile, model, row);
    let mut target = tile.viewport;
    target.align_row(model.rows.begin[row], visible);
    if animations_enabled
        && state.user.config.animation_time > 0.0
        && runtime.canvas_size != Vec2::ZERO
    {
        runtime.viewport_motion = Some(super::KonataViewportMotion {
            start: tile.viewport,
            target,
            elapsed: 0.0,
            duration: animation_duration,
        });
    } else {
        tile.viewport = target;
    }
}

fn stage_at_pointer(
    tile: &KonataTileState,
    model: &KonataModel,
    row: usize,
    pointer: Pos2,
    canvas_rect: Rect,
) -> Option<KonataStage> {
    let tick = tile.viewport.x_to_tick(pointer.x, canvas_rect.left());
    stages_at_tick(tile, model, row, tick).last()
}

fn stages_at_tick(
    tile: &KonataTileState,
    model: &KonataModel,
    row: usize,
    tick: f64,
) -> impl Iterator<Item = KonataStage> {
    let tolerance = 3.0 / tile.viewport.px_per_tick;
    model
        .stages_for_row(row)
        .iter()
        .filter(move |stage| {
            if stage.start == stage.end || stage.end < stage.start {
                (tick - stage.start as f64).abs() <= tolerance
            } else {
                tick >= stage.start as f64 && tick < stage.end as f64
            }
        })
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
}

fn row_at_pointer(
    tile: &KonataTileState,
    model: &KonataModel,
    pointer: Pos2,
    canvas_rect: Rect,
) -> Option<usize> {
    let visible = y_to_visible_row(tile, model, pointer.y, canvas_rect.top()).floor();
    (visible >= 0.0)
        .then(|| physical_row(tile, model, visible as usize))
        .flatten()
}

fn visible_row_count(tile: &KonataTileState, model: &KonataModel) -> usize {
    if tile.config.hide_flushed {
        model.visibility.visible_count()
    } else {
        model.row_count()
    }
}

fn physical_row(tile: &KonataTileState, model: &KonataModel, visible: usize) -> Option<usize> {
    if tile.config.hide_flushed {
        model.visibility.select(visible)
    } else {
        (visible < model.row_count()).then_some(visible)
    }
}

fn visible_index_of(tile: &KonataTileState, model: &KonataModel, row: usize) -> usize {
    if tile.config.hide_flushed {
        model.visibility.rank(row)
    } else {
        row
    }
}

fn row_pitch(tile: &KonataTileState, model: &KonataModel) -> f64 {
    let lanes = if tile.config.lane_mode == KonataLaneMode::SplitNatural {
        model.lane_names.len().max(1) as f64
    } else {
        1.0
    };
    tile.viewport.row_height_px * lanes
}

fn effective_detail(tile: &KonataTileState) -> f64 {
    let cycle_width = tile
        .config
        .clock_period_ticks
        .filter(|period| *period > 0)
        .map_or(tile.viewport.px_per_tick, |period| {
            tile.viewport.px_per_tick * period as f64
        });
    tile.viewport.row_height_px.min(cycle_width)
}

fn visible_row_to_y(tile: &KonataTileState, model: &KonataModel, row: f64, canvas_top: f32) -> f32 {
    canvas_top + ((row - tile.viewport.top_visible_row) * row_pitch(tile, model)) as f32
}

fn y_to_visible_row(tile: &KonataTileState, model: &KonataModel, y: f32, canvas_top: f32) -> f64 {
    tile.viewport.top_visible_row + f64::from(y - canvas_top) / row_pitch(tile, model)
}

fn zoom_at(
    tile: &mut KonataTileState,
    model: &KonataModel,
    factor: f64,
    anchor_x: f64,
    anchor_y: f64,
) {
    let lane_factor = if tile.config.lane_mode == KonataLaneMode::SplitNatural {
        model.lane_names.len().max(1) as f64
    } else {
        1.0
    };
    tile.viewport
        .zoom_at(factor, anchor_x, anchor_y / lane_factor);
}

fn row_label(model: &KonataModel, row: usize) -> String {
    let mut identity = row.to_string();
    if let Some(sid) = model.rows.sid(row) {
        identity.push_str(&format!(": s{sid}"));
    }
    let tid = model.rows.tid(row);
    let rid = model.rows.rid(row);
    if tid.is_some() || rid.is_some() || model.rows.flushed[row] == FlushState::True {
        identity.push_str(" (");
        if let Some(tid) = tid {
            identity.push('t');
            identity.push_str(model.thread_name(tid));
            if rid.is_some() || model.rows.flushed[row] == FlushState::True {
                identity.push_str(": ");
            }
        }
        if let Some(rid) = rid {
            identity.push_str(&format!("r{rid}"));
        } else {
            identity.push_str("r—");
        }
        identity.push(')');
    }
    identity.push_str(": ");
    if let Some(label) = model.rows.label(row) {
        identity.push_str(model.string(label));
    } else {
        identity.push_str(&format!("tx#{}", model.rows.tx_id[row]));
    }
    if model.rows.flags[row].0 != 0 {
        identity.push_str(" ⚠");
    }
    identity
}

fn pointer_position(tile: &KonataTileState, tick: f64) -> String {
    if tile.config.ruler_cycles
        && let Some(period) = tile.config.clock_period_ticks.filter(|period| *period > 0)
    {
        let cycle = (tick - tile.config.clock_origin_tick as f64) / period as f64;
        format!("cycle {cycle:.2}")
    } else {
        format!("time {tick:.2}")
    }
}

fn format_cycle_duration(duration: u64, period: u64) -> String {
    if duration.is_multiple_of(period) {
        (duration / period).to_string()
    } else {
        format!("{:.2} cy", duration as f64 / period as f64)
    }
}

fn shift_tick(tick: u64, shift: i128) -> u64 {
    (i128::from(tick) + shift).clamp(0, i128::from(u64::MAX)) as u64
}

fn execution_tick(tile: &KonataTileState, model: &KonataModel, row: usize) -> Option<u64> {
    model
        .stages_for_row(row)
        .iter()
        .find(|stage| {
            comma_list_matches(&tile.config.execution_stages, model.stage_name(stage), true)
        })
        .map(|stage| stage.start)
}

fn comma_list_matches(list: &str, value: &str, case_sensitive: bool) -> bool {
    list.split(',').map(str::trim).any(|candidate| {
        if case_sensitive {
            candidate == value
        } else {
            candidate.eq_ignore_ascii_case(value)
        }
    })
}

fn stage_color(
    tile: &KonataTileState,
    model: &KonataModel,
    row: usize,
    stage: &KonataStage,
    theme: &crate::config::KonataTheme,
    canvas_background: Color32,
) -> Color32 {
    let name = model.stage_name(stage);
    let color = if comma_list_matches(
        &tile.config.stall_stages,
        name,
        tile.config.stall_case_sensitive,
    ) {
        theme.stall
    } else {
        match tile.config.color_scheme {
            KonataColorScheme::Orange => theme.flat_orange,
            KonataColorScheme::RoyalBlue => theme.flat_royal_blue,
            KonataColorScheme::Thread => {
                let tid = model.rows.tid(row).unwrap_or_default() as usize;
                let base = palette_color(&theme.stage_palette, tid, theme.focus);
                multiply(base, 0.82 + f32::from(stage.name % 4) * 0.06)
            }
            KonataColorScheme::Auto => palette_color(
                &theme.stage_palette,
                stage.name as usize + usize::from(stage.lane) * 3,
                theme.focus,
            ),
            KonataColorScheme::Unique => {
                let hash = name.bytes().fold(0usize, |hash, byte| {
                    hash.wrapping_mul(16777619) ^ byte as usize
                });
                palette_color(&theme.stage_palette, hash, theme.focus)
            }
            KonataColorScheme::ColorBlindSafe => palette_color(
                &theme.color_blind_palette,
                stage.name as usize + usize::from(stage.lane),
                theme.focus,
            ),
            KonataColorScheme::Custom => tile
                .config
                .custom_color_schemes
                .get(&tile.config.custom_color_scheme)
                .map(|scheme| custom_hsl_color(scheme, model, stage))
                .or_else(|| {
                    tile.config
                        .custom_stage_colors
                        .get(name)
                        .map(|color| Color32::from_rgb(color[0], color[1], color[2]))
                })
                .unwrap_or_else(|| {
                    palette_color(&theme.stage_palette, stage.name as usize, theme.focus)
                }),
        }
    };
    clamp_contrast(color, canvas_background, theme.minimum_contrast)
}

fn custom_hsl_color(
    scheme: &super::KonataCustomColorScheme,
    model: &KonataModel,
    stage: &KonataStage,
) -> Color32 {
    let name = model.stage_name(stage);
    let lane = model.lane_name(stage.lane);
    let spec = scheme
        .stages
        .get(name)
        .or_else(|| scheme.lanes.get(lane))
        .unwrap_or(&scheme.default);
    let automatic_hue =
        (f32::from(stage.name) * 57.295_78 + f32::from(stage.lane) * 31.0).rem_euclid(360.0);
    let hue = spec.hue.resolve(automatic_hue).rem_euclid(360.0);
    let saturation = spec.saturation.resolve(0.62).clamp(0.0, 1.0);
    let lightness = spec.lightness.resolve(0.52).clamp(0.0, 1.0);
    hsl_to_rgb(hue, saturation, lightness)
}

fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> Color32 {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let segment = hue / 60.0;
    let secondary = chroma * (1.0 - (segment.rem_euclid(2.0) - 1.0).abs());
    let (red, green, blue) = match segment.floor() as i32 {
        0 => (chroma, secondary, 0.0),
        1 => (secondary, chroma, 0.0),
        2 => (0.0, chroma, secondary),
        3 => (0.0, secondary, chroma),
        4 => (secondary, 0.0, chroma),
        _ => (chroma, 0.0, secondary),
    };
    let offset = lightness - chroma * 0.5;
    Color32::from_rgb(
        ((red + offset) * 255.0).round() as u8,
        ((green + offset) * 255.0).round() as u8,
        ((blue + offset) * 255.0).round() as u8,
    )
}

fn palette_color(palette: &[Color32], index: usize, fallback: Color32) -> Color32 {
    palette
        .get(index % palette.len().max(1))
        .copied()
        .unwrap_or(fallback)
}

fn clamp_contrast(color: Color32, background: Color32, minimum: f32) -> Color32 {
    if contrast_ratio(color, background) >= minimum.max(1.0) {
        return color;
    }
    let black = Color32::BLACK;
    let white = Color32::WHITE;
    let target = if contrast_ratio(white, background) >= contrast_ratio(black, background) {
        white
    } else {
        black
    };
    (1..=16)
        .map(|step| mix_color(color, target, step as f32 / 16.0))
        .find(|candidate| contrast_ratio(*candidate, background) >= minimum.max(1.0))
        .unwrap_or(target)
}

fn mix_color(from: Color32, to: Color32, factor: f32) -> Color32 {
    let mix = |from: u8, to: u8| {
        (f32::from(from) + (f32::from(to) - f32::from(from)) * factor).round() as u8
    };
    Color32::from_rgba_unmultiplied(
        mix(from.r(), to.r()),
        mix(from.g(), to.g()),
        mix(from.b(), to.b()),
        from.a(),
    )
}

fn contrast_ratio(left: Color32, right: Color32) -> f32 {
    let left = relative_luminance(left);
    let right = relative_luminance(right);
    (left.max(right) + 0.05) / (left.min(right) + 0.05)
}

fn relative_luminance(color: Color32) -> f32 {
    let channel = |value: u8| {
        let value = f32::from(value) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
}

fn multiply(color: Color32, factor: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        (f32::from(color.r()) * factor).clamp(0.0, 255.0) as u8,
        (f32::from(color.g()) * factor).clamp(0.0, 255.0) as u8,
        (f32::from(color.b()) * factor).clamp(0.0, 255.0) as u8,
        color.a(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        konata::{KonataBuildInput, KonataModelSpec, KonataRuntimeState},
        transaction_container::TransactionStreamRef,
        transaction_events::test_util::{ftr_from_json, generator, stream, tx},
    };
    use ftr_parser::types::{GeneratorId, StreamId};
    use serde_json::json;

    #[test]
    fn adaptive_tick_steps_use_one_two_five_decades() {
        assert_eq!(nice_tick_step(0.1), 1.0);
        assert_eq!(nice_tick_step(11.0), 20.0);
        assert_eq!(nice_tick_step(21.0), 50.0);
        assert_eq!(nice_tick_step(51.0), 100.0);
    }

    #[test]
    fn cycle_duration_preserves_fractional_recorded_time() {
        assert_eq!(format_cycle_duration(12, 4), "3");
        assert_eq!(format_cycle_duration(10, 4), "2.50 cy");
    }

    #[test]
    fn effective_detail_uses_the_configured_cycle_width() {
        let mut tile = KonataTileState {
            spec: KonataModelSpec {
                source: Default::default(),
                generator: TransactionStreamRef::new_gen(
                    StreamId(1),
                    GeneratorId(10),
                    "instruction".to_string(),
                ),
            },
            title: String::new(),
            config: Default::default(),
            viewport: Default::default(),
        };
        tile.viewport.row_height_px = 24.0;
        tile.viewport.px_per_tick = 0.5;
        assert_eq!(effective_detail(&tile), 0.5);

        tile.config.clock_period_ticks = Some(8);
        assert_eq!(effective_detail(&tile), 4.0);
    }

    #[test]
    fn consecutive_drag_frame_deltas_pan_freely_in_both_axes() {
        let mut viewport = super::super::KonataViewport {
            left_tick: 100,
            px_per_tick: 2.0,
            top_visible_row: 20.0,
            row_height_px: 24.0,
            ..Default::default()
        };

        pan_viewport_by_drag_delta(&mut viewport, Vec2::new(8.0, 12.0), 24.0);
        pan_viewport_by_drag_delta(&mut viewport, Vec2::new(8.0, 12.0), 24.0);

        assert_eq!(viewport.left_value(), 92.0);
        assert_eq!(viewport.top_visible_row, 19.0);

        pan_viewport_by_drag_delta(&mut viewport, Vec2::new(-24.0, -48.0), 24.0);

        assert_eq!(viewport.left_value(), 104.0);
        assert_eq!(viewport.top_visible_row, 21.0);
    }

    #[test]
    fn scroll_modifiers_select_horizontal_or_two_axis_zoom() {
        let alt = egui::Modifiers {
            alt: true,
            ..Default::default()
        };
        let alt_gesture = resolve_scroll_gesture(Vec2::new(0.0, 40.0), 1.0, alt, 1.0 / 200.0);
        let Some(ScrollGesture::ZoomBoth(alt_factor)) = alt_gesture else {
            panic!("Alt + scroll should zoom both axes");
        };
        assert!((alt_factor - 0.2_f64.exp()).abs() < 1e-6);
        let Some(ScrollGesture::ZoomBoth(alt_reverse_factor)) =
            resolve_scroll_gesture(Vec2::new(0.0, -40.0), 1.0, alt, 1.0 / 200.0)
        else {
            panic!("reverse Alt + scroll should zoom both axes");
        };
        assert!((alt_factor * alt_reverse_factor - 1.0).abs() < 1e-6);

        let command = egui::Modifiers {
            command: true,
            ..Default::default()
        };
        assert_eq!(
            resolve_scroll_gesture(Vec2::ZERO, 1.25, command, 1.0 / 200.0),
            Some(ScrollGesture::ZoomX(1.25))
        );

        assert_eq!(
            resolve_scroll_gesture(Vec2::ZERO, 1.25, Default::default(), 1.0 / 200.0),
            Some(ScrollGesture::ZoomBoth(1.25))
        );
    }

    #[test]
    fn contrast_clamp_meets_theme_target() {
        let background = Color32::from_rgb(30, 32, 35);
        let clamped = clamp_contrast(Color32::from_rgb(35, 36, 38), background, 4.5);
        assert!(contrast_ratio(clamped, background) >= 4.5);
        let already_visible = Color32::from_rgb(230, 220, 210);
        assert_eq!(
            clamp_contrast(already_visible, background, 3.0),
            already_visible
        );
    }

    #[test]
    fn hsl_custom_colors_resolve_primary_hues() {
        assert_eq!(hsl_to_rgb(0.0, 1.0, 0.5), Color32::from_rgb(255, 0, 0));
        assert_eq!(hsl_to_rgb(120.0, 1.0, 0.5), Color32::from_rgb(0, 255, 0));
        assert_eq!(hsl_to_rgb(240.0, 1.0, 0.5), Color32::from_rgb(0, 0, 255));
        assert_eq!(
            super::super::KonataHslComponent::default().resolve(42.0),
            42.0
        );
    }

    #[test]
    fn dependency_walk_cycles_equal_time_ties_in_row_order() {
        let mut consumer = tx(3, 10, 2, 10, None, &[]);
        consumer["inc_relations"] = json!([
            {"name":"wakeup", "source_tx_id":1, "sink_tx_id":3, "source_stream_id":1, "sink_stream_id":1},
            {"name":"wakeup", "source_tx_id":2, "sink_tx_id":3, "source_stream_id":1, "sink_stream_id":1}
        ]);
        let ftr = ftr_from_json(
            json!({"1": stream(1, "cpu", &[10, 11])}),
            json!({
                "10": generator(10, 1, "instruction", json!([
                    tx(1, 10, 0, 8, None, &[]),
                    tx(2, 10, 0, 8, None, &[]),
                    consumer,
                ])),
                "11": generator(11, 1, "instruction.events", json!([])),
            }),
        );
        let model = KonataModel::build(KonataBuildInput {
            parent_generator: GeneratorId(10),
            event_generator: GeneratorId(11),
            stream: StreamId(1),
            parents: ftr
                .get_generator(GeneratorId(10))
                .unwrap()
                .transactions
                .clone(),
            events: ftr
                .get_generator(GeneratorId(11))
                .unwrap()
                .transactions
                .clone(),
            relations: ftr.tx_relations.clone(),
        });
        let tile = KonataTileState {
            spec: KonataModelSpec {
                source: Default::default(),
                generator: TransactionStreamRef::new_gen(
                    StreamId(1),
                    GeneratorId(10),
                    "instruction".to_string(),
                ),
            },
            title: String::new(),
            config: Default::default(),
            viewport: Default::default(),
        };
        let mut runtime = KonataRuntimeState::default();
        assert_eq!(
            dependency_walk_target(&tile, &model, &mut runtime, 2, false),
            Some(0)
        );
        assert_eq!(
            dependency_walk_target(&tile, &model, &mut runtime, 0, false),
            Some(1)
        );
        assert_eq!(
            dependency_walk_target(&tile, &model, &mut runtime, 1, false),
            Some(0)
        );
    }
}
