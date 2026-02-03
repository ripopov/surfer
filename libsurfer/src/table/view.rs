use crate::SystemState;
use crate::message::Message;
use crate::table::TableTileId;

pub fn draw_table_tile(
    state: &mut SystemState,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    msgs: &mut Vec<Message>,
    tile_id: TableTileId,
) {
    let _ = (state, ctx, ui, msgs, tile_id);
}
