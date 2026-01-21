# Waveform Tile Clipping Issue

## Problem

When using egui_tiles for the tile-based layout, waveform tile content rendered outside its tile boundary, overlapping with tiles below or the status bar. This occurred when resizing tiles to leave little vertical space.

## Root Cause

egui_tiles creates tile UIs with `Ui::new()` which doesn't inherit the parent's `clip_rect`. Nested SidePanel/CentralPanel UIs also don't inherit clip from the tile boundary, allowing content to spill outside.

Additionally, SidePanel `Frame::fill()` draws backgrounds *before* the callback, so we couldn't constrain clipping before the background rendered.

## Solution

All changes localized to `waveform_tile.rs`:

1. **Capture tile boundary**: `let tile_clip = ui.max_rect()` at start of `draw_waveform_tile`

2. **Re-apply clip in panel callbacks**: Each SidePanel/CentralPanel callback calls `ui.set_clip_rect(ui.clip_rect().intersect(tile_clip))` to constrain child UIs to the tile boundary

3. **Manual background for variable values**: Draw background with `ui.painter().with_clip_rect()` instead of `Frame::fill()` to respect the clipped region

## Key Insight

Use `ui.max_rect()` as the authoritative tile boundary and re-apply it inside each nested panel callback since egui panels don't inherit clip_rect from parents.
