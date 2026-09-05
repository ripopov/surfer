//! Commands whose effects belong to the shared document rather than a view.

use ::serde::Deserialize;
use num::BigInt;

use crate::wave_data::{ScopeType, WaveData};

#[derive(Debug, Clone, Copy, Deserialize)]
pub enum WcpVariableAction {
    GoToDeclaration,
    AddDrivers,
    AddLoads,
}

use super::{
    TileId,
    layout::{Direction, LayoutNode, Placement},
};

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
pub enum SplitMode {
    Linked,
    Independent,
    Clone,
}

/// Resolved user commands. Widget and input boundaries supply concrete tile IDs.
#[derive(Debug, Clone, Deserialize)]
pub enum WorkspaceCommand {
    CreateTile {
        kind: String,
        placement: Placement,
        focus: bool,
    },
    OpenTile {
        kind: String,
        placement: Placement,
        focus: bool,
    },
    CloseTile(TileId),
    CloseOtherTiles(TileId),
    FocusTile(TileId),
    SplitTile {
        tile: TileId,
        dir: Direction,
        mode: SplitMode,
    },
    MoveTile {
        tile: TileId,
        to: Placement,
    },
    RenameTile {
        tile: TileId,
        title: Option<String>,
    },
    SetLayout(Option<LayoutNode>),
    /// Restore the default layout: one waveform tile. `keep` retains that
    /// tile and its list; `None` creates an empty waveform.
    Reset {
        keep: Option<TileId>,
    },
}

#[derive(Debug, Deserialize)]
pub enum DocumentCommand {
    CursorSet(BigInt),
    /// `None` selects the hierarchy's top level.
    SetActiveScope(Option<ScopeType>),
}

impl WaveData {
    pub(crate) fn apply_command(&mut self, command: DocumentCommand) -> Option<()> {
        match command {
            DocumentCommand::CursorSet(time) => self.cursor = Some(time),
            DocumentCommand::SetActiveScope(scope) => self.set_active_scope(scope)?,
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        data_container::DataContainer,
        wave_data::TimeRange,
        wave_source::{WaveFormat, WaveSource},
    };

    #[test]
    fn shared_document_commands_need_no_item_list_or_view() {
        let mut document = WaveData {
            inner: DataContainer::Empty,
            source: WaveSource::Data,
            format: WaveFormat::Vcd,
            active_scope: None,
            cursor: None,
            markers: Default::default(),
            display_variable_indices: false,
            old_max_timestamp: None,
            cache_generation: 0,
            inflight_caches: Default::default(),
            cached_time_range: TimeRange::default(),
        };
        document
            .apply_command(DocumentCommand::CursorSet(BigInt::from(-17)))
            .unwrap();
        assert_eq!(document.cursor, Some(BigInt::from(-17)));
        document
            .apply_command(DocumentCommand::SetActiveScope(None))
            .unwrap();
        assert!(document.active_scope.is_none());
        assert!(document.markers.is_empty());
    }

    #[test]
    fn shared_document_has_no_thread_local_ui_caches() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<WaveData>();
    }
}
