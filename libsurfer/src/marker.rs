use ecolor::Color32;
use egui::{RichText, WidgetText};
use egui_extras::{Column, TableBuilder};
use emath::{Align2, Pos2, Rect};
use epaint::{CornerRadius, FontId, Stroke};
use itertools::Itertools;
use num::{BigInt, Zero};

use crate::drawing_canvas::draw_vertical_line_at_time;
use crate::{
    config::SurferTheme,
    displayed_item::{DisplayedItem, DisplayedItemRef, DisplayedMarker},
    item_drawing_info::ItemDrawingInfo,
    item_list::ItemList,
    message::Message,
    time::TimeFormatter,
    view::DrawingContext,
    viewport::Viewport,
    wave_data::WaveData,
};

pub const DEFAULT_MARKER_NAME: &str = "Marker";
const MAX_MARKERS: usize = 255;
const MAX_MARKER_INDEX: u8 = 254;
const CURSOR_MARKER_IDX: u8 = 255;

impl crate::SystemState {
    /// Apply an edit that changes one shared marker time and/or its row in the
    /// target waveform's list. The record retains only that marker's previous
    /// value and that list, so other markers and views are untouched by undo.
    pub(crate) fn edit_shared_marker(
        &mut self,
        label: String,
        known: Option<u8>,
        edit: impl FnOnce(&mut crate::wave_data::WaveformEdit<'_>) -> Option<()>,
    ) -> Option<()> {
        let target = self
            .user
            .workspace
            .resolve_waveform(crate::tiles::TileTarget::Focused)?;
        let list = self.user.workspace.tiles[&target].kind.item_list()?;
        let before_markers = self.user.waves.as_ref()?.markers.clone();
        let items = &self.user.workspace.item_lists[&list];
        let before = Self::current_canvas_state(list, items, label);
        let rows_before = items.displayed_items.len();
        let mut waves = self.user.waveform_edit_at(target)?;
        edit(&mut waves)?;
        let after = &self.user.waves.as_ref()?.markers;
        let changed = known
            .into_iter()
            .chain(before_markers.keys().copied())
            .chain(after.keys().copied())
            .find(|id| before_markers.get(id) != after.get(id));
        let rows_changed =
            rows_before != self.user.workspace.item_lists[&list].displayed_items.len();
        let id = match changed {
            Some(id) => id,
            None if rows_changed => known?,
            None => return None,
        };
        let time = before_markers.get(&id).cloned();
        self.record_edit(crate::tiles::history::UndoRecord::Marker {
            id,
            time,
            lists: vec![before],
        });
        self.invalidate_draw_commands();
        Some(())
    }

    pub(crate) fn remove_shared_marker(&mut self, id: u8) -> Option<()> {
        let time = self.user.waves.as_ref()?.markers.get(&id).cloned();
        let affected = self
            .user
            .workspace
            .item_lists
            .iter()
            .filter_map(|(list, items)| {
                let rows = items
                    .displayed_items
                    .iter()
                    .filter_map(|(row, item)| {
                        matches!(item, DisplayedItem::Marker(marker) if marker.idx == id)
                            .then_some(*row)
                    })
                    .collect::<Vec<_>>();
                (!rows.is_empty()).then_some((*list, rows))
            })
            .collect::<Vec<_>>();
        if time.is_none() && affected.is_empty() {
            return None;
        }
        let mut lists = Vec::new();
        for (list, rows) in affected {
            let items = self.user.workspace.item_lists.get_mut(&list).unwrap();
            lists.push(Self::current_canvas_state(
                list,
                items,
                "Remove marker".into(),
            ));
            let mut views = self
                .user
                .workspace
                .tiles
                .values_mut()
                .filter_map(|entry| match &mut entry.kind {
                    crate::tiles::kind::TileKind::Waveform(tile) if tile.items == list => {
                        Some(&mut tile.view)
                    }
                    _ => None,
                })
                .map(|view| {
                    let focus = view.focus_snapshot(items);
                    (view, focus)
                })
                .collect::<Vec<_>>();
            items.remove_items(&rows);
            for (view, focus) in &mut views {
                view.reconcile_item_focus(items, *focus);
                view.reconcile_annotations(items);
                view.invalidate_draw_cache();
            }
        }
        self.user.waves.as_mut().unwrap().markers.remove(&id);
        self.record_edit(crate::tiles::history::UndoRecord::Marker { id, time, lists });
        self.invalidate_draw_commands();
        Some(())
    }
}

impl crate::wave_data::WaveformEdit<'_> {
    pub fn add_marker(
        &mut self,
        location: &BigInt,
        name: Option<String>,
        move_focus: bool,
    ) -> Option<DisplayedItemRef> {
        if !self.can_add_marker() {
            return None;
        }

        let Some(idx) = (0..=MAX_MARKER_INDEX).find(|idx| !self.markers.contains_key(idx)) else {
            // This shouldn't happen since can_add_marker() was already checked,
            // but handle it gracefully
            return None;
        };

        let item_ref = self
            .insert_item(
                DisplayedItem::Marker(DisplayedMarker {
                    color: None,
                    background_color: None,
                    name,
                    idx,
                }),
                None,
                move_focus,
            )
            .ok()?;
        self.markers.insert(idx, location.clone());

        Some(item_ref)
    }

    /// Set the marker with the specified id to the location.
    ///
    /// If the marker doesn't exist already, it will be created.
    pub fn set_marker_position(
        &mut self,
        idx: u8,
        location: &BigInt,
    ) -> Result<(), crate::item_list::ItemEditError> {
        if !self.markers.contains_key(&idx) {
            self.insert_item(
                DisplayedItem::Marker(DisplayedMarker {
                    color: None,
                    background_color: None,
                    name: None,
                    idx,
                }),
                None,
                true,
            )?;
        }
        self.markers.insert(idx, location.clone());
        Ok(())
    }

    pub fn move_marker_to_cursor(
        &mut self,
        idx: u8,
    ) -> Result<(), crate::item_list::ItemEditError> {
        if let Some(location) = self.cursor.clone() {
            self.set_marker_position(idx, &location)?;
        }
        Ok(())
    }
}

impl crate::tile_kinds::waveform_services::WaveformReadServices<'_> {
    pub(crate) fn draw_marker_table(
        &self,
        waves: &crate::wave_data::WaveformRead<'_>,
        ui: &mut egui::Ui,
        msgs: &mut Vec<Message>,
    ) {
        // Construct markers list: cursor first (if present), then numbered markers
        let markers: Vec<(u8, &BigInt, WidgetText)> = waves
            .cursor
            .as_ref()
            .into_iter()
            .map(|cursor| {
                (
                    CURSOR_MARKER_IDX,
                    cursor,
                    WidgetText::RichText(RichText::new("Primary").into()),
                )
            })
            .chain(
                waves
                    .items
                    .items_tree
                    .iter()
                    .filter_map(|node| waves.items.displayed_items.get(&node.item_ref))
                    .filter_map(|displayed_item| match displayed_item {
                        DisplayedItem::Marker(marker) => {
                            let text_color = self.get_item_text_color(displayed_item);
                            Some((
                                marker.idx,
                                waves.numbered_marker_time(marker.idx),
                                marker.marker_text(text_color),
                            ))
                        }
                        _ => None,
                    })
                    .sorted_by(|a, b| Ord::cmp(&a.0, &b.0)),
            )
            .collect();

        ui.vertical_centered(|ui| {
            // Table of markers: header row then rows of time differences.
            let row_height = ui.text_style_height(&egui::TextStyle::Body);
            TableBuilder::new(ui)
                .striped(true)
                .cell_layout(egui::Layout::right_to_left(emath::Align::TOP))
                .columns(Column::auto().resizable(true), markers.len() + 1)
                .auto_shrink(emath::Vec2b::new(false, true))
                .header(row_height, |mut header| {
                    header.col(|ui| {
                        ui.label("");
                    });
                    for (marker_idx, _, widget_text) in &markers {
                        header.col(|ui| {
                            if ui
                                .add(
                                    egui::Label::new(widget_text.clone())
                                        .sense(egui::Sense::click()),
                                )
                                .clicked()
                            {
                                msgs.push(marker_click_message(
                                    *marker_idx,
                                    waves.cursor.as_ref(),
                                    waves.tile_id,
                                ));
                            }
                        });
                    }
                })
                .body(|body| {
                    let time_formatter = TimeFormatter::new(
                        &waves.inner.metadata().timescale,
                        &self.wanted_timeunit,
                        &self.time_format,
                    );
                    let numbber_of_markers = markers.len();
                    body.rows(row_height, numbber_of_markers, |mut row| {
                        let row_idx = row.index();
                        let (marker_idx, row_marker_time, row_widget_text) = &markers[row_idx];
                        row.col(|ui| {
                            if ui
                                .add(
                                    egui::Label::new(row_widget_text.clone())
                                        .sense(egui::Sense::click()),
                                )
                                .clicked()
                            {
                                msgs.push(marker_click_message(
                                    *marker_idx,
                                    waves.cursor.as_ref(),
                                    waves.tile_id,
                                ));
                            }
                        });
                        for (_, col_marker_time, _) in &markers {
                            let diff =
                                time_formatter.format(&(*row_marker_time - *col_marker_time));
                            row.col(|ui| {
                                ui.label(diff);
                            });
                        }
                    });
                });
        });
    }
}

/// Get the background color for a marker or cursor, with fallback to theme cursor color
fn get_marker_background_color(item: &DisplayedItem, theme: &SurferTheme) -> Color32 {
    item.color()
        .and_then(|color| theme.get_color(color))
        .unwrap_or(theme.cursor.color)
}

/// Generate the message for a marker click based on its index
fn marker_click_message(
    marker_idx: u8,
    cursor: Option<&BigInt>,
    tile_id: crate::tiles::TileId,
) -> Message {
    if marker_idx < CURSOR_MARKER_IDX {
        Message::GoToMarkerPosition(marker_idx, tile_id)
    } else {
        Message::GoToTime(cursor.cloned(), tile_id)
    }
}

impl ItemList {
    #[must_use]
    pub fn resolve_marker_name(&self, name: &str) -> Option<u8> {
        if let Some(id_str) = name.strip_prefix('#') {
            return id_str.parse::<u8>().ok();
        }

        self.displayed_items.values().find_map(|item| match item {
            DisplayedItem::Marker(marker) if marker.name.as_deref() == Some(name) => {
                Some(marker.idx)
            }
            _ => None,
        })
    }

    /// Get the color for a marker by its index, falling back to cursor color if not found
    fn get_marker_color(&self, idx: u8, theme: &SurferTheme) -> Color32 {
        self.items_tree
            .iter()
            .find_map(|node| {
                if let Some(DisplayedItem::Marker(marker)) =
                    self.displayed_items.get(&node.item_ref)
                    && marker.idx == idx
                {
                    return marker
                        .color
                        .as_ref()
                        .and_then(|color| theme.get_color(color));
                }
                None
            })
            .unwrap_or(theme.cursor.color)
    }

    pub fn draw_markers(
        &self,
        document: &WaveData,
        theme: &SurferTheme,
        ctx: &mut DrawingContext,
        viewport: &Viewport,
    ) {
        let range = document.time_range();
        for (idx, marker_time) in &document.markers {
            let color = self.get_marker_color(*idx, theme);
            let stroke = Stroke {
                color,
                width: theme.cursor.width,
            };
            draw_vertical_line_at_time(marker_time, ctx, stroke, viewport, range);
        }
    }

    /// Draw text with background box at the specified position
    /// Returns the text and its background rectangle info for reuse if needed
    fn draw_text_with_background(
        ctx: &mut DrawingContext,
        x: f32,
        text: &str,
        background_color: Color32,
        foreground_color: Color32,
        padding: f32,
    ) {
        let y = ctx.cfg.canvas_size.y * 0.5;
        let text_size = ctx.cfg.text_size;
        // Measure text first
        let rect = ctx.painter.text(
            (ctx.to_screen)(x, y),
            Align2::CENTER_CENTER,
            text,
            FontId::proportional(text_size),
            foreground_color,
        );

        // Background rectangle with padding
        let min = Pos2::new(rect.min.x - padding, rect.min.y - padding);
        let max = Pos2::new(rect.max.x + padding, rect.max.y + padding);

        ctx.painter
            .rect_filled(Rect { min, max }, CornerRadius::ZERO, background_color);

        // Draw text on top of background
        ctx.painter.text(
            (ctx.to_screen)(x, y),
            Align2::CENTER_CENTER,
            text,
            FontId::proportional(text_size),
            foreground_color,
        );
    }

    pub fn draw_marker_number_boxes(
        &self,
        document: &WaveData,
        ctx: &mut DrawingContext,
        theme: &SurferTheme,
        viewport: &Viewport,
    ) {
        for displayed_item in self
            .items_tree
            .iter_visible()
            .map(|node| self.displayed_items.get(&node.item_ref))
            .filter_map(|item| match item {
                Some(DisplayedItem::Marker(marker)) => Some(marker),
                _ => None,
            })
        {
            let item = DisplayedItem::Marker(displayed_item.clone());
            let background_color = get_marker_background_color(&item, theme);

            let x = document.numbered_marker_location(
                displayed_item.idx,
                viewport,
                ctx.cfg.canvas_size.x,
            );
            let idx_string = displayed_item.idx.to_string();

            Self::draw_text_with_background(
                ctx,
                x,
                &idx_string,
                background_color,
                theme.foreground,
                2.0,
            );
        }
    }
}

impl WaveData {
    pub fn draw_cursor(&self, theme: &SurferTheme, ctx: &mut DrawingContext, viewport: &Viewport) {
        if let Some(cursor_time) = &self.cursor {
            let range = self.time_range();
            draw_vertical_line_at_time(cursor_time, ctx, &theme.cursor, viewport, range);
        }
    }

    #[must_use]
    pub fn can_add_marker(&self) -> bool {
        self.markers.len() < MAX_MARKERS
    }
}

impl crate::tile_kinds::waveform_services::WaveformReadServices<'_> {
    pub fn draw_marker_boxes(
        &self,
        waves: &WaveData,
        items: &ItemList,
        ctx: &mut DrawingContext,
        viewport: &Viewport,
        row_offset: f32,
    ) {
        let horizontal_padding = self.config.layout.waveforms_gap;

        let time_formatter = TimeFormatter::new(
            &waves.inner.metadata().timescale,
            &self.wanted_timeunit,
            &self.time_format,
        );
        let visible_top = -row_offset;
        let visible_bottom = ctx.cfg.canvas_size.y - row_offset;
        for drawing_info in items
            .visible_drawing_infos(visible_top, visible_bottom)
            .iter()
            .filter_map(|item| match item {
                ItemDrawingInfo::Marker(marker) => Some(marker),
                _ => None,
            })
        {
            let Some(item) = items
                .items_tree
                .get_visible(drawing_info.vidx)
                .and_then(|node| items.displayed_items.get(&node.item_ref))
            else {
                continue;
            };

            let row_top = drawing_info.top + row_offset;
            let row_bottom = drawing_info.bottom + row_offset;

            let background_color = get_marker_background_color(item, &self.config.theme);

            let x =
                waves.numbered_marker_location(drawing_info.idx, viewport, ctx.cfg.canvas_size.x);

            // Time string
            let time = time_formatter.format(
                waves
                    .markers
                    .get(&drawing_info.idx)
                    .unwrap_or(&BigInt::zero()),
            );

            let text_color = self.config.theme.get_best_text_color(background_color);

            // Create galley
            let galley = ctx.painter.layout_no_wrap(
                time,
                FontId::proportional(ctx.cfg.text_size),
                text_color,
            );
            let offset_width = galley.rect.width() * 0.5 + horizontal_padding;

            // Background rectangle
            let min = (ctx.to_screen)(x - offset_width, row_top);
            let max = (ctx.to_screen)(x + offset_width, row_bottom);

            ctx.painter
                .rect_filled(Rect { min, max }, CornerRadius::ZERO, background_color);

            // Draw actual text on top of rectangle
            ctx.painter.galley(
                (ctx.to_screen)(
                    x - galley.rect.width() * 0.5,
                    (row_top + row_bottom - galley.rect.height()) * 0.5,
                ),
                galley,
                text_color,
            );
        }
    }
}
