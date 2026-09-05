use std::cell::RefCell;

use num::ToPrimitive;
use serde::{Deserialize, Serialize};

use crate::{drawing_canvas::WaveDrawCache, viewport::Viewport};

#[derive(Debug, Deserialize)]
pub enum WaveformMessage {
    Annotation(super::annotation_list::AnnotationCommand),
    AddDivider {
        name: Option<String>,
        position: crate::displayed_item_tree::TargetPosition,
    },
    MoveItems {
        items: Vec<crate::displayed_item::DisplayedItemRef>,
        position: crate::displayed_item_tree::TargetPosition,
    },
    RemoveItems(Vec<crate::displayed_item::DisplayedItemRef>),
    Navigate(WaveformNavigation),
    FocusItem(Option<crate::displayed_item::DisplayedItemRef>),
    Selection(crate::item_list::ItemSelection),
    FocusTransaction(Option<crate::transaction_container::TransactionRef>),
    MoveTransaction {
        next: bool,
    },
    ScrollTo(f32),
    ScrollEdge {
        end: bool,
    },
    ScrollRows {
        down: bool,
        count: usize,
    },
    LinkVerticalScroll(bool),
    Columns {
        names: bool,
        values: bool,
    },
    ColumnWidths {
        names: f32,
        values: f32,
    },
}

impl WaveformMessage {
    /// Content history belongs to the referenced list; view navigation is excluded.
    pub(crate) fn item_edit_label(&self) -> Option<&'static str> {
        match self {
            Self::Annotation(_) => Some("Edit annotation"),
            Self::AddDivider { .. } => Some("Add divider"),
            Self::MoveItems { .. } => Some("Move items"),
            Self::RemoveItems(_) => Some("Remove items"),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub enum WaveformNavigation {
    Pan(f64),
    Zoom {
        factor: f64,
        anchor: Option<num::BigInt>,
    },
    ZoomToCursor {
        factor: f64,
    },
    ZoomToFit,
    GoToStart,
    GoToEnd,
    GoToTime(num::BigInt),
    ZoomToRange {
        start: num::BigInt,
        end: num::BigInt,
    },
}

impl WaveformView {
    /// Validate a candidate before changing persistent navigation or runtime caches.
    pub(crate) fn apply_navigation(
        &mut self,
        command: WaveformNavigation,
        document: Option<&crate::wave_data::WaveData>,
    ) -> Result<bool, WaveformPayloadError> {
        let valid_time = |time: &num::BigInt| time.to_f64().is_some_and(f64::is_finite);
        let valid_factor = |factor: f64| factor.is_finite() && factor > 0.0;
        let mut viewport = self.viewport;
        match command {
            WaveformNavigation::Pan(delta) => {
                if !delta.is_finite() {
                    return Err(WaveformPayloadError::InvalidViewport);
                }
                viewport.handle_canvas_scroll(delta);
            }
            WaveformNavigation::ZoomToFit => viewport.zoom_to_fit(),
            WaveformNavigation::GoToStart => viewport.go_to_start(),
            WaveformNavigation::GoToEnd => viewport.go_to_end(),
            command => {
                let Some(document) = document.filter(|document| document.max_timestamp().is_some())
                else {
                    return Ok(false);
                };
                let range = document.time_range();
                match command {
                    WaveformNavigation::Zoom { factor, anchor } => {
                        if !valid_factor(factor)
                            || anchor.as_ref().is_some_and(|time| !valid_time(time))
                        {
                            return Err(WaveformPayloadError::InvalidViewport);
                        }
                        viewport.handle_canvas_zoom(anchor, factor, range);
                    }
                    WaveformNavigation::ZoomToCursor { factor } => {
                        if !valid_factor(factor) {
                            return Err(WaveformPayloadError::InvalidViewport);
                        }
                        let Some(cursor) = &document.cursor else {
                            return Ok(false);
                        };
                        if !valid_time(cursor) {
                            return Err(WaveformPayloadError::InvalidViewport);
                        }
                        viewport.zoom_to_time(cursor, factor, range);
                    }
                    WaveformNavigation::GoToTime(time) => {
                        if !valid_time(&time) {
                            return Err(WaveformPayloadError::InvalidViewport);
                        }
                        viewport.go_to_time(&time, range);
                    }
                    WaveformNavigation::ZoomToRange { start, end } => {
                        if start >= end || !valid_time(&start) || !valid_time(&end) {
                            return Err(WaveformPayloadError::InvalidViewport);
                        }
                        viewport.zoom_to_range(&start, &end, range);
                    }
                    _ => unreachable!("relative navigation handled above"),
                }
            }
        }
        if !viewport.valid_navigation() {
            return Err(WaveformPayloadError::InvalidViewport);
        }
        if viewport == self.viewport {
            return Ok(false);
        }
        self.viewport = viewport;
        self.invalidate_draw_cache();
        Ok(true)
    }
}

/// Registry-checked access to this waveform's list and sibling views of that list.
pub(crate) struct WaveformUpdateCtx<'a> {
    pub document: Option<&'a crate::wave_data::WaveData>,
    pub items: &'a mut crate::item_list::ItemList,
    pub visible: bool,
    pub peers: Vec<(&'a mut WaveformTile, bool)>,
}

impl WaveformTile {
    pub(crate) fn update(
        &mut self,
        message: WaveformMessage,
        cx: &mut WaveformUpdateCtx<'_>,
    ) -> Result<bool, WaveformPayloadError> {
        match message {
            WaveformMessage::Annotation(command) => {
                let changed = cx.items.apply_annotation_command(command)?;
                if changed {
                    self.view.reconcile_annotations(cx.items);
                    self.view.invalidate_draw_cache();
                    for (peer, _) in &mut cx.peers {
                        peer.view.reconcile_annotations(cx.items);
                        peer.view.invalidate_draw_cache();
                    }
                }
                Ok(changed)
            }

            WaveformMessage::FocusTransaction(transaction) => {
                Ok(self.view.focus_transaction(transaction))
            }
            WaveformMessage::MoveTransaction { next } => {
                Ok(self.view.move_to_transaction(cx.document, cx.items, next))
            }
            WaveformMessage::Selection(command) => {
                let changed = cx.items.apply_selection(command)?;
                if changed {
                    self.view.invalidate_draw_cache();
                    for (peer, _) in &cx.peers {
                        peer.view.invalidate_draw_cache();
                    }
                }
                Ok(changed)
            }
            WaveformMessage::AddDivider { name, position } => {
                let item = cx.items.insert_item(
                    crate::displayed_item::DisplayedItem::Divider(
                        crate::displayed_item::DisplayedDivider {
                            name,
                            color: None,
                            background_color: None,
                        },
                    ),
                    position,
                )?;
                if self.view.focused_item.is_some() {
                    self.view.focused_item = Some(item);
                }
                self.view.invalidate_draw_cache();
                for (peer, _) in &cx.peers {
                    peer.view.invalidate_draw_cache();
                }
                Ok(true)
            }
            WaveformMessage::MoveItems { items, position } => {
                if !cx.items.move_items(&items, position)? {
                    return Ok(false);
                }
                self.view.invalidate_draw_cache();
                for (peer, _) in &cx.peers {
                    peer.view.invalidate_draw_cache();
                }
                Ok(true)
            }
            WaveformMessage::RemoveItems(ids) => {
                let focused = self.view.focus_snapshot(cx.items);
                let peers = cx
                    .peers
                    .iter()
                    .map(|(peer, _)| peer.view.focus_snapshot(cx.items))
                    .collect::<Vec<_>>();
                if !cx.items.remove_items(&ids) {
                    return Ok(false);
                }
                self.view.reconcile_item_focus(cx.items, focused);
                self.view.reconcile_annotations(cx.items);
                self.view.invalidate_draw_cache();
                for ((peer, _), focused) in cx.peers.iter_mut().zip(peers) {
                    peer.view.reconcile_item_focus(cx.items, focused);
                    peer.view.reconcile_annotations(cx.items);
                    peer.view.invalidate_draw_cache();
                }
                Ok(true)
            }
            WaveformMessage::Navigate(command) => self.view.apply_navigation(command, cx.document),
            WaveformMessage::FocusItem(item) => {
                if item.is_some_and(|id| !cx.items.displayed_items.contains_key(&id)) {
                    return Err(WaveformPayloadError::InvalidFocus);
                }
                if self.view.focused_item == item {
                    return Ok(false);
                }
                self.view.focused_item = item;
                self.view.invalidate_draw_cache();
                Ok(true)
            }
            WaveformMessage::Columns { names, values } => {
                let changed = self.show_name_column != names || self.show_value_column != values;
                self.show_name_column = names;
                self.show_value_column = values;
                Ok(changed)
            }
            WaveformMessage::ColumnWidths { names, values } => {
                if [names, values]
                    .into_iter()
                    .any(|width| !width.is_finite() || width <= 0.0)
                {
                    return Err(WaveformPayloadError::InvalidColumns);
                }
                let changed = self.name_column_width != names || self.value_column_width != values;
                self.name_column_width = names;
                self.value_column_width = values;
                Ok(changed)
            }
            WaveformMessage::ScrollEdge { end } => {
                let offset = if end {
                    cx.items.layout_cache.borrow().total_height
                } else {
                    0.0
                };
                Ok(self.scroll_group(offset, cx))
            }
            WaveformMessage::ScrollRows { down, count } => {
                let current = self.view.get_top_item(cx.items);
                let target = if down {
                    current.saturating_add(count)
                } else {
                    current.saturating_sub(count)
                };
                let offset = {
                    let layout = cx.items.layout_cache.borrow();
                    layout
                        .infos
                        .get(target)
                        .or_else(|| layout.infos.last())
                        .map(|info| info.top())
                };
                Ok(offset.is_some_and(|offset| self.scroll_group(offset, cx)))
            }
            WaveformMessage::ScrollTo(offset) => {
                if !offset.is_finite() {
                    return Err(WaveformPayloadError::InvalidScroll);
                }
                Ok(self.scroll_group(offset, cx))
            }
            WaveformMessage::LinkVerticalScroll(link) => {
                if self.link_vertical_scroll == link {
                    return Ok(false);
                }
                let offset = if link {
                    cx.peers
                        .iter()
                        .find(|(peer, _)| peer.link_vertical_scroll)
                        .map_or(self.view.scroll_offset, |(peer, _)| peer.view.scroll_offset)
                } else {
                    self.view.scroll_offset
                };
                self.link_vertical_scroll = link;
                self.scroll_group(offset, cx);
                Ok(true)
            }
        }
    }

    fn scroll_group(&mut self, requested: f32, cx: &mut WaveformUpdateCtx<'_>) -> bool {
        let mut offset = requested.max(0.0);
        // Until rows and viewport geometry have been measured, retain navigation.
        let layout = cx.items.layout_cache.borrow();
        if layout.signature.is_some() {
            let mut clamp_for = |tile: &WaveformTile, visible: bool| {
                if visible
                    && tile.view.viewport_height.is_finite()
                    && tile.view.viewport_height > 0.0
                {
                    offset = offset.min((layout.total_height - tile.view.viewport_height).max(0.0));
                }
            };
            clamp_for(self, cx.visible);
            if self.link_vertical_scroll {
                for (peer, visible) in &cx.peers {
                    if peer.link_vertical_scroll {
                        clamp_for(peer, *visible);
                    }
                }
            }
        }
        let update = |view: &mut WaveformView| {
            if view.scroll_offset == offset {
                return false;
            }
            view.scroll_offset = offset;
            view.invalidate_draw_cache();
            true
        };
        let mut changed = update(&mut self.view);
        if self.link_vertical_scroll {
            for (peer, _) in &mut cx.peers {
                if peer.link_vertical_scroll {
                    changed |= update(&mut peer.view);
                }
            }
        }
        changed
    }
}

/// Disposable drag payload. Row identities are captured once, before any list edits.
pub(crate) enum WaveformDrag {
    Rows {
        tile_id: crate::tiles::TileId,
        items: Vec<crate::displayed_item::DisplayedItemRef>,
    },
    Variables(Vec<crate::wave_container::VariableRef>),
}

impl WaveformDrag {
    pub fn accepts(&self, tile_id: crate::tiles::TileId) -> bool {
        match self {
            Self::Rows {
                tile_id: source, ..
            } => *source == tile_id,
            Self::Variables(_) => true,
        }
    }
}

/// One waveform view owns both its time navigation and disposable draw data.
pub struct WaveformView {
    pub viewport: Viewport,
    pub interaction: WaveformInteraction,
    pub scroll_offset: f32,
    pub viewport_height: f32,
    pub focused_item: Option<crate::displayed_item::DisplayedItemRef>,
    pub focused_transaction: Option<crate::transaction_container::TransactionRef>,
    pub selected_annotation: Option<egui::Id>,
    pub annotation_menu: Option<(egui::Pos2, num::BigInt)>,
    pub(crate) draw_cache: RefCell<WaveDrawCache>,
}

#[derive(Clone, Copy)]
pub(crate) struct FocusAnchor {
    id: crate::displayed_item::DisplayedItemRef,
    index: Option<crate::displayed_item_tree::VisibleItemIndex>,
}

#[derive(Default)]
pub struct WaveformInteraction {
    pub gesture_start_location: Option<egui::Pos2>,
    pub gesture_start_time: Option<num::BigInt>,
    pub measure_start_location: Option<egui::Pos2>,
    pub annotation_kind: Option<crate::mousegestures::AnnotationKind>,
}

impl WaveformView {
    pub(crate) fn reset_runtime(&mut self) {
        self.interaction = WaveformInteraction::default();
        self.annotation_menu = None;
        self.selected_annotation = None;
        self.viewport_height = 0.0;
        self.invalidate_draw_cache();
    }

    pub(crate) fn focused_index(
        &self,
        items: &crate::item_list::ItemList,
    ) -> Option<crate::displayed_item_tree::VisibleItemIndex> {
        self.focused_item
            .and_then(|id| items.get_displayed_item_index(&id))
    }

    pub(crate) fn focus_snapshot(&self, items: &crate::item_list::ItemList) -> Option<FocusAnchor> {
        self.focused_item.map(|id| FocusAnchor {
            id,
            index: self.focused_index(items),
        })
    }

    pub(crate) fn reconcile_item_focus(
        &mut self,
        items: &crate::item_list::ItemList,
        previous: Option<FocusAnchor>,
    ) {
        self.focused_item = previous.and_then(|anchor| {
            if items.displayed_items.contains_key(&anchor.id) {
                return Some(anchor.id);
            }
            anchor
                .index
                .and_then(|index| items.items_tree.get_visible(index))
                .or_else(|| items.items_tree.iter_visible().last())
                .map(|node| node.item_ref)
        });
    }

    pub(crate) fn invalidate_draw_cache(&self) {
        *self.draw_cache.borrow_mut() = WaveDrawCache::default();
    }

    pub(crate) fn reconcile_annotations(&mut self, items: &crate::item_list::ItemList) {
        if self
            .selected_annotation
            .is_some_and(|id| items.get_annotation_by_id(&id).is_none())
        {
            self.selected_annotation = None;
            self.annotation_menu = None;
        }
    }

    /// Find the top-most of the currently visible items.
    ///
    /// Returns the index of the item currently at the top of the visible area.
    #[must_use]
    pub fn get_top_item(&self, items: &crate::item_list::ItemList) -> usize {
        let layout = items.layout_cache.borrow();
        if layout.infos.is_empty() {
            return 0;
        }
        // `drawing_infos` is offset-free (canonical): the first row is always at y = 0, so
        // the visible top is simply the scroll offset.
        let visible_top = self.scroll_offset;

        // Sorted by `top()`, so binary search for the first row at or past `visible_top`.
        layout
            .infos
            .partition_point(|di| di.top() < visible_top - 1.) // 1px margin for floating-point errors
            .min(layout.infos.len() - 1)
    }

    pub fn scroll_to_item(&mut self, items: &crate::item_list::ItemList, idx: usize) {
        let layout = items.layout_cache.borrow();
        if layout.infos.is_empty() {
            return;
        }
        // `drawing_infos` is offset-free (canonical): the first row is always at y = 0, so
        // the last row's bottom is the total content height.
        let content_height = items.drawing_bottom().unwrap();

        // Don't scroll if all content fits in viewport
        let max_scroll = content_height - self.viewport_height;
        if max_scroll <= 0.0 {
            return;
        }

        let item_y = layout
            .infos
            .get(idx)
            .unwrap_or_else(|| layout.infos.last().unwrap())
            .top();

        // Clamp scroll to valid range: [0, max_scroll]
        self.scroll_offset = item_y.clamp(0.0, max_scroll);
    }
}

impl From<Viewport> for WaveformView {
    fn from(viewport: Viewport) -> Self {
        Self {
            viewport,
            interaction: WaveformInteraction::default(),
            scroll_offset: 0.0,
            viewport_height: 0.0,
            focused_item: None,
            focused_transaction: None,
            selected_annotation: None,
            annotation_menu: None,
            draw_cache: RefCell::default(),
        }
    }
}

impl Clone for WaveformView {
    fn clone(&self) -> Self {
        Self {
            scroll_offset: self.scroll_offset,
            focused_item: self.focused_item,
            focused_transaction: self.focused_transaction.clone(),
            ..self.viewport.into()
        }
    }
}

impl std::ops::Deref for WaveformView {
    type Target = Viewport;
    fn deref(&self) -> &Self::Target {
        &self.viewport
    }
}

impl std::ops::DerefMut for WaveformView {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.viewport
    }
}

/// Persistent view navigation. Geometry and draw commands are never restored.
#[derive(Serialize, Deserialize)]
pub struct WaveformViewFile {
    pub focused_item: Option<crate::displayed_item::DisplayedItemRef>,
    pub focused_transaction: Option<crate::transaction_container::TransactionRef>,
    pub viewport: Viewport,
    pub scroll_offset: f32,
}

impl From<&WaveformView> for WaveformViewFile {
    fn from(view: &WaveformView) -> Self {
        Self {
            viewport: view.viewport,
            focused_item: view.focused_item,
            focused_transaction: view.focused_transaction.clone(),
            scroll_offset: view.scroll_offset,
        }
    }
}

impl From<WaveformViewFile> for WaveformView {
    fn from(file: WaveformViewFile) -> Self {
        Self {
            scroll_offset: file.scroll_offset,
            focused_item: file.focused_item,
            focused_transaction: file.focused_transaction,
            ..file.viewport.into()
        }
    }
}

/// A waveform tile references shared content and owns one view.
#[derive(Clone)]
pub struct WaveformTile {
    pub items: crate::tiles::ItemListId,
    pub view: WaveformView,
    pub link_vertical_scroll: bool,
    pub show_name_column: bool,
    pub show_value_column: bool,
    pub name_column_width: f32,
    pub value_column_width: f32,
}

impl WaveformTile {
    pub fn new(items: crate::tiles::ItemListId) -> Self {
        Self {
            items,
            view: WaveformView::from(Viewport::new()),
            link_vertical_scroll: false,
            show_name_column: true,
            show_value_column: true,
            name_column_width: 220.0,
            value_column_width: 100.0,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct WaveformTileFile {
    pub items: crate::tiles::ItemListId,
    pub view: WaveformViewFile,
    pub link_vertical_scroll: bool,
    pub show_name_column: bool,
    pub show_value_column: bool,
    pub name_column_width: f32,
    pub value_column_width: f32,
}

#[derive(Debug, thiserror::Error)]
pub enum WaveformPayloadError {
    #[error(transparent)]
    Annotation(#[from] super::annotation_list::AnnotationEditError),
    #[error(transparent)]
    ItemEdit(#[from] crate::item_list::ItemEditError),
    #[error("waveform focus refers to a missing item")]
    InvalidFocus,
    #[error("waveform item-list ID must be nonzero")]
    InvalidList,
    #[error("waveform viewport must contain finite, ordered time windows")]
    InvalidViewport,
    #[error("waveform scroll offset must be finite and nonnegative")]
    InvalidScroll,
    #[error("waveform column widths must be finite and positive")]
    InvalidColumns,
}

impl From<&WaveformTile> for WaveformTileFile {
    fn from(tile: &WaveformTile) -> Self {
        Self {
            items: tile.items,
            view: (&tile.view).into(),
            link_vertical_scroll: tile.link_vertical_scroll,
            show_name_column: tile.show_name_column,
            show_value_column: tile.show_value_column,
            name_column_width: tile.name_column_width,
            value_column_width: tile.value_column_width,
        }
    }
}

impl TryFrom<WaveformTileFile> for WaveformTile {
    type Error = WaveformPayloadError;
    fn try_from(file: WaveformTileFile) -> Result<Self, Self::Error> {
        if file.items.0 == 0 {
            return Err(WaveformPayloadError::InvalidList);
        }
        if !file.view.viewport.valid_navigation() {
            return Err(WaveformPayloadError::InvalidViewport);
        }
        if !file.view.scroll_offset.is_finite() || file.view.scroll_offset < 0.0 {
            return Err(WaveformPayloadError::InvalidScroll);
        }
        if [file.name_column_width, file.value_column_width]
            .into_iter()
            .any(|width| !width.is_finite() || width <= 0.0)
        {
            return Err(WaveformPayloadError::InvalidColumns);
        }
        Ok(Self {
            items: file.items,
            view: file.view.into(),
            link_vertical_scroll: file.link_vertical_scroll,
            show_name_column: file.show_name_column,
            show_value_column: file.show_value_column,
            name_column_width: file.name_column_width,
            value_column_width: file.value_column_width,
        })
    }
}

impl WaveformTile {
    pub(crate) fn tab_context_menu(
        &self,
        ui: &mut egui::Ui,
        cx: &mut crate::tiles::view::TileCtx<'_>,
    ) {
        use crate::tiles::kind::TileMessage;
        ui.separator();
        let mut names = self.show_name_column;
        let mut values = self.show_value_column;
        let names_changed = ui.checkbox(&mut names, "Name column").changed();
        let values_changed = ui.checkbox(&mut values, "Value column").changed();
        if names_changed || values_changed {
            cx.send_self(TileMessage::Waveform(WaveformMessage::Columns {
                names,
                values,
            }));
        }
        let mut linked = self.link_vertical_scroll;
        if ui
            .checkbox(&mut linked, "Link vertical scrolling")
            .changed()
        {
            cx.send_self(TileMessage::Waveform(WaveformMessage::LinkVerticalScroll(
                linked,
            )));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linked_views_scroll_using_their_own_visible_height() {
        use crate::item_drawing_info::{DividerDrawingInfo, ItemDrawingInfo};
        let items = crate::item_list::ItemList::default();
        items.layout_cache.borrow_mut().infos = (0..3)
            .map(|index| {
                ItemDrawingInfo::Divider(DividerDrawingInfo {
                    vidx: crate::displayed_item_tree::VisibleItemIndex(index),
                    top: index as f32 * 100.0,
                    bottom: (index + 1) as f32 * 100.0,
                })
            })
            .collect();
        let mut first = WaveformView::from(Viewport::new());
        let mut second = first.clone();
        first.viewport_height = 100.0;
        second.viewport_height = 250.0;
        first.scroll_to_item(&items, 2);
        second.scroll_to_item(&items, 2);
        assert_eq!(first.scroll_offset, 200.0);
        assert_eq!(second.scroll_offset, 50.0);
        assert_eq!(first.get_top_item(&items), 2);
        assert_eq!(second.get_top_item(&items), 1);
        second.scroll_to_item(&items, 0);
        assert_eq!(second.scroll_offset, 0.0);
        assert_eq!(first.scroll_offset, 200.0);
        assert_eq!(items.layout_cache.borrow().infos.len(), 3);
    }

    #[test]
    fn copied_and_restored_views_keep_navigation_but_reset_draw_cache() {
        let mut view = WaveformView::from(Viewport::new());
        view.viewport.curr_left = crate::viewport::Relative(0.25);
        view.viewport.curr_right = crate::viewport::Relative(0.75);
        view.draw_cache.borrow_mut().builds = 7;
        view.scroll_offset = 70.0;
        view.focused_item = Some(crate::displayed_item::DisplayedItemRef(37));
        view.viewport_height = 600.0;
        let payload = ron::to_string(&WaveformViewFile::from(&view)).unwrap();
        let restored = WaveformView::from(ron::from_str::<WaveformViewFile>(&payload).unwrap());
        for fresh in [view.clone(), restored] {
            assert_eq!(fresh.viewport.curr_left, view.viewport.curr_left);
            assert_eq!(fresh.viewport.curr_right, view.viewport.curr_right);
            assert_eq!(fresh.draw_cache.borrow().builds, 0);
            assert_eq!(fresh.scroll_offset, 70.0);
            assert_eq!(fresh.focused_item, view.focused_item);
            assert_eq!(fresh.viewport_height, 0.0);
        }
        assert_eq!(view.draw_cache.borrow().builds, 7);
    }
}
