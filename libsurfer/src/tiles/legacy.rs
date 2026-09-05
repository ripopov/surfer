//! Version-zero waveform DTO. Migration fields never live on the shared document.

use crate::{
    annotation::Annotation,
    annotation_list::AnnotationGroup,
    data_container::DataContainer,
    displayed_item::{DisplayedItem, DisplayedItemRef},
    displayed_item_tree::{DisplayedItemTree, VisibleItemIndex},
    graphics::{Graphic, GraphicId},
    transaction_container::TransactionRef,
    variable_name_type::VariableNameType,
    viewport::Viewport,
    wave_container::AnalogCacheKey,
    wave_data::{ScopeType, TimeRange, WaveformData},
    wave_source::{WaveFormat, WaveSource},
};
use egui::Id;
use ftr_parser::types::Transaction;
use num::BigInt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Deserialize)]
pub(crate) struct LegacyWaveDataV0 {
    #[serde(skip, default = "DataContainer::__new_empty")]
    pub inner: DataContainer,
    pub source: WaveSource,
    pub format: WaveFormat,
    pub active_scope: Option<ScopeType>,
    /// Root items (variables, dividers, ...) to display
    pub items_tree: DisplayedItemTree,
    pub displayed_items: HashMap<DisplayedItemRef, DisplayedItem>,
    /// Tracks the consecutive displayed item refs
    pub display_item_ref_counter: usize,
    pub viewports: Vec<Viewport>,
    pub cursor: Option<BigInt>,
    pub markers: HashMap<u8, BigInt>,
    #[serde(default)]
    pub selected_annotation: Option<Id>,

    #[serde(default)]
    pub annotations: Vec<Annotation>,
    pub annotation_groups: Vec<AnnotationGroup>, // List of unique group names
    pub annotation_list_visible: bool,
    #[serde(default)]
    pub annotation_counter: i32,
    pub last_active_viewport_idx: usize,

    pub focused_item: Option<VisibleItemIndex>,
    pub focused_transaction: (Option<TransactionRef>, Option<Transaction>),
    pub default_variable_name_type: VariableNameType,
    pub scroll_offset: f32,
    pub display_variable_indices: bool,
    pub graphics: HashMap<GraphicId, Graphic>,
    #[serde(skip)]
    pub old_max_timestamp: Option<BigInt>,
    /// Generation counter for analog cache invalidation on waveform reload.
    #[serde(skip)]
    pub cache_generation: u64,
    /// Registry of in-flight analog cache builds for sharing.
    /// Cleared on waveform reload when generation changes.
    #[serde(skip)]
    pub inflight_caches:
        HashMap<AnalogCacheKey, std::sync::Arc<crate::analog_signal_cache::AnalogCacheEntry>>,
    /// Cached effective time offset, updated on waveform load and config change
    #[serde(skip, default)]
    pub(crate) cached_time_range: TimeRange,
}

impl From<LegacyWaveDataV0> for WaveformData {
    fn from(old: LegacyWaveDataV0) -> Self {
        let focused_item = old
            .focused_item
            .and_then(|index| old.items_tree.get_visible(index))
            .map(|node| node.item_ref);
        Self {
            document: crate::wave_data::WaveData {
                inner: old.inner,
                source: old.source,
                format: old.format,
                active_scope: old.active_scope,
                cursor: old.cursor,
                markers: old.markers,
                display_variable_indices: old.display_variable_indices,
                old_max_timestamp: old.old_max_timestamp,
                cache_generation: old.cache_generation,
                inflight_caches: old.inflight_caches,
                cached_time_range: old.cached_time_range,
            },
            items: crate::item_list::ItemList {
                items_tree: old.items_tree,
                displayed_items: old.displayed_items,
                display_item_ref_counter: old.display_item_ref_counter,
                default_variable_name_type: old.default_variable_name_type,
                annotations: old.annotations,
                annotation_groups: old.annotation_groups,
                annotation_counter: old.annotation_counter,
                graphics: old.graphics,
                layout_cache: Default::default(),
                flattened_rows_cache: Default::default(),
            },
            viewports: old
                .viewports
                .into_iter()
                .enumerate()
                .map(
                    |(index, viewport)| crate::tile_kinds::waveform::WaveformView {
                        scroll_offset: old.scroll_offset,
                        focused_item,
                        focused_transaction: old.focused_transaction.0.clone(),
                        selected_annotation: old
                            .selected_annotation
                            .filter(|_| index == old.last_active_viewport_idx),
                        ..viewport.into()
                    },
                )
                .collect(),
            annotation_list_visible: old.annotation_list_visible,
            last_active_viewport_idx: old.last_active_viewport_idx,
        }
    }
}
impl<'de> Deserialize<'de> for WaveformData {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut old = LegacyWaveDataV0::deserialize(deserializer)?;
        if old.viewports.is_empty() {
            old.viewports.push(Viewport::new());
        }
        if old.last_active_viewport_idx >= old.viewports.len() {
            old.last_active_viewport_idx = 0;
        }
        Ok(old.into())
    }
}
// Until the workspace envelope is installed, keep writing the legacy field names.
impl Serialize for WaveformData {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("WaveData", 21)?;
        state.serialize_field("source", &self.source)?;
        state.serialize_field("format", &self.format)?;
        state.serialize_field("active_scope", &self.active_scope)?;
        state.serialize_field("items_tree", &self.items.items_tree)?;
        state.serialize_field("displayed_items", &self.items.displayed_items)?;
        state.serialize_field(
            "display_item_ref_counter",
            &self.items.display_item_ref_counter,
        )?;
        state.serialize_field(
            "viewports",
            &self
                .viewports
                .iter()
                .map(|view| view.viewport)
                .collect::<Vec<_>>(),
        )?;
        state.serialize_field("cursor", &self.cursor)?;
        state.serialize_field("markers", &self.markers)?;
        state.serialize_field(
            "selected_annotation",
            &self.viewports[self.last_active_viewport_idx].selected_annotation,
        )?;
        state.serialize_field("annotations", &self.items.annotations)?;
        state.serialize_field("annotation_groups", &self.items.annotation_groups)?;
        state.serialize_field("annotation_list_visible", &self.annotation_list_visible)?;
        state.serialize_field("annotation_counter", &self.items.annotation_counter)?;
        state.serialize_field("last_active_viewport_idx", &self.last_active_viewport_idx)?;
        state.serialize_field(
            "focused_item",
            &self.viewports[self.last_active_viewport_idx].focused_index(&self.items),
        )?;
        state.serialize_field(
            "focused_transaction",
            &(
                self.viewports[self.last_active_viewport_idx]
                    .focused_transaction
                    .clone(),
                Option::<Transaction>::None,
            ),
        )?;
        state.serialize_field(
            "default_variable_name_type",
            &self.items.default_variable_name_type,
        )?;
        state.serialize_field(
            "scroll_offset",
            &self.viewports[self.last_active_viewport_idx].scroll_offset,
        )?;
        state.serialize_field("display_variable_indices", &self.display_variable_indices)?;
        state.serialize_field("graphics", &self.items.graphics)?;
        state.end()
    }
}

/// Ownership handoff for the application migration. Widget visibility remains
/// explicit until its corresponding kind is installed by the caller.
pub struct MigratedWaveform {
    pub document: crate::wave_data::WaveData,
    pub workspace: super::workspace::Workspace,
    pub annotation_list_visible: bool,
}

impl WaveformData {
    /// Consume the old presentation owner. Document, list content and views move
    /// into their final owners; no second mutable copy of any resource remains.
    pub fn into_workspace(
        self,
        runtime: &mut super::runtime::WorkspaceRuntime,
    ) -> Result<MigratedWaveform, super::workspace::WorkspaceError> {
        use super::{
            kind::{TileEntry, TileKind},
            layout::{Layout, LayoutFile, LayoutNode, SplitDir},
            workspace::Workspace,
        };
        use crate::tile_kinds::waveform::WaveformTile;
        let Self {
            document,
            items,
            mut viewports,
            last_active_viewport_idx,
            annotation_list_visible,
        } = self;
        if viewports.is_empty() {
            viewports.push(Viewport::new().into());
        }
        let scroll_offset = viewports
            .get(last_active_viewport_idx)
            .unwrap_or(&viewports[0])
            .scroll_offset;
        items.layout_cache.take();
        items.flattened_rows_cache.take();
        let list_id = runtime.allocate_list()?;
        let mut tiles = std::collections::BTreeMap::new();
        let mut order = Vec::with_capacity(viewports.len());
        for (index, mut view) in viewports.into_iter().enumerate() {
            let focus = view.focus_snapshot(&items);
            view.reset_runtime();
            view.scroll_offset = scroll_offset;
            view.reconcile_item_focus(&items, focus);
            let tile_id = runtime.allocate_tile()?;
            let mut tile = WaveformTile::new(list_id);
            tile.view = view;
            tile.link_vertical_scroll = true;
            tile.show_name_column = index == 0;
            tile.show_value_column = index == 0;
            tile.name_column_width = 100.0;
            tile.value_column_width = 100.0;
            tiles.insert(
                tile_id,
                TileEntry {
                    title: None,
                    kind: TileKind::Waveform(Box::new(tile)),
                },
            );
            order.push(tile_id);
        }
        let focused = order
            .get(last_active_viewport_idx)
            .copied()
            .unwrap_or(order[0]);
        let children = order
            .iter()
            .copied()
            .map(LayoutNode::Tile)
            .collect::<Vec<_>>();
        let root = if children.len() == 1 {
            children.into_iter().next()
        } else {
            Some(LayoutNode::Split {
                dir: SplitDir::Horizontal,
                shares: vec![1.0; children.len()],
                children,
            })
        };
        let layout = Layout::from_file(
            LayoutFile {
                root,
                focused: Some(focused),
                focus_history: vec![focused],
            },
            &order.into_iter().collect(),
        )?;
        let workspace = Workspace {
            layout,
            tiles,
            item_lists: std::collections::BTreeMap::from([(list_id, items)]),
        };
        Ok(MigratedWaveform {
            document,
            workspace,
            annotation_list_visible,
        })
    }
}
