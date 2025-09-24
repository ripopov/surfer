---
inclusion: manual
---

# Task: Implement PNG Export Feature

## Clear Objective

Implement the PNG export functionality for waveform views in the Surfer project, enabling the export of specific views or multiple regions of a waveform as PNG images via command files, and providing a GUI menu item for interactive export of the current plot to PNG. This also includes implementing a `set_time_range` command that can target specific viewports, and a `--headless` CLI argument for running Surfer without a GUI in CI environments.

## Acceptance Criteria

- **Headless Mode:**
  - The `surfer` executable can be invoked with a `--headless` argument, preventing the GUI from launching.
- **Command File Export:**
  - The `surfer` executable can process command files containing `export_png` commands with parameters to specify regions or views, saving one or more PNG images of the waveform views at the specified path(s).
  - Each exported PNG accurately reflects the loaded waveform, signals, cursors, and markers within its specified region or view.
  - The `export_png` command supports exporting a single specified viewport, or by default, all viewports either as a single combined PNG or as separate PNGs.
  - The command file execution exits without errors on successful export and with an informative error message on failure (e.g., invalid path, rendering error, invalid region specification).
- **GUI Menu Export:**
  - A new menu item (e.g., "File -> Export Plot as PNG...") is available in the Surfer GUI.
  - Selecting this menu item opens a file dialog, allowing the user to choose the output path and filename.
  - The current waveform plot is accurately exported as a PNG image to the specified location upon confirmation.
  - A confirmation message is displayed on successful GUI export, and an informative error message is displayed on failure.
- **Viewport Zoom Control:**
  - The `surfer` executable can process command files containing `set_time_range <START_TIME> <END_TIME> [VIEWPORT_ID]` commands.
  - When `VIEWPORT_ID` is specified, the zoom is applied only to that viewport.
  - When `VIEWPORT_ID` is omitted, the zoom is applied to the currently active viewport.

## Requirements Traceability

- Traces to: User Story: Export Waveform as PNG (`.work-items/png-export/user-story.md`)
- Traces to: Design Document: PNG Export Feature (`.work-items/png-export/design.md`)

## Test Strategy

- Create new integration tests that load a sample waveform, apply some commands (e.g., adding signals, cursors, `set_time_range` for specific viewports), and then execute a command file with `export_png` commands, including parameters for multiple regions or views and viewport selection.
- Verify that multiple PNG files are created at the specified paths, with appropriate naming conventions (e.g., suffixed), and that each accurately represents its specified area.
- Create new GUI-specific tests (if applicable to the testing framework) to verify the functionality of the "Export Plot as PNG..." menu item, including file dialog interaction and successful image generation.
- (Future step) Compare the generated PNGs with reference images to ensure visual correctness for both command file and GUI exports.

## Task Breakdown Methodology: Sequential, ACID-Compliant Steps

This task will be broken down into the following sequential, ACID-Compliant Steps:

- `01_create_export_module.md`: Create the `libsurfer/src/export.rs` module with a basic `export_png` function, designed to be extensible for region/view specifications and current view export.
- `02_integrate_export_into_libsurfer.md`: Expose the `export_png` function and a new `set_zoom_viewport` function through `libsurfer/src/lib.rs`, ensuring they can accept parameters for region/view specifications and viewport selection.
- `03_add_command_file_parsing.md`: Implement parsing for the new `export_png` and `set_time_range` commands and their associated arguments (including viewport ID and region/view specifications) within `surfer/src/main.rs`.
- `04_implement_command_file_export_logic.md`: Implement the logic within `libsurfer/src/export.rs` to handle multiple region/view specifications and viewport selection from command files, configure the `SystemState` for each, and export individual PNGs.
- `05_implement_set_zoom_logic.md`: Implement the logic for the `set_time_range` command within `libsurfer/src/export.rs` (or a related module) to adjust the zoom for a specified or active viewport.
- `06_add_gui_menu_item.md`: Implement the "File -> Export Plot as PNG..." menu item in the Surfer GUI, including file dialog interaction.
- `07_integrate_gui_export_logic.md`: Integrate the `libsurfer` export function with the GUI menu item to export the current plot, considering viewport selection.
- `08_add_headless_cli_argument.md`: Implement the `--headless` CLI argument in `surfer/src/main.rs` to prevent GUI launch.
- `09_add_integration_tests.md`: Create comprehensive integration tests to verify the command file export functionality (including `set_time_range` and `export_png` with viewport options).
- `10_add_gui_tests.md`: Create tests for the GUI menu item export functionality (if applicable to the testing framework).
