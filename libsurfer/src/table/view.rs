use crate::SystemState;
use crate::message::Message;
use crate::table::{TableCacheKey, TableModelKey, TableTileId};

/// Renders a table tile in the UI.
///
/// This function handles the full lifecycle of table rendering:
/// - If no tile config exists, shows an error message
/// - If cache is not ready, shows a loading indicator and triggers cache build
/// - If cache has an error, shows the error message
/// - Otherwise, renders the table (placeholder for Stage 5)
pub fn draw_table_tile(
    state: &mut SystemState,
    _ctx: &egui::Context,
    ui: &mut egui::Ui,
    msgs: &mut Vec<Message>,
    tile_id: TableTileId,
) {
    let Some(tile_state) = state.user.table_tiles.get(&tile_id) else {
        ui.centered_and_justified(|ui| {
            ui.label("Table tile not found");
        });
        return;
    };

    let title = tile_state.config.title.clone();
    let display_filter = tile_state.config.display_filter.clone();
    let view_sort = tile_state.config.sort.clone();

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

    // Render UI based on current state
    ui.vertical(|ui| {
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
                // Cache is ready - render table (placeholder for Stage 5)
                if let Some(cache) = cache_entry.get() {
                    ui.label(format!("Rows: {}", cache.row_ids.len()));
                    ui.label("(Table rendering will be implemented in Stage 5)");
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
    });
}
