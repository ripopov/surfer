//! Read-only inspector of the remembered waveform's selected transaction.
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransactionDetailsTile {}

impl TransactionDetailsTile {
    pub(crate) fn ui(&self, ui: &mut egui::Ui, waves: Option<crate::wave_data::WaveformRead<'_>>) {
        let Some(waves) = waves else {
            ui.label("Open a waveform tile to inspect a transaction.");
            return;
        };
        let Some(reference) = &waves.view.focused_transaction else {
            ui.label("Select a transaction to view its details.");
            return;
        };
        let Some(transactions) = waves.inner.as_transactions() else {
            ui.label("Transaction data is unavailable.");
            return;
        };
        let Some(transaction) = transactions.get_transaction(reference) else {
            ui.label("The selected transaction is unavailable in this document.");
            return;
        };
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        egui::ScrollArea::both().show(ui, |ui| {
            crate::transactions::draw_focused_transaction_details(ui, transactions, transaction);
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::transaction_container::TransactionRef;
    use crate::{
        Message, SystemState,
        tile_kinds::waveform::WaveformMessage,
        tiles::{
            commands::WorkspaceCommand,
            kind::TileMessage,
            layout::{Direction, Placement},
        },
    };
    use ftr_parser::types::TransactionId;

    #[test]
    fn transaction_focus_opens_one_inspector_without_stealing_focus_and_clearing_keeps_it() {
        let mut state = SystemState::new_default_config().unwrap();
        state
            .update(Message::Workspace(WorkspaceCommand::CreateTile {
                kind: "waveform".into(),
                placement: Placement::Root,
                focus: true,
            }))
            .unwrap();
        let waveform = state.user.workspace.layout.focused().unwrap();
        let focus = |id: Option<TransactionId>| {
            Message::ToTile(
                waveform,
                TileMessage::Waveform(WaveformMessage::FocusTransaction(
                    id.map(|id| TransactionRef { id }),
                )),
            )
        };
        state.update(focus(Some(TransactionId(12)))).unwrap();
        let details = *state
            .user
            .workspace
            .tiles
            .iter()
            .find(|(_, entry)| entry.kind.kind_name() == "transaction_details")
            .unwrap()
            .0;
        assert_eq!(state.user.workspace.layout.focused(), Some(waveform));
        assert!(
            state
                .user
                .workspace
                .layout
                .visible_tiles()
                .contains(&details)
        );
        let revision = state.user.workspace.layout.revision();
        state.update(focus(Some(TransactionId(13)))).unwrap();
        assert_eq!(state.user.workspace.tiles.len(), 2);
        assert_eq!(state.user.workspace.layout.revision(), revision);
        state.update(focus(None)).unwrap();
        assert!(state.user.workspace.tiles.contains_key(&details));
        assert_eq!(state.user.workspace.layout.focused(), Some(waveform));
        let encoded = state.encode_state().unwrap();
        let restored: crate::state::UserState = crate::tiles::serde::decode(&encoded).unwrap();
        assert_eq!(
            restored.workspace.tiles[&details].kind.kind_name(),
            "transaction_details"
        );
        assert!(
            state
                .update(Message::Workspace(WorkspaceCommand::SplitTile {
                    tile: details,
                    dir: Direction::Down,
                    mode: crate::tiles::commands::SplitMode::Clone
                }))
                .is_none()
        );
        state
            .update(Message::Workspace(WorkspaceCommand::CloseTile(details)))
            .unwrap();
        state.update(focus(Some(TransactionId(14)))).unwrap();
        let reopened = *state
            .user
            .workspace
            .tiles
            .iter()
            .find(|(_, entry)| entry.kind.kind_name() == "transaction_details")
            .unwrap()
            .0;
        assert_ne!(reopened, details);
        assert_eq!(state.user.workspace.layout.focused(), Some(waveform));
    }
}
