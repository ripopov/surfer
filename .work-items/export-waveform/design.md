---
inclusion: manual
---

# Design Document: PNG Export Feature

## 1. Objective

To enable CI/CD Engineer Carl to specify commands within a Surfer command file to export specific views or multiple regions of a waveform as PNG images, facilitating automated, focused visual diffs of waveform files in merge requests. Additionally, to provide a GUI menu item for interactive export of the current plot to PNG.

## 2. Technical Design

The PNG export functionality will be integrated into both the `surfer` command file processing and the GUI. A new `export_png` command will be added to the command file syntax. For the GUI, a new menu item will be added to trigger the export of the current view. Both methods will leverage the existing `egui_skia_renderer` and `image` crates, which are already used for snapshot testing, to render the current `SystemState` into offscreen surfaces and then encode them as PNG files.

The core rendering logic will reside in `libsurfer` to allow for reuse. The `surfer` executable will handle parsing the command file and orchestrating the rendering and file writing process based on the `export_png` commands, including the specification of multiple regions or views to export. The GUI will handle user interaction for exporting the current view.

### 2.1. Terminology Clarification: 'Export' vs. 'Screenshot'

While this feature primarily captures a visual representation of the waveform plots (akin to a screenshot), the term "export" has been chosen for consistency with common software terminology for saving rendered output to a file. Alternatives like "Save Plot as PNG" or "Capture Plot as PNG" were considered. However, "export" better conveys the action of generating a file from the application's internal state, especially in the context of command-line automation where the output is a derived artifact rather than a direct screen capture. It also aligns with the broader concept of exporting data or views in various formats, even if the current scope is limited to PNG images of the plot. This distinction is important to avoid confusion with exporting raw waveform data, which would be a separate feature.

### 2.2. Platform-Specific Implementation Considerations (WASM vs. Native)

The implementation of the PNG export feature requires careful consideration of the target platform due to fundamental differences in file system access and rendering contexts between native desktop applications (macOS, Linux, Windows) and WebAssembly (WASM) environments (web browsers).

- **File I/O (`std::fs::write`):**
  - **Native:** On native platforms, `std::fs::write` (or similar file system APIs) can be used directly to save the generated PNG data to a specified file path. This is a straightforward operation with full file system access.
  - **WASM (Browser):** In a web browser environment, direct file system access is restricted for security reasons. `std::fs::write` will not work. Instead, the PNG data must be provided to the browser, typically by:
        1. Creating a JavaScript `Blob` object from the raw image byte array.
        2. Generating a temporary URL for this `Blob` using `URL.createObjectURL()`.
        3. Creating an `<a>` (anchor) HTML element, setting its `href` to the Blob URL and its `download` attribute to the desired filename.
        4. Programmatically triggering a click event on this `<a>` element to initiate a file download.
        5. Revoking the object URL using `URL.revokeObjectURL()` after the download has started to release resources.
  - **Solution:** Conditional compilation (`#[cfg(target_arch = "wasm32")]` vs. `#[cfg(not(target_arch = "wasm32"))]`) will be used to provide platform-specific implementations for the file-saving logic within the `libsurfer::export` module.

- **Rendering Context (`egui_skia_renderer`):**
  - `egui_skia_renderer` is designed to work across platforms. It abstracts away the underlying graphics API (Skia, WebGPU, OpenGL, etc.). The key is that it provides a mechanism to render the `SystemState` to an offscreen buffer, which can then be read as raw pixel data.
  - The `image_snapshot().encode_png_data()` pseudocode assumes a method that can extract the raw pixel data from the rendering surface and encode it into PNG bytes, which should be consistent across platforms where `egui_skia_renderer` operates.

- **Error Handling:**
  - Platform-specific error types may need to be mapped to a common `ExportError` enum in `libsurfer::export` to provide consistent error reporting, regardless of whether the error originated from a native file system operation or a browser API call.

- **User Experience:**
  - **Native:** A native file dialog (e.g., using `rfd` or `native-dialog` crates) provides a familiar and integrated user experience for selecting a save location.
  - **WASM:** The browser's default download mechanism will handle the file saving. A file dialog is not directly controllable by WASM code; the file will typically be saved to the user's default downloads folder, or the browser may prompt the user depending on their settings.
  - **Solution:** The GUI layer (e.g., `surfer/src/gui/app.rs`) will use platform-specific file dialog implementations (e.g., `rfd` for native, or a simple browser download trigger for WASM) to initiate the export process.

These considerations will be addressed by creating a thin abstraction layer or using conditional compilation within `libsurfer::export` to handle the platform-specific differences in file I/O, while leveraging the cross-platform capabilities of `egui_skia_renderer` for the core rendering logic.

### 2.3. User Experience and Feedback Patterns

Based on analysis of the existing Surfer codebase, the following UX patterns have been identified and will be followed for the PNG export feature:

#### 2.3.1. Existing UI Patterns in Surfer

**Current Feedback Mechanisms:**
- **Status Bar Updates:** The status bar (`libsurfer/src/statusbar.rs`) displays contextual information including file paths, timestamps, and operation status
- **Modal Dialogs:** Used for critical user decisions (e.g., `ReloadWaveformDialog`, `OpenSiblingStateFileDialog` in `libsurfer/src/dialog.rs`)
- **Progress Indicators:** File loading operations show progress information in the status bar
- **Error Handling:** Errors trigger `Message::Error` which logs to console and shows logs panel (`self.user.show_logs = true`)

**File Operation Patterns:**
- **File Dialogs:** Uses `rfd` crate with `AsyncFileDialog` for both open and save operations
- **Async Operations:** File operations are handled asynchronously using `perform_async_work`
- **Message-Based Communication:** All UI interactions use the `Message` enum system for decoupled communication

#### 2.3.2. Recommended UX Approach for PNG Export

Following modern UX best practices and Surfer's existing patterns:

**Success Feedback (Non-Intrusive):**
- **Status Bar Update:** Display export success with file path in status bar (following existing pattern)
- **No Modal Dialogs:** Avoid interrupting user workflow for routine operations
- **Brief Duration:** Status message should persist for 3-5 seconds then fade

**Error Feedback (User Attention Required):**
- **Modal Dialog:** Use existing dialog pattern for critical errors that require user action
- **Specific Error Messages:** Provide actionable error information (file permissions, disk space, etc.)
- **Error Logging:** Log detailed error information for debugging

**Implementation Pattern:**
```rust
// Success: Update status bar (non-intrusive)
match export_result {
    Ok(path) => {
        // Update status bar with success message and file path
        self.status_message = Some(format!("Exported to: {}", path.display()));
        self.status_message_timer = Some(Instant::now() + Duration::from_secs(4));
    }
    Err(e) => {
        // Show modal dialog for errors (requires user attention)
        self.show_export_error_dialog = Some(e.to_string());
    }
}
```

This approach aligns with Surfer's existing patterns while following modern UX principles of "success should be quiet, failures should be loud."

**Relevant Architecture Documents:** (None explicitly, as this is a new feature leveraging existing rendering capabilities. However, it aligns with the overall CLI and GUI architecture of Surfer.)

## 3. Key Changes

### 3.1. API Contracts

- **New CLI Argument:** A new command-line argument, `--headless`, will be added to the `surfer` executable. When present, this argument will prevent the GUI from launching, allowing Surfer to run in a headless mode suitable for CI/CD environments where command files are processed for tasks like PNG export.
- **New Command File Commands:**
  - `export_png <path/to/output_prefix.png> [OPTIONS]`: Export the waveform view(s) as one or more images. The command supports various configuration options to control the export behavior:
    - `--format <FORMAT>`: Image format (png, jpg). Default: png
    - `--size <WIDTH>x<HEIGHT>`: Output image dimensions. Default: 1280x720
    - `--dpi <DPI>`: DPI/resolution multiplier for high-resolution exports. Default: 1.0
    - `--viewport <ID>`: Export specific viewport only. If omitted, exports all viewports
    - `--hide-ui`: Hide all UI elements (menu, toolbar, side panel, status bar, overview)
    - `--show-menu`: Include menu bar in export (overrides --hide-ui for menu)
    - `--show-toolbar`: Include toolbar in export (overrides --hide-ui for toolbar)
    - `--show-side-panel`: Include side panel in export (overrides --hide-ui for side panel)
    - `--show-statusbar`: Include status bar in export (overrides --hide-ui for status bar)
    - `--show-overview`: Include overview panel in export (overrides --hide-ui for overview)
    - `--show-timeline`: Include timeline/axis in export
    - `--show-markers`: Include time markers in export
    - `--show-cursor`: Include cursor line in export
    - `--theme <THEME>`: Apply specific theme for export (dark+, light+, solarized, etc.)
    - `--feathering`: Enable anti-aliasing/feathering for smoother rendering
    - `--time-range <START> <END>`: Export specific time range instead of current view
    - `--signals <SIGNAL_LIST>`: Export only specified signals (comma-separated list)
  - `set_time_range <START_TIME> <END_TIME> [VIEWPORT_ID]`: Set the visible time range of the waveform display. `START_TIME` and `END_TIME` can be absolute times (e.g., `100ns`) or relative to the current view. `VIEWPORT_ID` (optional) specifies which viewport to apply the zoom to. If omitted, the zoom is applied to the currently active viewport.
- **New GUI Menu Item:** A new menu item (e.g., "File -> Export Plot as PNG...") will be added to the Surfer GUI. This will open a dialog allowing users to configure export options including file format, dimensions, DPI, UI element visibility, and theme before exporting the current view.

### 3.2. Export Configuration Options

Based on analysis of the existing snapshot testing infrastructure, the following configuration options will be available for PNG export:

#### 3.2.1. Image Format and Quality Options
- **Format Selection**: Support for PNG (default) and JPEG formats
- **Image Dimensions**: Configurable width and height (default: 1280x720, matching snapshot tests)
- **DPI/Resolution**: Multiplier for high-resolution exports (default: 1.0, supports 2.0 for retina displays)
- **Anti-aliasing**: Feathering option for smoother rendering (default: false, matching snapshot behavior)

#### 3.2.2. UI Element Visibility Controls
The export system will provide granular control over which UI elements are included in the exported image, based on the toggleable elements identified in snapshot tests:

- **Menu Bar**: `--show-menu` / `--hide-menu` (default: hidden for clean exports)
- **Toolbar**: `--show-toolbar` / `--hide-toolbar` (default: hidden for clean exports)
- **Side Panel**: `--show-side-panel` / `--hide-side-panel` (default: hidden for clean exports)
- **Status Bar**: `--show-statusbar` / `--hide-statusbar` (default: hidden for clean exports)
- **Overview Panel**: `--show-overview` / `--hide-overview` (default: hidden for clean exports)
- **Timeline/Axis**: `--show-timeline` / `--hide-timeline` (default: shown for waveform context)
- **Time Markers**: `--show-markers` / `--hide-markers` (default: shown if present)
- **Cursor Line**: `--show-cursor` / `--hide-cursor` (default: shown if present)
- **Signal Indices**: `--show-indices` / `--hide-indices` (default: shown)

#### 3.2.3. Content and View Options
- **Theme Selection**: Apply specific themes (dark+, light+, solarized, ibm, etc.) for export
- **Time Range**: Export specific time ranges instead of current view
- **Signal Filtering**: Export only specified signals or signal groups
- **Viewport Selection**: Export specific viewports or all viewports
- **Zoom Level**: Control zoom/scale of the exported waveform

#### 3.2.4. Rendering Options
- **Background Color**: Custom background color (default: theme-based)
- **Signal Colors**: Preserve current signal colors or apply theme defaults
- **Font Rendering**: Use custom fonts (matching snapshot test behavior)
- **Frame Rendering**: Number of frames to render before capture (default: 5, matching snapshot tests)

### 3.3. Data Models

- **ExportConfig**: A new configuration struct will be introduced to encapsulate all export options
- **ExportFormat**: Enum for supported image formats (PNG, JPEG)
- **UIElementVisibility**: Struct to control which UI elements are shown/hidden
- **ExportRegion**: Struct to define specific time ranges and signal selections for export
- No new core data models are required for the existing `SystemState` rendering. However, new transient data structures will be introduced within the command file parser and `libsurfer` export function to manage export configurations and multiple export regions.

### 3.4. Component Responsibilities

- **`surfer/src/main.rs`:**
  - Process command files, including parsing and executing the new `export_png` command with all configuration options and the `set_time_range` command.
  - Parse export configuration options from command-line arguments and command files, creating `ExportConfig` structs.
  - Initialize `SystemState` (potentially loading a waveform and other commands from the command file), apply export-specific UI element visibility settings, call the `libsurfer` export function with the configuration, and handle success/failure.
  - For GUI mode, handle the new menu item action, opening a configuration dialog, and triggering the `libsurfer` export function with user-selected options.
  - Ensure that the GUI is not launched when processing command files in a headless export mode.
- **`libsurfer/src/export.rs` (New File):**
  - A new module `export.rs` will be created to house the `export_png` function and related configuration structures.
  - The `export_png` function will accept `SystemState`, `ExportConfig`, and output file path, handling all configuration options including format, dimensions, DPI, UI element visibility, theme, and content filtering.
  - It will configure the `SystemState` based on the export configuration (e.g., hiding UI elements, applying themes, setting time ranges), use `egui_skia_renderer` to create an offscreen surface with the specified dimensions and DPI, draw the configured `SystemState` onto it, and then encode and save the surface content in the specified format.
  - Support for multiple export formats (PNG, JPEG) with appropriate encoding options.
  - Handle platform-specific file I/O differences (native vs WASM) for saving exported images.
- **`libsurfer/src/lib.rs`:**
  - Expose the `export_png` function, `ExportConfig` struct, and related configuration types from `export.rs`.

## 4. Alternatives Considered

- **Using a separate headless executable or a direct CLI flag:** Initially considered a separate executable or a direct CLI flag for export. However, the approach of integrating export functionality directly into the main `surfer` executable via command file commands was chosen for its simplicity, reusability of code, and alignment with existing command file processing logic. This allows for more complex and reproducible export scenarios.
- **Directly using `egui_skia_renderer` in `surfer/src/main.rs`:** While possible, encapsulating the export logic in `libsurfer/src/export.rs` promotes better modularity and reusability, aligning with the existing structure where core functionalities reside in `libsurfer`.

## 5. Out of Scope

- **SVG Export**: Vector-based SVG export is significantly more complex than raster image export, requiring conversion of the entire rendering pipeline from raster (Skia) to vector (SVG) operations. This would involve rewriting substantial portions of the rendering system and is not feasible for the initial implementation.
- **WaveDrom Integration**: [WaveDrom](https://wavedrom.com/tutorial.html) is an interesting JavaScript-based approach for creating digital timing diagrams with a JSON-based format. While WaveDrom could potentially be used as an alternative export format for waveform visualization, integrating it would require either:
  - Converting Surfer's internal waveform data to WaveDrom's WaveJSON format
  - Embedding WaveDrom as a web component within the application
  - Creating a separate WaveDrom export pipeline
  This represents a fundamentally different approach to waveform visualization and export that is beyond the scope of the current PNG/JPEG raster export feature.
- Advanced rendering options (e.g., custom background colors, specific signal visibility filters) beyond what is currently displayed in the `SystemState` (unless explicitly part of a region/view specification).
- While the initial implementation focuses on PNG and JPEG, the design should allow for future expansion to other raster image formats without requiring a complete re-architecture.

## 6. Testing Strategy

Testing for the PNG export feature will adhere to the existing project style for testing, leveraging the existing snapshot testing infrastructure. The export functionality will be tested using the same patterns established in `libsurfer/src/tests/snapshot.rs` and `libsurfer/src/tests/export.rs`.

### 6.1. Test Categories

- **Basic Export Functionality**: Tests that verify PNG export works with default settings (1280x720, PNG format, clean UI)
- **Configuration Option Tests**: Individual tests for each configuration option (format, dimensions, DPI, UI element visibility, themes)
- **UI Element Visibility Tests**: Tests for each toggleable UI element (menu, toolbar, side panel, status bar, overview, timeline, markers, cursor)
- **Format Support Tests**: Tests for PNG and JPEG export formats
- **Theme Application Tests**: Tests for applying different themes during export
- **High-Resolution Tests**: Tests for DPI scaling and high-resolution exports
- **Command File Integration Tests**: Tests for export commands within command files
- **GUI Integration Tests**: Tests for the GUI export dialog and menu integration

### 6.2. Test Data and Examples

Tests will use the existing example files (`examples/counter.vcd`, `examples/picorv32.vcd`) and follow the established patterns:
- Use the same Tokio runtime setup as snapshot tests
- Apply the same UI element toggling patterns (`Message::ToggleMenu`, `Message::ToggleToolbar`, etc.)
- Use the same rendering parameters (feathering, frame count, surface creation)
- Validate exported images for correct dimensions, format, and content

### 6.3. Test Structure

New tests will be added to `libsurfer/src/tests/export.rs` following the established pattern:
- Simple test functions (no macros for single tests)
- Proper cleanup of temporary files
- Validation of exported image properties
- Platform-specific testing considerations (native vs WASM)
