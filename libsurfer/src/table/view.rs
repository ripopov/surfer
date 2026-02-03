use crate::SystemState;
use crate::message::Message;
use crate::table::{
    TableCache, TableCacheKey, TableCell, TableModel, TableModelKey, TableRowId, TableSearchMode,
    TableSearchSpec, TableSelection, TableSelectionMode, TableSortSpec, TableTileId,
    TableTileState, find_type_search_match, format_selection_count, navigate_down, navigate_end,
    navigate_extend_selection, navigate_home, navigate_page_down, navigate_page_up, navigate_up,
    selection_on_click_multi, selection_on_click_single, selection_on_ctrl_click,
    selection_on_shift_click, sort_indicator, sort_spec_on_click, sort_spec_on_shift_click,
};
use egui_extras::{Column, TableBuilder};
use std::collections::HashMap;
use std::sync::Arc;

/// Default row height for normal table rows.
const ROW_HEIGHT_NORMAL: f32 = 20.0;
/// Row height for dense mode (smaller text, less padding).
const ROW_HEIGHT_DENSE: f32 = 16.0;

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

    // Get the model for schema access
    let model = tile_state.spec.create_model();

    // Get or create runtime state
    let runtime = state.table_runtime.entry(tile_id).or_default();

    // Compute current cache key
    // For now, use tile_id as a simple model_key; generation is 0 until waveform reload support
    let cache_key = TableCacheKey {
        model_key: TableModelKey(tile_id.0),
        display_filter,
        view_sort,
        generation: 0,
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

    // Get selection mode from config
    let selection_mode = tile_state.config.selection_mode;

    // Get total row count from model (unfiltered)
    let total_rows = model.as_ref().map_or(0, |m| m.row_count());

    // Create a unique ID for the table area to track focus
    let table_area_id = egui::Id::new(("table_area", tile_id.0));

    // Render UI based on current state
    let table_response = ui
        .vertical(|ui| {
            ui.heading(&title);
            ui.separator();

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
                        if let Some(ref model) = model {
                            // Get current selection for rendering
                            let selection = runtime.selection.clone();
                            let type_search_buffer = runtime.type_search.buffer.clone();

                            // Render filter bar above the table
                            render_filter_bar(
                                ui,
                                msgs,
                                tile_id,
                                &tile_state.config.display_filter,
                                total_rows,
                                cache.row_ids.len(),
                                &selection,
                                &cache.row_ids,
                            );

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
                                model.clone(),
                                cache,
                                &tile_state.config.sort,
                                &selection,
                                selection_mode,
                                dense_rows,
                                sticky_header,
                                header_bg,
                                text_color,
                                selection_bg,
                            );
                        } else {
                            ui.label("Model not available");
                        }
                    }
                } else {
                    // Loading state
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Loading table data...");
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
    // Clone data we need from runtime to avoid borrow conflicts
    let (visible_rows, search_texts, selection) = {
        let Some(runtime) = state.table_runtime.get(&tile_id) else {
            return;
        };
        let Some(cache_entry) = &runtime.cache else {
            return;
        };
        let Some(cache) = cache_entry.get() else {
            return;
        };

        (
            cache.row_ids.clone(),
            cache.search_texts.clone(),
            runtime.selection.clone(),
        )
    };

    // Calculate page size based on visible area (approximate)
    let page_size = 20; // Default page size; could be calculated from UI height

    // Collect keyboard input
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

    // Handle Ctrl/Cmd+C - copy selection
    if key_c && modifiers.command {
        msgs.push(Message::TableCopySelection {
            tile_id,
            include_header: modifiers.shift,
        });
        return;
    }

    // Handle navigation keys
    let nav_result = if modifiers.shift {
        // Shift+navigation extends selection
        let target = if up {
            navigate_up(&selection, &visible_rows).target_row
        } else if down {
            navigate_down(&selection, &visible_rows).target_row
        } else if page_up {
            navigate_page_up(&selection, &visible_rows, page_size).target_row
        } else if page_down {
            navigate_page_down(&selection, &visible_rows, page_size).target_row
        } else if home {
            navigate_home(&visible_rows).target_row
        } else if end {
            navigate_end(&visible_rows).target_row
        } else {
            None
        };

        target.map(|t| navigate_extend_selection(&selection, t, &visible_rows))
    } else if up {
        Some(navigate_up(&selection, &visible_rows))
    } else if down {
        Some(navigate_down(&selection, &visible_rows))
    } else if page_up {
        Some(navigate_page_up(&selection, &visible_rows, page_size))
    } else if page_down {
        Some(navigate_page_down(&selection, &visible_rows, page_size))
    } else if home || (modifiers.command && up) {
        Some(navigate_home(&visible_rows))
    } else if end || (modifiers.command && down) {
        Some(navigate_end(&visible_rows))
    } else {
        None
    };

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
    // Check for text input events
    for event in &events {
        if let egui::Event::Text(text) = event {
            // Only process single characters for type-to-search
            if text.len() == 1 && !modifiers.command && !modifiers.ctrl && !modifiers.alt {
                let c = text.chars().next().unwrap();
                if c.is_alphanumeric() || c.is_whitespace() || c == '_' || c == '-' {
                    // Update type search state
                    let now = std::time::Instant::now();

                    // Get mutable reference to runtime for type_search update
                    if let Some(runtime) = state.table_runtime.get_mut(&tile_id) {
                        let _buffer = runtime.type_search.push_char(c, now);
                        let query = runtime.type_search.buffer.clone();

                        // Find matching row
                        if let Some(match_row) =
                            find_type_search_match(&query, &selection, &visible_rows, &search_texts)
                        {
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
    model: Arc<dyn TableModel>,
    cache: &TableCache,
    current_sort: &[TableSortSpec],
    selection: &TableSelection,
    selection_mode: TableSelectionMode,
    dense_rows: bool,
    _sticky_header: bool, // Reserved for future use; egui_extras headers are always sticky
    header_bg: egui::Color32,
    text_color: egui::Color32,
    selection_bg: egui::Color32,
) {
    let schema = model.schema();
    let row_height = if dense_rows {
        ROW_HEIGHT_DENSE
    } else {
        ROW_HEIGHT_NORMAL
    };

    // Build columns from schema
    let mut builder = TableBuilder::new(ui)
        .striped(true)
        .vscroll(true)
        .sense(egui::Sense::click())
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center));

    // Add columns based on schema
    for col in &schema.columns {
        let width = col.default_width.unwrap_or(100.0);
        let column = if col.default_resizable {
            Column::initial(width).resizable(true).clip(true)
        } else {
            Column::exact(width)
        };
        builder = builder.column(column);
    }

    // Track sort changes and selection changes to emit after rendering
    let mut new_sort: Option<Vec<TableSortSpec>> = None;
    let mut new_selection: Option<TableSelection> = None;

    // Clone data needed inside closures
    let selection_clone = selection.clone();
    let visible_rows: Vec<TableRowId> = cache.row_ids.clone();

    // Render header with clickable sorting
    builder
        .header(row_height, |mut header| {
            for col in &schema.columns {
                header.col(|ui| {
                    ui.painter()
                        .rect_filled(ui.available_rect_before_wrap(), 0.0, header_bg);

                    // Build header text with sort indicator
                    let indicator = sort_indicator(current_sort, &col.key);
                    let header_text = match &indicator {
                        Some(ind) => format!("{} {}", col.label, ind),
                        None => col.label.clone(),
                    };

                    let label = if dense_rows {
                        egui::RichText::new(&header_text).small().color(text_color)
                    } else {
                        egui::RichText::new(&header_text).strong().color(text_color)
                    };

                    // Make the header clickable for sorting
                    let response = ui.add(
                        egui::Label::new(label)
                            .selectable(false)
                            .sense(egui::Sense::click()),
                    );

                    if response.clicked() {
                        // Check for Shift modifier
                        let modifiers = ui.input(|i| i.modifiers);
                        let computed_sort = if modifiers.shift {
                            sort_spec_on_shift_click(current_sort, &col.key)
                        } else {
                            sort_spec_on_click(current_sort, &col.key)
                        };
                        new_sort = Some(computed_sort);
                    }

                    // Show tooltip for sorting help
                    response.on_hover_text("Click to sort, Shift+click for multi-column sort");
                });
            }
        })
        .body(|body| {
            body.rows(row_height, cache.row_ids.len(), |mut row| {
                let row_idx = row.index();
                if let Some(&row_id) = cache.row_ids.get(row_idx) {
                    // Check if this row is selected
                    let is_selected = selection_clone.contains(row_id);

                    // Set row background color for selected rows
                    if is_selected {
                        row.set_selected(true);
                    }

                    for col_idx in 0..schema.columns.len() {
                        row.col(|ui| {
                            // Paint selection background if selected
                            if is_selected {
                                ui.painter().rect_filled(
                                    ui.available_rect_before_wrap(),
                                    0.0,
                                    selection_bg,
                                );
                            }

                            let cell = model.cell(row_id, col_idx);
                            let text = match cell {
                                TableCell::Text(s) => s,
                                TableCell::RichText(rt) => {
                                    ui.label(rt);
                                    return;
                                }
                            };
                            let label = if dense_rows {
                                egui::RichText::new(&text).small()
                            } else {
                                egui::RichText::new(&text)
                            };
                            ui.label(label);
                        });
                    }

                    // Handle row click for selection (only if selection mode is not None)
                    if selection_mode != TableSelectionMode::None {
                        let response = row.response();
                        if response.clicked() {
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
                                            &visible_rows,
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

    // Emit sort change message if needed
    if let Some(sort) = new_sort {
        msgs.push(Message::SetTableSort { tile_id, sort });
    }

    // Emit selection change message if needed
    if let Some(selection) = new_selection {
        msgs.push(Message::SetTableSelection { tile_id, selection });
    }
}

/// Renders the filter bar above the table with text input, mode selector, and case toggle.
#[allow(clippy::too_many_arguments)]
fn render_filter_bar(
    ui: &mut egui::Ui,
    msgs: &mut Vec<Message>,
    tile_id: TableTileId,
    current_filter: &TableSearchSpec,
    total_rows: usize,
    filtered_rows: usize,
    selection: &TableSelection,
    visible_rows: &[TableRowId],
) {
    // Track changes to emit after rendering
    let mut filter_changed = false;
    let mut new_text = current_filter.text.clone();
    let mut new_mode = current_filter.mode;
    let mut new_case_sensitive = current_filter.case_sensitive;

    ui.horizontal(|ui| {
        // Filter icon/label to indicate this is a filter bar
        let filter_active = !current_filter.text.is_empty();
        if filter_active {
            ui.label(egui::RichText::new("Filter:").strong());
        } else {
            ui.label("Filter:");
        }

        // Text input field
        let text_response = ui.add(
            egui::TextEdit::singleline(&mut new_text)
                .hint_text("Search...")
                .desired_width(150.0),
        );
        if text_response.changed() {
            filter_changed = true;
        }

        // Mode selector dropdown
        let mode_label = match current_filter.mode {
            TableSearchMode::Contains => "Contains",
            TableSearchMode::Exact => "Exact",
            TableSearchMode::Regex => "Regex",
            TableSearchMode::Fuzzy => "Fuzzy",
        };

        egui::ComboBox::from_id_salt(format!("filter_mode_{}", tile_id.0))
            .selected_text(mode_label)
            .width(70.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_value(&mut new_mode, TableSearchMode::Contains, "Contains")
                    .changed()
                {
                    filter_changed = true;
                }
                if ui
                    .selectable_value(&mut new_mode, TableSearchMode::Exact, "Exact")
                    .changed()
                {
                    filter_changed = true;
                }
                if ui
                    .selectable_value(&mut new_mode, TableSearchMode::Regex, "Regex")
                    .changed()
                {
                    filter_changed = true;
                }
                if ui
                    .selectable_value(&mut new_mode, TableSearchMode::Fuzzy, "Fuzzy")
                    .changed()
                {
                    filter_changed = true;
                }
            });

        // Case sensitivity toggle
        let case_label = if new_case_sensitive { "Aa" } else { "aa" };
        let case_tooltip = if new_case_sensitive {
            "Case sensitive (click to toggle)"
        } else {
            "Case insensitive (click to toggle)"
        };
        if ui
            .add(egui::Button::new(case_label).min_size(egui::vec2(28.0, 0.0)))
            .on_hover_text(case_tooltip)
            .clicked()
        {
            new_case_sensitive = !new_case_sensitive;
            filter_changed = true;
        }

        // Clear button (only shown when filter is active)
        if filter_active
            && ui
                .add(egui::Button::new("Clear").min_size(egui::vec2(40.0, 0.0)))
                .clicked()
        {
            new_text.clear();
            filter_changed = true;
        }

        // Row count and selection count display
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Selection count (displayed on the right)
            let hidden_count = selection.count_hidden(visible_rows);
            let selection_text = format_selection_count(selection.len(), hidden_count);
            if !selection_text.is_empty() {
                ui.label(egui::RichText::new(&selection_text).italics());
                ui.separator();
            }

            // Row count
            if filter_active {
                ui.label(format!("Showing {} of {} rows", filtered_rows, total_rows));
            } else {
                ui.label(format!("{} rows", total_rows));
            }
        });
    });

    // Emit filter change message if needed
    if filter_changed {
        msgs.push(Message::SetTableDisplayFilter {
            tile_id,
            filter: TableSearchSpec {
                mode: new_mode,
                case_sensitive: new_case_sensitive,
                text: new_text,
            },
        });
    }

    ui.separator();
}
