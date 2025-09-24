---
inclusion: manual
---

# Design Document: PNG Export Feature

## 1. Objective

To enable CI/CD Engineer Carl to specify commands within a Surfer command file to export specific views or multiple regions of a waveform as PNG images, facilitating automated, focused visual diffs of waveform files in merge requests. Additionally, to provide a GUI menu item for interactive export of the current plot to PNG.

## 2. Technical Design

The PNG export functionality will be integrated into both the `surfer` command file processing and the GUI. A new `export_png` command will be added to the command file syntax. For the GUI, a new menu item will be added to trigger the export of the current view. Both methods will leverage the existing `egui_skia_renderer` and `image` crates, which are already used for snapshot testing, to render the current `SystemState` into offscreen surfaces and then encode them as PNG files.

The core rendering logic will reside in `libsurfer` to allow for reuse. The `surfer` executable will handle parsing the command file and orchestrating the rendering and file writing process based on the `export_png` commands, including the specification of multiple regions or views to export. The GUI will handle user interaction for exporting the current view.

**Relevant Architecture Documents:** (None explicitly, as this is a new feature leveraging existing rendering capabilities. However, it aligns with the overall CLI and GUI architecture of Surfer.)

## 3. Key Changes

### 3.1. API Contracts

- **New CLI Argument:** A new command-line argument, `--headless`, will be added to the `surfer` executable. When present, this argument will prevent the GUI from launching, allowing Surfer to run in a headless mode suitable for CI/CD environments where command files are processed for tasks like PNG export.
- **New Command File Commands:**
  - `export_png <path/to/output_prefix.png> [VIEWPORT_ID] [REGION_SPECIFICATIONS]`: Export the waveform view(s) as one or more PNG images. `FILE_PATH_PREFIX` specifies the base name for the output file(s). If multiple images are exported (e.g., multiple viewports or regions), a suffix (e.g., `_viewport_0`, `_region_0`) will be appended before the `.png` extension. `VIEWPORT_ID` (optional) specifies a single viewport to export. If omitted, all viewports are considered. By default, if multiple viewports exist and no `VIEWPORT_ID` is specified, each viewport will be exported as a separate PNG. An option to combine all viewports into a single PNG will be available (details to be specified in configuration). `REGION_SPECIFICATIONS` (optional) allows defining specific time ranges or signal groups to export within the selected viewport(s). If omitted, the currently displayed view of the viewport(s) is exported.
  - `set_zoom <START_TIME> <END_TIME> [VIEWPORT_ID]`: Set the visible time range of the waveform display. `START_TIME` and `END_TIME` can be absolute times (e.g., `100ns`) or relative to the current view. `VIEWPORT_ID` (optional) specifies which viewport to apply the zoom to. If omitted, the zoom is applied to the currently active viewport.
- **New GUI Menu Item:** A new menu item (e.g., "File -> Export Plot as PNG...") will be added to the Surfer GUI. This will trigger the export of the currently displayed waveform view to a user-specified PNG file.

### 3.2. Data Models

- No new core data models are required. The existing `SystemState` will be used for rendering. However, new transient data structures may be introduced within the command file parser and `libsurfer` export function to define and manage the multiple regions or views to be exported.

### 3.3. Component Responsibilities

- **`surfer/src/main.rs`:**
  - Process command files, including parsing and executing the new `export_png` and `set_zoom` commands and their associated region/view specifications.
  - Initialize `SystemState` (potentially loading a waveform and other commands from the command file), call the `libsurfer` export function (potentially multiple times for different regions/viewports), and handle success/failure.
  - For GUI mode, handle the new menu item action, triggering the `libsurfer` export function for the current view.
  - Ensure that the GUI is not launched when processing command files in a headless export mode.
- **`libsurfer/src/export.rs` (New File):**
  - A new module `export.rs` will be created to house the `export_png` function.
  - This function will be enhanced to accept `SystemState`, an output file path prefix, desired image size, and a collection of region/view specifications, including viewport selection. It should also support exporting the current view without explicit region specifications.
  - It will iterate through the region/view specifications (or export the current view/viewports), configure the `SystemState` for each region/viewport (e.g., adjusting zoom, pan, visible signals), use `egui_skia_renderer` to create an offscreen surface, draw the `SystemState` onto it, and then use the `image` crate to encode and save the surface content as a PNG, appending a unique suffix for each exported region/viewport (if applicable).
- **`libsurfer/src/lib.rs`:**
  - Expose the enhanced `export_png` function and a new `set_zoom_viewport` function from `export.rs`.

## 4. Alternatives Considered

- **Using a separate headless executable or a direct CLI flag:** Initially considered a separate executable or a direct CLI flag for export. However, the approach of integrating export functionality directly into the main `surfer` executable via command file commands was chosen for its simplicity, reusability of code, and alignment with existing command file processing logic. This allows for more complex and reproducible export scenarios.
- **Directly using `egui_skia_renderer` in `surfer/src/main.rs`:** While possible, encapsulating the export logic in `libsurfer/src/export.rs` promotes better modularity and reusability, aligning with the existing structure where core functionalities reside in `libsurfer`.

## 5. Out of Scope

- While the initial implementation focuses on PNG, the design should allow for future expansion to other image formats (e.g., SVG, JPEG) without requiring a complete re-architecture.
- Advanced rendering options (e.g., custom background colors, specific signal visibility filters) beyond what is currently displayed in the `SystemState` (unless explicitly part of a region/view specification).

## 6. Testing Strategy

Testing for the PNG export feature will adhere to the existing project style for testing. This includes leveraging existing snapshot testing infrastructure where appropriate and ensuring new tests follow established patterns for unit and integration tests within the `libsurfer` and `surfer` crates. New tests will specifically cover the export of multiple regions and views via command files, and the export of the current view via the GUI menu item, verifying that each generated image accurately represents its specified area.
