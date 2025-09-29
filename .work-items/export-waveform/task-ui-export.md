---
inclusion: manual
---

# Task: UI Export Plot as PNG

## Clear Objective

Implement the functionality to export the current waveform plot as a high-quality PNG image via a new menu item in the Surfer GUI. This feature focuses solely on the interactive user experience for exporting the currently displayed view.

## Acceptance Criteria

- **GUI Menu Item:**
  - [x] A new menu item (e.g., "File -> Export Plot...") is available in the Surfer GUI. (Default as PNG)
  - [x] Selecting this menu item opens a file dialog, allowing the user to choose the output path and filename.
  - [x] The current waveform plot is accurately exported as a PNG image to the specified location upon confirmation.
  - [ ] The menu item is disabled (grayed out) when no waveform data is loaded or no plot is currently displayed.
  - [ ] User short cuts
  - [ ] Success feedback is provided via status bar update (non-intrusive), and error feedback is provided via modal dialog (user attention required).
  - [ ] Settings under "Settings" menu Export Plot
  - [ ] WASM implementation

## Requirements Traceability

- Traces to: Epic: Implement PNG Export Feature (`.work-items/png-export/epic.md`)
- Traces to: User Story: Export Waveform as PNG (`.work-items/png-export/user-story.md`)
- Traces to: Design Document: PNG Export Feature (`.work-items/png-export/design.md`)

## Test Strategy

Following a Test-Driven Development (TDD) approach, tests will be written *before* the corresponding GUI and export logic. This ensures that the implementation is guided by verifiable behavior.

- **Unit Tests for `libsurfer::export` (Future Step):** While not the immediate focus of this UI task, robust unit tests will be developed for the `libsurfer::export` module. These tests will mock the `SystemState` and rendering context to verify that the core export logic correctly processes rendering instructions, encodes to PNG, and handles various scenarios (e.g., empty view, different dimensions, error conditions).
- **GUI Integration Tests:** New GUI-specific integration tests will be created to simulate user interaction with the "Export Plot as PNG..." menu item. These tests will:
  - **Verify Menu State:** Assert that the menu item is disabled when no waveform data is loaded and enabled when waveform data is present.
  - **Simulate Menu Click:** Programmatically trigger the menu item selection.
  - **Mock File Dialog:** Intercept the file dialog call and provide a predefined temporary output path.
  - **Verify Export Function Call:** Assert that the `libsurfer::export::export_current_view_to_png` function is invoked with the correct `SystemState`, output path, and dimensions.
  - **Verify File Creation:** Check for the existence of the generated PNG file at the temporary path.
  - **Verify UI Feedback:** Assert that appropriate success or error messages are displayed to the user within the GUI.
  - **(Future Step - Visual Correctness):** Compare the generated PNGs with reference images using a visual regression testing framework to ensure pixel-perfect accuracy for GUI exports.

### GUI Integration Test

See `libsurfer/src/tests/export.rs`.

## Architectural Considerations (from `standards-architecture.md`)

### Conceptual Diagram: UI Export Flow

```txt
+---------------------+     +---------------------+
| Surfer GUI          |     | libsurfer           |
| (Menu Item Click)   |---->| (Public API:        |
|                     |     |  export_png_current_view)|
+----------|----------+     +----------|----------+
           |                           |
           | (File Dialog)             | (SystemState, Path)
           V                           V
+---------------------+     +---------------------+
| File System         |     | libsurfer::export   |
| (User selects path) |<----| (Rendering & PNG    |
|                     |     |  Encoding Logic)    |
+---------------------+     +----------|----------+
                                         |
                                         | (PNG Data)
                                         V
                               +---------------------+
                               | File System         |
                               | (Writes PNG File)   |
                               +---------------------+
```

### Logical View

- The UI export feature will primarily interact with the existing `SystemState` to retrieve the current waveform data, visible time range, active signals, cursors, markers, and theme settings.
- A new `export` module (e.g., `libsurfer/src/export.rs`) will encapsulate the rendering and PNG encoding logic, abstracting it from the GUI.
- The GUI component will act as a client to this `export` module, providing the necessary `SystemState` and output path.

### Process View

- User clicks "Export Plot as PNG...".
- GUI triggers a file dialog to get the output path.
- GUI calls a function in `libsurfer` (e.g., `libsurfer::export::export_current_view_to_png`) passing the current `SystemState` and the chosen path.
- The `export` module renders the `SystemState` to an in-memory buffer, encodes it to PNG, and writes to the file.
- Control returns to the GUI, which displays a success or error message.

### Data View

- The `SystemState` will be the primary data source for rendering, containing all visual elements.
- The output will be a standard PNG file, containing rasterized image data.

## Implementation Plan & Pseudocode

### Keyboard Shortcut Considerations

While the primary interaction for this feature is through the GUI menu item, if a keyboard shortcut is to be implemented in the future, the following platform-specific considerations must be taken into account:

- **WASM (WebAssembly):** Keyboard event handling in web environments can differ. Ensure that any chosen shortcut does not conflict with browser-native shortcuts or accessibility features. Consider using `eframe`'s or `egui`'s platform-agnostic input handling if available, or implement specific web-focused event listeners.
- **Native (macOS/Linux/Windows):** Native desktop applications typically have more control over keyboard events. Shortcuts should adhere to platform conventions (e.g., `Cmd+E` on macOS, `Ctrl+E` on Windows/Linux for save operations). The implementation should use `eframe`'s or `egui`'s native event handling, which generally abstracts away OS-specific differences, but careful testing on each platform is crucial.

### GUI Interaction Pseudocode (e.g., in `surfer/src/gui/app.rs`)

```rust
// In Surfer GUI application logic
fn handle_menu_action(action: MenuAction, app_state: &mut SystemState) {
    match action {
        MenuAction::ExportPng => {
            // 1. Open file dialog
            if let Some(output_path) = open_save_file_dialog("Export Plot as PNG", "plot.png") {
                // 2. Call libsurfer export function
                let result = libsurfer::export::export_current_view_to_png(
                    app_state,
                    &output_path,
                    app_state.current_viewport_width(), // Get current render size
                    app_state.current_viewport_height(),
                );

                // 3. Display feedback to user
                match result {
                    Ok(_) => show_toast_notification("Plot exported successfully!"),
                    Err(e) => show_error_message(&format!("Failed to export plot: {}", e)),
                }
            }
        }
        // ... other menu actions
    }
}

// Function to determine if export menu should be enabled
fn is_export_menu_enabled(app_state: &SystemState) -> bool {
    // Check if waveform data is loaded and plot is available
    app_state.has_waveform_data() && app_state.has_visible_plot()
}

// Placeholder for GUI file dialog (e.g., using `rfd` or `native-dialog` crate)
fn open_save_file_dialog(title: &str, default_filename: &str) -> Option<PathBuf> {
    // ... implementation to open a native save file dialog ...
    Some(PathBuf::from("/path/to/selected/file.png")) // Example
}

// Placeholder for GUI notifications
fn show_toast_notification(message: &str) { /* ... */ }
fn show_error_message(message: &str) { /* ... */ }
```

### Core Export Logic

See `libsurfer/src/export.rs`.

## Task Breakdown Methodology: Sequential, ACID-Compliant Steps

This feature will be broken down into the following sequential, ACID-Compliant Steps:

- **01_create_export_module**: Create the `libsurfer/src/export.rs` module with a basic `export_png` function that takes `SystemState`, a file path, and image size, and uses `egui_skia_renderer` and `image` crates to render the state and save it as a PNG.
  - *Acceptance Criteria*: New file `libsurfer/src/export.rs` exists; contains `export_png` with specified signature; function successfully renders `SystemState` and encodes as PNG; returns `Result<()>`. (Traces to: Design Document: PNG Export Feature Section 3.3)
- **02_integrate_export_into_libsurfer**: Make the `export_png` function from `libsurfer/src/export.rs` publicly accessible through the `libsurfer` crate.
  - *Acceptance Criteria*: `export_png` re-exported in `libsurfer/src/lib.rs`; `libsurfer` compiles successfully. (Traces to: Design Document: PNG Export Feature Section 3.3)
- **03_add_gui_menu_item**: Implement the "File -> Export Plot as PNG..." menu item in the Surfer GUI, including file dialog interaction and proper enable/disable state management.
  - *Acceptance Criteria*: New menu item exists; menu item is disabled when no waveform data is loaded; menu item is enabled when waveform data is present; selecting it opens a file dialog; user can choose output path/filename.
- **04_integrate_gui_export_logic**: Integrate the `libsurfer` export function with the GUI menu item to export the current plot.
  - *Acceptance Criteria*: GUI menu item successfully calls `libsurfer::export_png` with current `SystemState` and chosen path; confirmation/error messages displayed.
- **05_add_gui_tests**: Create comprehensive GUI integration tests for the menu item export functionality.
  - *Acceptance Criteria*: New GUI tests exist; tests verify menu item enable/disable state based on waveform data presence; tests simulate menu item selection and file dialog interaction; verify that `libsurfer::export::export_current_view_to_png` is called with correct arguments; assert existence of generated PNG file; verify correct UI feedback (success/error messages); tests pass successfully.
