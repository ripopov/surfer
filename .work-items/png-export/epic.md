---
inclusion: manual
---

# Epic: Implement PNG Export Feature

## Clear Objective

Implement the PNG export functionality for waveform views in the Surfer project, enabling the export of specific views or multiple regions of a waveform as PNG images via command files, and providing a GUI menu item for interactive export of the current plot to PNG. This also includes implementing a `set_time_range` command that can target specific viewports, and a `--headless` CLI argument for running Surfer without a GUI in CI environments.

## Acceptance Criteria

- **Overall PNG Export Functionality:**
  - Users can export waveform views as PNG images, either interactively via the GUI or programmatically via command files.
  - Exports accurately reflect the loaded waveform, signals, cursors, markers, and applied themes.
  - The system handles various export scenarios, including specific regions, multiple views, and viewport selection.
- **Headless Operation:**
  - Surfer can execute export commands without launching a GUI.
- **Viewport Control:**
  - Users can control viewport zoom levels via command files.

## Requirements Traceability

- Traces to: User Story: Export Waveform as PNG (`.work-items/png-export/user-story.md`)
- Traces to: Design Document: PNG Export Feature (`.work-items/png-export/design.md`)

## Test Strategy

- Comprehensive integration tests will verify command file-driven exports, including `set_time_range` and `export_png` with various parameters.
- GUI-specific tests will validate the interactive "Export Plot as PNG..." menu item.
- Visual correctness of generated PNGs will be ensured through comparison with reference images (future step).

## Related Features

- [UI Export Plot as PNG](task-ui-export.md)
- [Headless Export Plot as PNG](task-headless-export.md)
