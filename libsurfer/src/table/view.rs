use crate::SystemState;
use crate::message::Message;
use crate::table::{
    FilterDraft, PendingScrollOp, ScrollTarget, TableCache, TableCacheKey, TableCell,
    TableColumnKey, TableModel, TableModelSpec, TableRuntimeState, TableSchema, TableSearchMode,
    TableSearchSpec, TableSelection, TableSelectionMode, TableSortSpec, TableTileId,
    TableTileState, TableViewConfig, find_type_search_match_in_cache, format_selection_count,
    hidden_columns, is_default_search_spec, navigate_down, navigate_end, navigate_extend_selection,
    navigate_home, navigate_page_down, navigate_page_up, navigate_up, normalize_search_specs,
    scroll_target_after_filter, scroll_target_after_sort, selection_on_click_multi,
    selection_on_click_single, selection_on_ctrl_click, selection_on_shift_click,
    should_clear_selection_on_generation_change, sort_indicator, sort_spec_on_click,
    sort_spec_on_shift_click, visible_columns,
};
use crate::wave_container::VariableRefExt;
use egui_extras::{Column, TableBuilder};
use regex::RegexBuilder;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Default row height for normal table rows.
const ROW_HEIGHT_NORMAL: f32 = 20.0;
/// Row height for dense mode (smaller text, less padding).
const ROW_HEIGHT_DENSE: f32 = 16.0;
/// Blend ratio for header hover background = 3/5 base + 2/5 accent.
const HEADER_HOVER_BASE_WEIGHT: u16 = 3;
const HEADER_HOVER_ACCENT_WEIGHT: u16 = 2;

fn blend_channel(base: u8, accent: u8) -> u8 {
    let total = HEADER_HOVER_BASE_WEIGHT + HEADER_HOVER_ACCENT_WEIGHT;
    ((u16::from(base) * HEADER_HOVER_BASE_WEIGHT + u16::from(accent) * HEADER_HOVER_ACCENT_WEIGHT)
        / total) as u8
}

fn header_hover_background_color(
    header_bg: egui::Color32,
    selection_bg: egui::Color32,
) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(
        blend_channel(header_bg.r(), selection_bg.r()),
        blend_channel(header_bg.g(), selection_bg.g()),
        blend_channel(header_bg.b(), selection_bg.b()),
        blend_channel(header_bg.a(), selection_bg.a()),
    )
}

fn is_light_color(color: egui::Color32) -> bool {
    // Perceived luminance approximation for quick theme-mode detection.
    let luminance =
        (u32::from(color.r()) * 299 + u32::from(color.g()) * 587 + u32::from(color.b()) * 114)
            / 1000;
    luminance >= 128
}

fn header_cell_interaction(
    ui: &mut egui::Ui,
    cell_rect: egui::Rect,
    column_key: &TableColumnKey,
) -> egui::Response {
    ui.interact(
        cell_rect,
        ui.id().with(("table_header_cell", column_key)),
        egui::Sense::click(),
    )
}

fn primary_click_modifiers_for_rect(
    events: &[egui::Event],
    rect: egui::Rect,
    fallback: egui::Modifiers,
) -> egui::Modifiers {
    events
        .iter()
        .rev()
        .find_map(|event| match event {
            egui::Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers,
            } if *button == egui::PointerButton::Primary && !pressed && rect.contains(*pos) => {
                Some(*modifiers)
            }
            _ => None,
        })
        .unwrap_or(fallback)
}

fn header_click_modifiers(response: &egui::Response) -> egui::Modifiers {
    response
        .ctx
        .input(|i| primary_click_modifiers_for_rect(&i.events, response.interact_rect, i.modifiers))
}

fn column_key_string(key: &TableColumnKey) -> String {
    match key {
        TableColumnKey::Str(value) => value.clone(),
        TableColumnKey::Id(value) => value.to_string(),
    }
}

fn full_column_label(column: &crate::table::TableColumn) -> String {
    if column.label.is_empty() {
        column_key_string(&column.key)
    } else {
        column.label.clone()
    }
}

fn split_label_tokens(label: &str) -> Vec<&str> {
    label.split('.').collect()
}

fn suffix_candidates(label: &str) -> Vec<String> {
    let tokens = split_label_tokens(label);
    (1..=tokens.len())
        .map(|suffix_len| tokens[tokens.len() - suffix_len..].join("."))
        .collect()
}

fn compact_unique_labels(
    visible_columns: &[(TableColumnKey, String)],
) -> HashMap<TableColumnKey, String> {
    let suffix_counts = visible_columns
        .iter()
        .flat_map(|(_, full_label)| suffix_candidates(full_label))
        .fold(HashMap::new(), |mut counts, suffix| {
            *counts.entry(suffix).or_insert(0usize) += 1;
            counts
        });

    visible_columns
        .iter()
        .map(|(key, full_label)| {
            let chosen = suffix_candidates(full_label)
                .into_iter()
                .find(|candidate| suffix_counts.get(candidate) == Some(&1))
                .unwrap_or_else(|| full_label.clone());
            let compact = if chosen != *full_label {
                format!(":{chosen}")
            } else {
                chosen
            };
            (key.clone(), compact)
        })
        .collect()
}

fn fit_render_label<F>(
    compact_label: &str,
    available_width: f32,
    indicator_reserved_width: f32,
    measure_width: F,
) -> String
where
    F: Fn(&str) -> f32,
{
    let label_width = (available_width - indicator_reserved_width).max(0.0);
    if measure_width(compact_label) <= label_width {
        return compact_label.to_string();
    }

    let trimmed = compact_label.strip_prefix(':').unwrap_or(compact_label);
    for (start, _) in trimmed.char_indices() {
        let candidate = format!(":{}", &trimmed[start..]);
        if measure_width(&candidate) <= label_width {
            return candidate;
        }
    }

    if measure_width(":") <= label_width {
        ":".to_string()
    } else {
        String::new()
    }
}

/// Renders a table tile in the UI.
///
/// This function handles the full lifecycle of table rendering:
/// - If no tile config exists, shows an error message
/// - If cache is not ready, shows a loading indicator and triggers cache build
/// - If cache has an error, shows the error message
/// - Otherwise, renders the table using egui_extras::TableBuilder
///
/// Note: `table_tiles` is passed separately because it's temporarily moved out of
/// `state.user` during the tile tree rendering to avoid borrow conflicts.
pub fn draw_table_tile(
    state: &mut SystemState,
    _ctx: &egui::Context,
    ui: &mut egui::Ui,
    msgs: &mut Vec<Message>,
    tile_id: TableTileId,
    table_tiles: &HashMap<TableTileId, TableTileState>,
) {
    let Some(tile_state) = table_tiles.get(&tile_id) else {
        ui.centered_and_justified(|ui| {
            ui.label("Table tile not found");
        });
        return;
    };

    let title = tile_state.config.title.clone();
    let display_filter = tile_state.config.display_filter.clone();
    let view_sort = tile_state.config.sort.clone();
    let dense_rows = tile_state.config.dense_rows;
    let sticky_header = tile_state.config.sticky_header;
    let show_signal_analysis_actions = matches!(
        &tile_state.spec,
        TableModelSpec::AnalysisResults {
            kind: crate::table::AnalysisKind::SignalAnalysisV1,
            params: crate::table::AnalysisParams::SignalAnalysisV1 { .. },
        }
    );

    // Get or create runtime state
    let runtime = state.table_runtime.entry(tile_id).or_default();

    // Get current generation from wave data (0 if no wave data loaded)
    let current_generation = state.user.waves.as_ref().map_or(0, |w| w.cache_generation);

    // Check if generation changed and clear selection/model if so
    let last_generation = runtime.scroll_state.last_generation;
    if should_clear_selection_on_generation_change(current_generation, last_generation) {
        runtime.selection.clear();
        runtime.model = None;
        runtime.hidden_selection_count = 0;
        runtime.scroll_state.last_generation = current_generation;
    }

    // Compute current cache key
    let cache_key = TableCacheKey {
        model_key: tile_state.spec.model_key_for_tile(tile_id),
        display_filter,
        pinned_filters: tile_state.config.pinned_filters.clone(),
        view_sort,
        generation: current_generation,
    };

    // Check if we need to request a cache build
    let cache_ready = runtime
        .cache
        .as_ref()
        .is_some_and(|entry| entry.is_ready() && entry.cache_key == cache_key);

    let needs_build = !cache_ready && runtime.cache_key.as_ref() != Some(&cache_key);

    if needs_build {
        msgs.push(Message::BuildTableCache {
            tile_id,
            cache_key: cache_key.clone(),
        });
    }

    // Get theme colors
    let theme = &state.user.config.theme;
    let header_bg = theme.secondary_ui_color.background;
    let text_color = theme.foreground;
    let selection_bg = theme.selected_elements_colors.background;
    let use_light_table_visuals = is_light_color(theme.primary_ui_color.background);

    // Fill the full tile rect in light themes so transparent table rows inherit
    // theme background instead of the renderer clear color.
    if use_light_table_visuals {
        ui.painter().rect_filled(ui.max_rect(), 0.0, header_bg);
    }

    // Get selection mode from config
    let selection_mode = tile_state.config.selection_mode;

    // Create a unique ID for the table area to track focus
    let table_area_id = egui::Id::new(("table_area", tile_id.0));

    // Render UI based on current state
    let table_response = ui
        .vertical(|ui| {
            ui.horizontal(|ui| {
                ui.heading(&title);
                if show_signal_analysis_actions {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Refresh analysis").clicked() {
                            msgs.push(Message::RefreshSignalAnalysis { tile_id });
                        }
                        if ui.small_button("Edit configuration...").clicked() {
                            msgs.push(Message::EditSignalAnalysis { tile_id });
                        }
                    });
                }
            });
            ui.separator();

            // Always render filter bar first (bound to draft state for focus preservation)
            let filter_schema = state
                .table_runtime
                .get(&tile_id)
                .and_then(|runtime| runtime.model.as_ref().map(|model| model.schema()));
            {
                let runtime = state.table_runtime.entry(tile_id).or_default();
                render_filter_bar(
                    ui,
                    msgs,
                    tile_id,
                    runtime,
                    &tile_state.config,
                    filter_schema.as_ref(),
                );
            }

            // Check debounce AFTER render_filter_bar updates draft (avoids stale filter apply)
            check_filter_debounce(state, tile_id, &tile_state.config.display_filter, msgs);

            // Request repaint while draft is dirty (for debounce timer)
            if let Some(runtime) = state.table_runtime.get(&tile_id)
                && let Some(draft) = &runtime.filter_draft
                && draft.is_dirty(&tile_state.config.display_filter)
            {
                ui.ctx().request_repaint_after(Duration::from_millis(50));
            }

            // Re-get runtime state after potential mutation
            let runtime = state.table_runtime.get(&tile_id);

            if let Some(runtime) = runtime {
                if let Some(error) = &runtime.last_error {
                    // Show error state
                    ui.colored_label(egui::Color32::RED, format!("Error: {error:?}"));
                } else if let Some(cache_entry) = &runtime.cache
                    && cache_entry.is_ready()
                    && cache_entry.cache_key == cache_key
                {
                    // Cache is ready - render the table
                    if let Some(cache) = cache_entry.get() {
                        if let Some(model) = runtime.model.clone() {
                            let schema = model.schema();
                            // Get current selection for rendering
                            let selection = runtime.selection.clone();
                            let type_search_buffer = runtime.type_search.buffer.clone();

                            // Process pending scroll operations
                            let pending_op = runtime.scroll_state.pending_scroll_op;
                            let scroll_target =
                                runtime.scroll_state.scroll_target.clone().or_else(|| {
                                    pending_op.map(|op| match op {
                                        PendingScrollOp::AfterSort => scroll_target_after_sort(
                                            &selection,
                                            &cache.row_ids,
                                            &cache.row_index,
                                        ),
                                        PendingScrollOp::AfterFilter => scroll_target_after_filter(
                                            &selection,
                                            &cache.row_ids,
                                            &cache.row_index,
                                        ),
                                        PendingScrollOp::AfterActivation(row) => {
                                            ScrollTarget::ToRow(row)
                                        }
                                    })
                                });

                            // Render column visibility toggle
                            let columns_config = &tile_state.config.columns;
                            let hidden_cols = hidden_columns(columns_config);
                            if !hidden_cols.is_empty() {
                                render_column_visibility_bar(
                                    ui,
                                    msgs,
                                    tile_id,
                                    &hidden_cols,
                                    &schema,
                                );
                            }

                            // Show type-to-search indicator if active
                            if !type_search_buffer.is_empty() {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "Search: {type_search_buffer}"
                                        ))
                                        .italics()
                                        .color(egui::Color32::GRAY),
                                    );
                                });
                            }

                            render_table(
                                ui,
                                msgs,
                                tile_id,
                                table_area_id,
                                model,
                                &schema,
                                cache,
                                &tile_state.config.sort,
                                &tile_state.config.columns,
                                &selection,
                                selection_mode,
                                dense_rows,
                                sticky_header,
                                header_bg,
                                text_color,
                                selection_bg,
                                use_light_table_visuals,
                                scroll_target.as_ref(),
                            );
                        } else {
                            ui.label("Model not available");
                        }
                    }
                } else {
                    // Loading state - filter bar is already shown above
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Filtering...");
                    });
                }
            } else {
                ui.label("Initializing...");
            }
        })
        .response;

    // Make the table area focusable and handle clicks for focus
    let table_response = table_response.interact(egui::Sense::click());
    if table_response.clicked() {
        ui.memory_mut(|mem| mem.request_focus(table_area_id));
    }

    // Handle keyboard events when the table has focus
    let has_focus = ui.memory(|mem| mem.has_focus(table_area_id));
    if has_focus {
        handle_keyboard_navigation(state, ui, msgs, tile_id, selection_mode, table_tiles);
    }

    // Clear pending scroll operations after rendering
    if let Some(runtime) = state.table_runtime.get_mut(&tile_id) {
        runtime.scroll_state.pending_scroll_op = None;
        runtime.scroll_state.scroll_target = None;
    }
}

/// Handles keyboard navigation for the table.
fn handle_keyboard_navigation(
    state: &mut SystemState,
    ui: &mut egui::Ui,
    msgs: &mut Vec<Message>,
    tile_id: TableTileId,
    selection_mode: TableSelectionMode,
    _table_tiles: &HashMap<TableTileId, TableTileState>,
) {
    // Calculate page size based on visible area (approximate)
    let page_size = 20; // Default page size; could be calculated from UI height

    // Collect keyboard input (does not borrow state)
    let input = ui.input(|i| {
        (
            i.modifiers,
            i.key_pressed(egui::Key::ArrowUp),
            i.key_pressed(egui::Key::ArrowDown),
            i.key_pressed(egui::Key::PageUp),
            i.key_pressed(egui::Key::PageDown),
            i.key_pressed(egui::Key::Home),
            i.key_pressed(egui::Key::End),
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
            i.key_pressed(egui::Key::A),
            i.key_pressed(egui::Key::C),
            i.events.clone(),
        )
    });

    let (modifiers, up, down, page_up, page_down, home, end, enter, escape, key_a, key_c, events) =
        input;

    // Handle Escape - clear selection
    if escape {
        msgs.push(Message::ClearTableSelection { tile_id });
        return;
    }

    // Handle Enter - activate selection
    if enter {
        msgs.push(Message::TableActivateSelection { tile_id });
        return;
    }

    // Handle Ctrl/Cmd+A - select all
    if key_a && modifiers.command && selection_mode == TableSelectionMode::Multi {
        msgs.push(Message::TableSelectAll { tile_id });
        return;
    }

    let copy_event = events
        .iter()
        .any(|event| matches!(event, egui::Event::Copy));

    // Handle Ctrl/Cmd+C - copy selection
    if (key_c && modifiers.command) || copy_event {
        msgs.push(Message::TableCopySelection {
            tile_id,
            include_header: modifiers.shift,
        });
        return;
    }

    // PHASE 1: Read-only navigation (immutable borrow of state)
    let nav_result = {
        let Some(runtime) = state.table_runtime.get(&tile_id) else {
            return;
        };
        let Some(cache_entry) = &runtime.cache else {
            return;
        };
        let Some(cache) = cache_entry.get() else {
            return;
        };
        let visible_rows = &cache.row_ids;
        let row_index = &cache.row_index;
        let selection = &runtime.selection;

        if modifiers.shift {
            // Shift+navigation extends selection
            let target = if up {
                navigate_up(selection, visible_rows, row_index).target_row
            } else if down {
                navigate_down(selection, visible_rows, row_index).target_row
            } else if page_up {
                navigate_page_up(selection, visible_rows, row_index, page_size).target_row
            } else if page_down {
                navigate_page_down(selection, visible_rows, row_index, page_size).target_row
            } else if home {
                navigate_home(visible_rows).target_row
            } else if end {
                navigate_end(visible_rows).target_row
            } else {
                None
            };

            target.map(|t| navigate_extend_selection(selection, t, visible_rows, row_index))
        } else if up {
            Some(navigate_up(selection, visible_rows, row_index))
        } else if down {
            Some(navigate_down(selection, visible_rows, row_index))
        } else if page_up {
            Some(navigate_page_up(
                selection,
                visible_rows,
                row_index,
                page_size,
            ))
        } else if page_down {
            Some(navigate_page_down(
                selection,
                visible_rows,
                row_index,
                page_size,
            ))
        } else if home || (modifiers.command && up) {
            Some(navigate_home(visible_rows))
        } else if end || (modifiers.command && down) {
            Some(navigate_end(visible_rows))
        } else {
            None
        }
    };

    // PHASE 2: Apply navigation result
    if let Some(result) = nav_result {
        if result.selection_changed
            && let Some(new_selection) = result.new_selection
        {
            msgs.push(Message::SetTableSelection {
                tile_id,
                selection: new_selection,
            });
        }
        return;
    }

    // Handle type-to-search for printable characters
    for event in &events {
        if let egui::Event::Text(text) = event {
            // Only process single characters for type-to-search
            if text.len() == 1 && !modifiers.command && !modifiers.ctrl && !modifiers.alt {
                let c = text.chars().next().unwrap();
                if c.is_alphanumeric() || c.is_whitespace() || c == '_' || c == '-' {
                    // Update type search state (mutable borrow)
                    let now = std::time::Instant::now();
                    if let Some(runtime) = state.table_runtime.get_mut(&tile_id) {
                        let _buffer = runtime.type_search.push_char(c, now);
                    }

                    // Re-borrow immutably for search match
                    if let Some(runtime) = state.table_runtime.get(&tile_id)
                        && let Some(cache_entry) = &runtime.cache
                        && let Some(cache) = cache_entry.get()
                        && let Some(model) = runtime.model.as_deref()
                    {
                        let query = runtime.type_search.buffer.clone();
                        if let Some(match_row) = find_type_search_match_in_cache(
                            &query,
                            &runtime.selection,
                            cache,
                            model,
                        ) {
                            let mut new_selection = TableSelection::new();
                            new_selection.rows.insert(match_row);
                            new_selection.anchor = Some(match_row);
                            msgs.push(Message::SetTableSelection {
                                tile_id,
                                selection: new_selection,
                            });
                        }
                    }
                    return;
                }
            }
        }
    }
}

/// Renders the actual table using egui_extras::TableBuilder.
///
/// Note: `sticky_header` config is stored but egui_extras::TableBuilder always renders
/// headers as sticky (fixed above the scrolling body). Non-sticky headers would require
/// custom scrolling logic and is deferred to a future version.
#[allow(clippy::too_many_arguments)]
fn render_table(
    ui: &mut egui::Ui,
    msgs: &mut Vec<Message>,
    tile_id: TableTileId,
    table_area_id: egui::Id,
    model: Arc<dyn TableModel>,
    schema: &TableSchema,
    cache: &TableCache,
    current_sort: &[TableSortSpec],
    columns_config: &[crate::table::TableColumnConfig],
    selection: &TableSelection,
    selection_mode: TableSelectionMode,
    dense_rows: bool,
    _sticky_header: bool, // Reserved for future use; egui_extras headers are always sticky
    header_bg: egui::Color32,
    text_color: egui::Color32,
    selection_bg: egui::Color32,
    use_light_table_visuals: bool,
    scroll_target: Option<&ScrollTarget>,
) {
    let row_height = if dense_rows {
        ROW_HEIGHT_DENSE
    } else {
        ROW_HEIGHT_NORMAL
    };
    let header_hover_bg = header_hover_background_color(header_bg, selection_bg);

    // Build list of visible columns with their indices
    // If columns_config is empty, show all schema columns
    let visible_col_info: Vec<(usize, &crate::table::TableColumn)> = if columns_config.is_empty() {
        schema.columns.iter().enumerate().collect()
    } else {
        // Get visible columns in config order
        let vis_keys = visible_columns(columns_config);
        vis_keys
            .iter()
            .filter_map(|key| {
                schema
                    .columns
                    .iter()
                    .position(|col| &col.key == key)
                    .map(|idx| (idx, &schema.columns[idx]))
            })
            .collect()
    };

    let full_labels: HashMap<TableColumnKey, String> = visible_col_info
        .iter()
        .map(|(_, column)| (column.key.clone(), full_column_label(column)))
        .collect();
    let visible_label_inputs: Vec<(TableColumnKey, String)> = visible_col_info
        .iter()
        .map(|(_, column)| {
            (
                column.key.clone(),
                full_labels
                    .get(&column.key)
                    .cloned()
                    .unwrap_or_else(|| column_key_string(&column.key)),
            )
        })
        .collect();
    let compact_labels = compact_unique_labels(&visible_label_inputs);

    // Determine scroll-to row index if scroll target specified
    let scroll_to_row = scroll_target.and_then(|target| match target {
        ScrollTarget::ToRow(row_id) => cache.row_index.get(row_id).copied(),
        ScrollTarget::ToTop => Some(0),
        ScrollTarget::ToBottom if !cache.row_ids.is_empty() => Some(cache.row_ids.len() - 1),
        _ => None,
    });

    // Track sort changes, selection changes, and visibility changes to emit after rendering
    let mut new_sort: Option<Vec<TableSortSpec>> = None;
    let mut new_selection: Option<TableSelection> = None;
    let mut new_visibility_toggle: Option<TableColumnKey> = None;

    // Use references to cache data — cache outlives the closures
    let selection_clone = selection.clone();
    let visible_rows = &cache.row_ids;
    let row_index = &cache.row_index;

    // Track column keys and indices for context menu and rendering
    let column_keys: Vec<TableColumnKey> = visible_col_info
        .iter()
        .map(|(_, c)| c.key.clone())
        .collect();
    let all_schema_columns: Vec<TableColumnKey> =
        schema.columns.iter().map(|c| c.key.clone()).collect();

    // Wrap in horizontal ScrollArea for wide tables (follows logs.rs pattern)
    ui.scope(|ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);

        if use_light_table_visuals {
            let visuals = ui.visuals_mut();
            visuals.dark_mode = false;
            visuals.faint_bg_color = header_bg;
            visuals.widgets.hovered.bg_fill = header_hover_bg;
        }

        egui::ScrollArea::horizontal()
            .auto_shrink(false)
            .show(ui, |ui| {
                // Build columns from visible columns
                let mut builder = TableBuilder::new(ui)
                    .striped(true)
                    .vscroll(true)
                    .sense(egui::Sense::click())
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center));

                // Add columns based on visible columns
                for (schema_idx, col) in &visible_col_info {
                    // Get width from config if available, otherwise use schema default
                    let width = columns_config
                        .iter()
                        .find(|c| c.key == col.key)
                        .and_then(|c| c.width)
                        .or(col.default_width)
                        .unwrap_or(100.0);

                    let resizable = columns_config
                        .iter()
                        .find(|c| c.key == col.key)
                        .map(|c| c.resizable)
                        .unwrap_or(col.default_resizable);

                    let column = if resizable {
                        Column::initial(width).resizable(true).clip(true)
                    } else {
                        Column::exact(width)
                    };
                    builder = builder.column(column);
                    let _ = schema_idx; // Used in body rendering
                }

                // Apply scroll target using egui's scroll_to_row
                if let Some(row_idx) = scroll_to_row {
                    builder = builder.scroll_to_row(row_idx, Some(egui::Align::Center));
                }

                // Render header with clickable sorting
                builder
                    .header(row_height, |mut header| {
                        for (_, col) in &visible_col_info {
                            header.col(|ui| {
                                let header_cell_rect = ui.available_rect_before_wrap();
                                let response = header_cell_interaction(ui, header_cell_rect, &col.key);
                                ui.painter().rect_filled(
                                    header_cell_rect,
                                    0.0,
                                    if response.hovered() {
                                        header_hover_bg
                                    } else {
                                        header_bg
                                    },
                                );

                                let full_label = full_labels
                                    .get(&col.key)
                                    .cloned()
                                    .unwrap_or_else(|| column_key_string(&col.key));
                                let compact_label = compact_labels
                                    .get(&col.key)
                                    .cloned()
                                    .unwrap_or_else(|| column_key_string(&col.key));
                                let indicator = sort_indicator(current_sort, &col.key);
                                let font_id = if dense_rows {
                                    egui::TextStyle::Small.resolve(ui.style())
                                } else {
                                    egui::TextStyle::Body.resolve(ui.style())
                                };
                                let measure_width = |text: &str| {
                                    ui.painter()
                                        .layout_no_wrap(text.to_string(), font_id.clone(), text_color)
                                        .size()
                                        .x
                                };
                                let indicator_reserved_width = indicator.as_ref().map_or(0.0, |ind| {
                                    measure_width(ind) + measure_width(" ")
                                });
                                let render_label = fit_render_label(
                                    &compact_label,
                                    header_cell_rect.width().max(0.0),
                                    indicator_reserved_width,
                                    measure_width,
                                );
                                let header_text = match &indicator {
                                    Some(ind) if !render_label.is_empty() => {
                                        format!("{render_label} {ind}")
                                    }
                                    Some(ind) => ind.clone(),
                                    None => render_label,
                                };
                                let label = if dense_rows {
                                    egui::RichText::new(&header_text).small().color(text_color)
                                } else {
                                    egui::RichText::new(&header_text).strong().color(text_color)
                                };
                                ui.add(egui::Label::new(label).selectable(false));

                                if response.clicked() {
                                    response
                                        .ctx
                                        .memory_mut(|mem| mem.request_focus(table_area_id));
                                    // Capture modifiers from the click event to preserve Shift+click behavior.
                                    let modifiers = header_click_modifiers(&response);
                                    let computed_sort = if modifiers.shift {
                                        sort_spec_on_shift_click(current_sort, &col.key)
                                    } else {
                                        sort_spec_on_click(current_sort, &col.key)
                                    };
                                    new_sort = Some(computed_sort);
                                }

                                // Context menu for column visibility and drill-down
                                response.context_menu(|ui| {
                                    // Drill-down: open single-signal change list for signal columns
                                    if let TableColumnKey::Str(key_str) = &col.key
                                        && let Some((full_path, field)) =
                                            crate::table::sources::decode_signal_column_key(key_str)
                                    {
                                        if ui.button("Signal change list").clicked() {
                                            let variable =
                                                crate::wave_container::VariableRef::from_hierarchy_string(
                                                    &full_path,
                                                );
                                            msgs.push(Message::AddTableTile {
                                                spec: TableModelSpec::SignalChangeList { variable, field },
                                            });
                                            ui.close();
                                        }
                                        ui.separator();
                                    }
                                    ui.label("Column visibility:");
                                    ui.separator();
                                    for key in &all_schema_columns {
                                        let mut is_visible = column_keys.contains(key);
                                        let col_label = schema
                                            .columns
                                            .iter()
                                            .find(|c| &c.key == key)
                                            .map(full_column_label)
                                            .unwrap_or_else(|| column_key_string(key));
                                        if ui.checkbox(&mut is_visible, col_label).changed() {
                                            new_visibility_toggle = Some(key.clone());
                                            ui.close();
                                        }
                                    }
                                });

                                // Show tooltip for sorting help
                                response.on_hover_text(format!(
                                    "{full_label}\nClick to sort, Shift+click for multi-column sort, right-click for column options"
                                ));
                            });
                        }
                    })
                    .body(|body| {
                        let total_rows = cache.row_ids.len();

                        body.rows(row_height, total_rows, |mut row| {
                            let row_idx = row.index();
                            if let Some(&row_id) = cache.row_ids.get(row_idx) {
                                // Check if this row is selected
                                let is_selected = selection_clone.contains(row_id);

                                // Set row background color for selected rows
                                if is_selected {
                                    row.set_selected(true);
                                }

                                // Render only visible columns
                                for (col_idx, _) in &visible_col_info {
                                    row.col(|ui| {
                                        // Paint selection background if selected
                                        if is_selected {
                                            ui.painter().rect_filled(
                                                ui.available_rect_before_wrap(),
                                                0.0,
                                                selection_bg,
                                            );
                                        }

                                        // Per-row cell access (egui only calls visible rows)
                                        let cell = model.cell(row_id, *col_idx);
                                        let text = match cell {
                                            TableCell::Text(s) => s,
                                            TableCell::RichText(rt) => {
                                                ui.add(egui::Label::new(rt).selectable(false));
                                                return;
                                            }
                                        };
                                        let label = if dense_rows {
                                            egui::RichText::new(&text).small()
                                        } else {
                                            egui::RichText::new(&text)
                                        };
                                        ui.add(egui::Label::new(label).selectable(false));
                                    });
                                }

                                // Handle row click for selection (only if selection mode is not None)
                                if selection_mode != TableSelectionMode::None {
                                    let response = row.response();
                                    if response.clicked() {
                                        response
                                            .ctx
                                            .memory_mut(|mem| mem.request_focus(table_area_id));
                                        let modifiers = response.ctx.input(|i| i.modifiers);
                                        let update = match selection_mode {
                                            TableSelectionMode::None => None,
                                            TableSelectionMode::Single => {
                                                Some(selection_on_click_single(&selection_clone, row_id))
                                            }
                                            TableSelectionMode::Multi => {
                                                if modifiers.command {
                                                    // Ctrl/Cmd+click: toggle
                                                    Some(selection_on_ctrl_click(&selection_clone, row_id))
                                                } else if modifiers.shift {
                                                    // Shift+click: range selection
                                                    Some(selection_on_shift_click(
                                                        &selection_clone,
                                                        row_id,
                                                        visible_rows,
                                                        row_index,
                                                    ))
                                                } else {
                                                    // Plain click: select single
                                                    Some(selection_on_click_multi(&selection_clone, row_id))
                                                }
                                            }
                                        };

                                        if let Some(update) = update
                                            && update.changed
                                        {
                                            new_selection = Some(update.selection);
                                        }
                                    }
                                }
                            }
                        });
                    });
            });
    });

    // Emit sort change message if needed
    if let Some(sort) = new_sort {
        msgs.push(Message::SetTableSort { tile_id, sort });
    }

    // Emit selection change message if needed
    if let Some(selection) = new_selection {
        msgs.push(Message::SetTableSelection { tile_id, selection });
    }

    // Emit column visibility toggle message if needed
    if let Some(column_key) = new_visibility_toggle {
        msgs.push(Message::ToggleTableColumnVisibility {
            tile_id,
            column_key,
        });
    }
}

fn search_mode_label(mode: TableSearchMode) -> &'static str {
    match mode {
        TableSearchMode::Contains => "Contains",
        TableSearchMode::Exact => "Exact",
        TableSearchMode::Regex => "Regex",
        TableSearchMode::Fuzzy => "Fuzzy",
    }
}

fn case_sensitivity_label(case_sensitive: bool) -> &'static str {
    if case_sensitive { "Aa" } else { "aa" }
}

fn search_mode_case_tooltip(case_sensitive: bool) -> &'static str {
    if case_sensitive {
        "Case sensitive (click to toggle)"
    } else {
        "Case insensitive (click to toggle)"
    }
}

fn filter_column_options(schema: Option<&TableSchema>) -> Vec<(TableColumnKey, String)> {
    schema
        .map(|schema| {
            schema
                .columns
                .iter()
                .map(|column| (column.key.clone(), full_column_label(column)))
                .collect()
        })
        .unwrap_or_default()
}

fn selected_filter_column_label(
    selected: Option<&TableColumnKey>,
    schema: Option<&TableSchema>,
    options: &[(TableColumnKey, String)],
) -> String {
    match selected {
        None => "All columns".to_string(),
        Some(selected_key) => options
            .iter()
            .find(|(key, _)| key == selected_key)
            .map(|(_, label)| label.clone())
            .or_else(|| {
                schema.and_then(|schema| {
                    schema
                        .columns
                        .iter()
                        .find(|column| &column.key == selected_key)
                        .map(full_column_label)
                })
            })
            .unwrap_or_else(|| "<column missing>".to_string()),
    }
}

fn chip_column_label(column: Option<&TableColumnKey>, schema: Option<&TableSchema>) -> String {
    match column {
        None => "All".to_string(),
        Some(column_key) => schema
            .and_then(|schema| {
                schema
                    .columns
                    .iter()
                    .find(|column| column.key == *column_key)
                    .map(full_column_label)
            })
            .unwrap_or_else(|| "<column missing>".to_string()),
    }
}

fn pinned_filter_chip_label(filter: &TableSearchSpec, schema: Option<&TableSchema>) -> String {
    format!(
        "{} | {} | {} | {}",
        chip_column_label(filter.column.as_ref(), schema),
        search_mode_label(filter.mode),
        case_sensitivity_label(filter.case_sensitive),
        filter.text
    )
}

fn is_valid_pin_candidate(draft: &FilterDraft) -> bool {
    if draft.text.is_empty() {
        return false;
    }
    if draft.mode != TableSearchMode::Regex {
        return true;
    }
    RegexBuilder::new(&draft.text)
        .case_insensitive(!draft.case_sensitive)
        .build()
        .is_ok()
}

fn append_pinned_filter(
    filters: &[TableSearchSpec],
    new_filter: TableSearchSpec,
) -> Vec<TableSearchSpec> {
    normalize_search_specs(
        filters
            .iter()
            .cloned()
            .chain(std::iter::once(new_filter))
            .collect(),
    )
}

/// Checks if the filter draft should be applied (debounce elapsed) and emits message if so.
/// MUST be called AFTER `render_filter_bar` to avoid applying stale values.
fn check_filter_debounce(
    state: &mut SystemState,
    tile_id: TableTileId,
    applied_filter: &TableSearchSpec,
    msgs: &mut Vec<Message>,
) {
    let Some(runtime) = state.table_runtime.get_mut(&tile_id) else {
        return;
    };
    let Some(draft) = &runtime.filter_draft else {
        return;
    };

    if draft.is_dirty(applied_filter) && draft.debounce_elapsed_now() {
        let filter_spec = draft.to_spec();
        msgs.push(Message::SetTableDisplayFilter {
            tile_id,
            filter: filter_spec,
        });
        // Clear timestamp to prevent re-applying until next change
        if let Some(d) = &mut runtime.filter_draft {
            d.last_changed = None;
        }
    }
}

/// Renders the filter bar above the table with text input, mode selector, and case toggle.
/// Uses draft state for UI binding to preserve focus during cache rebuilds.
fn render_filter_bar(
    ui: &mut egui::Ui,
    msgs: &mut Vec<Message>,
    tile_id: TableTileId,
    runtime: &mut TableRuntimeState,
    config: &TableViewConfig,
    schema: Option<&TableSchema>,
) {
    // Initialize draft from applied filter if needed.
    // This handles: fresh runtime, state load from disk, external filter changes.
    let draft = runtime
        .filter_draft
        .get_or_insert_with(|| FilterDraft::from_spec(&config.display_filter));

    let mut changed = false;
    let mut reset_toolbar = false;
    let mut focus_text_input = false;
    let filter_active = !draft.text.is_empty();
    let column_options = filter_column_options(schema);
    let selected_column_label =
        selected_filter_column_label(draft.column.as_ref(), schema, &column_options);

    ui.horizontal(|ui| {
        // Filter icon/label to indicate this is a filter bar
        if filter_active {
            ui.label(egui::RichText::new("Filter:").strong());
        } else {
            ui.label("Filter:");
        }

        egui::ComboBox::from_id_salt(format!("filter_column_{}", tile_id.0))
            .selected_text(selected_column_label)
            .width(140.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(draft.column.is_none(), "All columns")
                    .clicked()
                    && draft.column.is_some()
                {
                    draft.column = None;
                    changed = true;
                }
                for (key, label) in &column_options {
                    if ui
                        .selectable_label(draft.column.as_ref() == Some(key), label)
                        .clicked()
                        && draft.column.as_ref() != Some(key)
                    {
                        draft.column = Some(key.clone());
                        changed = true;
                    }
                }
            });

        // Text input bound to draft - MUST have tile-scoped ID for multi-table support
        let text_response = ui.add(
            egui::TextEdit::singleline(&mut draft.text)
                .id(egui::Id::new(("filter_text", tile_id.0)))
                .hint_text("Search...")
                .desired_width(150.0),
        );
        if text_response.changed() {
            changed = true;
        }

        // Mode selector bound to draft (already tile-scoped via from_id_salt)
        egui::ComboBox::from_id_salt(format!("filter_mode_{}", tile_id.0))
            .selected_text(search_mode_label(draft.mode))
            .width(70.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_value(&mut draft.mode, TableSearchMode::Contains, "Contains")
                    .changed()
                {
                    changed = true;
                }
                if ui
                    .selectable_value(&mut draft.mode, TableSearchMode::Exact, "Exact")
                    .changed()
                {
                    changed = true;
                }
                if ui
                    .selectable_value(&mut draft.mode, TableSearchMode::Regex, "Regex")
                    .changed()
                {
                    changed = true;
                }
                if ui
                    .selectable_value(&mut draft.mode, TableSearchMode::Fuzzy, "Fuzzy")
                    .changed()
                {
                    changed = true;
                }
            });

        // Case sensitivity toggle bound to draft
        if ui
            .add(
                egui::Button::new(case_sensitivity_label(draft.case_sensitive))
                    .min_size(egui::vec2(28.0, 0.0)),
            )
            .on_hover_text(search_mode_case_tooltip(draft.case_sensitive))
            .clicked()
        {
            draft.case_sensitive = !draft.case_sensitive;
            changed = true;
        }

        // Pin button - applies pinned filter immediately and resets toolbar.
        if ui
            .add_enabled(
                is_valid_pin_candidate(draft),
                egui::Button::new("Pin").min_size(egui::vec2(32.0, 0.0)),
            )
            .on_hover_text("Pin current filter")
            .clicked()
        {
            msgs.push(Message::SetTablePinnedFilters {
                tile_id,
                filters: append_pinned_filter(&config.pinned_filters, draft.to_spec()),
            });
            reset_toolbar = true;
            focus_text_input = true;
        }

        // Clear button - applies immediately (no debounce), does not affect pinned filters.
        let clear_enabled = !is_default_search_spec(&config.display_filter)
            || draft.is_dirty(&config.display_filter);
        if ui
            .add_enabled(
                clear_enabled,
                egui::Button::new("Clear").min_size(egui::vec2(40.0, 0.0)),
            )
            .clicked()
        {
            reset_toolbar = true;
        }

        // Show pending indicator when draft differs from applied
        if draft.is_dirty(&config.display_filter) {
            ui.spinner();
        }

        // Row count and selection count display (right-aligned)
        // Note: Uses applied filter stats since those reflect currently displayed data
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Selection count (displayed on the right)
            let selection = &runtime.selection;
            let hidden_count = runtime.hidden_selection_count;
            let selection_text = format_selection_count(selection.len(), hidden_count);
            if !selection_text.is_empty() {
                ui.label(egui::RichText::new(&selection_text).italics());
                ui.separator();
            }

            // Row count - show from cache if available
            if let Some(cache_entry) = &runtime.cache
                && let Some(cache) = cache_entry.get()
            {
                let filtered_rows = cache.row_ids.len();
                ui.label(format!("{filtered_rows} rows"));
            }
        });
    });

    if reset_toolbar {
        *draft = FilterDraft::default();
        draft.last_changed = None;
        msgs.push(Message::SetTableDisplayFilter {
            tile_id,
            filter: TableSearchSpec::default(),
        });
    } else if changed {
        // Mark draft as changed AFTER all UI interactions
        draft.last_changed = Some(Instant::now());
    }

    if !config.pinned_filters.is_empty() {
        let mut remove_index: Option<usize> = None;
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("Pinned:").small().italics());
            for (index, filter) in config.pinned_filters.iter().enumerate() {
                if ui
                    .small_button(pinned_filter_chip_label(filter, schema))
                    .on_hover_text("Click to remove")
                    .clicked()
                {
                    remove_index = Some(index);
                }
            }
        });
        if let Some(index) = remove_index {
            let mut filters = config.pinned_filters.clone();
            filters.remove(index);
            msgs.push(Message::SetTablePinnedFilters { tile_id, filters });
        }
    }

    if focus_text_input {
        ui.memory_mut(|memory| {
            memory.request_focus(egui::Id::new(("filter_text", tile_id.0)));
        });
    }

    ui.separator();
}

/// Renders a bar showing hidden columns that can be clicked to show.
fn render_column_visibility_bar(
    ui: &mut egui::Ui,
    msgs: &mut Vec<Message>,
    tile_id: TableTileId,
    hidden_cols: &[TableColumnKey],
    schema: &TableSchema,
) {
    if hidden_cols.is_empty() {
        return;
    }

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("{} hidden column(s):", hidden_cols.len()))
                .small()
                .italics(),
        );

        for key in hidden_cols {
            let label = schema
                .columns
                .iter()
                .find(|column| &column.key == key)
                .map(full_column_label)
                .unwrap_or_else(|| column_key_string(key));
            if ui
                .small_button(&label)
                .on_hover_text("Click to show")
                .clicked()
            {
                msgs.push(Message::ToggleTableColumnVisibility {
                    tile_id,
                    column_key: key.clone(),
                });
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_channel_uses_expected_weighting() {
        assert_eq!(blend_channel(10, 110), 50);
        assert_eq!(blend_channel(20, 220), 100);
        assert_eq!(blend_channel(30, 70), 46);
    }

    #[test]
    fn header_hover_background_blends_header_and_selection_colors() {
        let header = egui::Color32::from_rgb(10, 20, 30);
        let selection = egui::Color32::from_rgb(110, 220, 70);
        let hover = header_hover_background_color(header, selection);

        assert_eq!(hover, egui::Color32::from_rgb(50, 100, 46));
    }

    #[test]
    fn header_hover_background_keeps_same_color_for_identical_inputs() {
        let color = egui::Color32::from_rgba_unmultiplied(42, 84, 126, 255);
        assert_eq!(header_hover_background_color(color, color), color);
    }

    #[test]
    fn is_light_color_detects_light_and_dark_colors() {
        assert!(is_light_color(egui::Color32::from_rgb(240, 240, 240)));
        assert!(!is_light_color(egui::Color32::from_rgb(20, 20, 20)));
    }

    #[test]
    fn header_cell_interaction_uses_full_cell_rect() {
        let cell_rect = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(120.0, 24.0));
        let column_key = TableColumnKey::Str("col".to_string());

        egui::__run_test_ui(|ui| {
            let response = header_cell_interaction(ui, cell_rect, &column_key);
            assert_eq!(response.rect, cell_rect);
            assert_eq!(response.interact_rect, cell_rect);
            assert!(response.sense.senses_click());
        });
    }

    #[test]
    fn header_cell_interaction_id_varies_by_column_key() {
        let cell_rect = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(120.0, 24.0));
        let left_key = TableColumnKey::Str("left".to_string());
        let right_key = TableColumnKey::Str("right".to_string());

        egui::__run_test_ui(|ui| {
            let left_response = header_cell_interaction(ui, cell_rect, &left_key);
            let right_response = header_cell_interaction(ui, cell_rect, &right_key);
            assert_ne!(left_response.id, right_response.id);
        });
    }

    #[test]
    fn primary_click_modifiers_uses_latest_primary_release_inside_rect() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 20.0));
        let mut shift = egui::Modifiers::default();
        shift.shift = true;

        let mut command = egui::Modifiers::default();
        command.command = true;

        let events = vec![
            egui::Event::PointerButton {
                pos: egui::pos2(10.0, 10.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: command,
            },
            egui::Event::PointerButton {
                pos: egui::pos2(11.0, 10.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: shift,
            },
        ];

        assert_eq!(
            primary_click_modifiers_for_rect(&events, rect, egui::Modifiers::default()),
            shift
        );
    }

    #[test]
    fn primary_click_modifiers_ignores_non_matching_events() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 20.0));
        let mut fallback = egui::Modifiers::default();
        fallback.alt = true;

        let mut shift = egui::Modifiers::default();
        shift.shift = true;

        let events = vec![
            egui::Event::PointerButton {
                pos: egui::pos2(10.0, 10.0),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: shift,
            },
            egui::Event::PointerButton {
                pos: egui::pos2(200.0, 10.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: shift,
            },
            egui::Event::PointerButton {
                pos: egui::pos2(10.0, 10.0),
                button: egui::PointerButton::Secondary,
                pressed: false,
                modifiers: shift,
            },
        ];

        assert_eq!(
            primary_click_modifiers_for_rect(&events, rect, fallback),
            fallback
        );
    }

    #[test]
    fn pinned_filter_chip_label_formats_expected_text() {
        let schema = TableSchema {
            columns: vec![crate::table::TableColumn {
                key: TableColumnKey::Str("action".to_string()),
                label: "Action".to_string(),
                default_width: None,
                default_visible: true,
                default_resizable: true,
            }],
        };
        let filter = TableSearchSpec {
            mode: TableSearchMode::Exact,
            case_sensitive: true,
            text: "READ".to_string(),
            column: Some(TableColumnKey::Str("action".to_string())),
        };

        assert_eq!(
            pinned_filter_chip_label(&filter, Some(&schema)),
            "Action | Exact | Aa | READ"
        );
    }

    #[test]
    fn append_pinned_filter_dedupes_and_drops_empty() {
        let duplicate = TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: "Type".to_string(),
            column: Some(TableColumnKey::Str("type".to_string())),
        };
        let existing = vec![
            TableSearchSpec {
                mode: TableSearchMode::Contains,
                case_sensitive: false,
                text: String::new(),
                column: Some(TableColumnKey::Str("ignored".to_string())),
            },
            duplicate.clone(),
        ];

        let result = append_pinned_filter(&existing, duplicate);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].text, "Type");
    }

    #[test]
    fn column_key_string_uses_stable_strings() {
        assert_eq!(
            column_key_string(&TableColumnKey::Str("alpha.beta".to_string())),
            "alpha.beta"
        );
        assert_eq!(column_key_string(&TableColumnKey::Id(42)), "42");
    }

    #[test]
    fn full_column_label_falls_back_to_column_key_when_empty() {
        let column = crate::table::TableColumn {
            key: TableColumnKey::Str("fallback.name".to_string()),
            label: String::new(),
            default_width: None,
            default_visible: true,
            default_resizable: true,
        };

        assert_eq!(full_column_label(&column), "fallback.name");
    }

    #[test]
    fn compact_unique_labels_uses_shortest_unique_suffixes() {
        let key0 = TableColumnKey::Str("k0".to_string());
        let key1 = TableColumnKey::Str("k1".to_string());
        let key2 = TableColumnKey::Str("k2".to_string());
        let inputs = vec![
            (key0.clone(), "tb.top.cpu0.core0.regfile.wdata".to_string()),
            (key1.clone(), "tb.top.cpu0.core1.regfile.wdata".to_string()),
            (key2.clone(), "tb.top.cpu0.core1.regfile.rdata".to_string()),
        ];

        let labels = compact_unique_labels(&inputs);
        assert_eq!(labels.get(&key0), Some(&":core0.regfile.wdata".to_string()));
        assert_eq!(labels.get(&key1), Some(&":core1.regfile.wdata".to_string()));
        assert_eq!(labels.get(&key2), Some(&":rdata".to_string()));
    }

    #[test]
    fn compact_unique_labels_falls_back_to_full_label_when_no_suffix_is_unique() {
        let key0 = TableColumnKey::Str("k0".to_string());
        let key1 = TableColumnKey::Str("k1".to_string());
        let inputs = vec![
            (key0.clone(), "tb.top.cpu0.wdata".to_string()),
            (key1.clone(), "tb.top.cpu0.wdata".to_string()),
        ];

        let labels = compact_unique_labels(&inputs);
        assert_eq!(labels.get(&key0), Some(&"tb.top.cpu0.wdata".to_string()));
        assert_eq!(labels.get(&key1), Some(&"tb.top.cpu0.wdata".to_string()));
    }

    #[test]
    fn fit_render_label_preserves_right_edge_with_truncation_marker() {
        let measure = |text: &str| text.chars().count() as f32;

        assert_eq!(
            fit_render_label(":core0.regfile.wdata", 6.0, 0.0, measure),
            ":wdata"
        );
        assert_eq!(
            fit_render_label(":core0.regfile.wdata", 2.0, 0.0, measure),
            ":a"
        );
        assert_eq!(
            fit_render_label(":core0.regfile.wdata", 1.0, 0.0, measure),
            ":"
        );
        assert_eq!(
            fit_render_label(":core0.regfile.wdata", 0.0, 0.0, measure),
            ""
        );
    }

    #[test]
    fn fit_render_label_reserves_sort_indicator_width_before_fitting() {
        let measure = |text: &str| text.chars().count() as f32;
        assert_eq!(
            fit_render_label(":wdata.avg", 12.0, 4.0, measure),
            ":ata.avg"
        );
    }

    #[test]
    fn filter_column_options_use_full_labels_only() {
        let schema = TableSchema {
            columns: vec![
                crate::table::TableColumn {
                    key: TableColumnKey::Str("type".to_string()),
                    label: "Type".to_string(),
                    default_width: None,
                    default_visible: true,
                    default_resizable: true,
                },
                crate::table::TableColumn {
                    key: TableColumnKey::Str("action".to_string()),
                    label: "Action".to_string(),
                    default_width: None,
                    default_visible: true,
                    default_resizable: true,
                },
            ],
        };

        let options = filter_column_options(Some(&schema));
        let action_option = options
            .iter()
            .find(|(key, _)| *key == TableColumnKey::Str("action".to_string()))
            .expect("action option");
        assert_eq!(action_option.1, "Action");
    }

    #[test]
    fn filter_column_options_fallback_to_key_when_label_empty() {
        let schema = TableSchema {
            columns: vec![crate::table::TableColumn {
                key: TableColumnKey::Str("tb.top.counter.avg".to_string()),
                label: String::new(),
                default_width: None,
                default_visible: true,
                default_resizable: true,
            }],
        };

        let options = filter_column_options(Some(&schema));
        assert_eq!(options[0].1, "tb.top.counter.avg");
    }
}
