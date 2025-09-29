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
  - [x] The menu item is disabled (grayed out) when no waveform data is loaded or no plot is currently displayed.
  - [x] User short cuts
  - [x] Success feedback is provided via status bar update (non-intrusive)
  - [~] error feedback is provided via modal dialog (user attention required) -- already handled with the Logs dialog that pops up
  - [ ] Settings under "Settings" menu Export Plot
    - [ ] Format dropdown (PNG/JPEG) - Simple enum dropdown
    - [ ] Include UI checkbox - Single boolean setting
    - [ ] Theme dropdown (Use Current Theme, Dark+, Light+, Solarized) - Simple enum dropdown
    - [ ] Include Waveform Elements checkboxes (Timeline/Axis, Time Markers, Cursor Line) - Multiple boolean settings
    - [ ] Size dropdown (Current Window, HD, Full HD, 2K, Custom) - Enum with custom option
    - [ ] Resolution dropdown (Auto, 1x, 2x, 3x) - Enum with auto detection
    - [ ] Custom size input fields (Width/Height) - Text input with validation
    - [ ] Settings persistence (save/load from config) - File I/O and serialization
    - [ ] Settings validation and error handling - Input validation logic
    - [ ] Custom size validation - Complex validation with bounds checking
    - [ ] DPI detection implementation - Platform-specific system calls
  - [ ] WASM implementation

## Export Settings Configuration

Based on the design document analysis, the following export settings will be available in the UI. These represent the most essential options that users need for waveform export, following the 80/20 rule:

### 3.1. Essential Export Settings (80% of use cases)

#### Image Format

- **Setting Name**: "Default Format"
- **UI Element**: Dropdown menu
- **Values**:
  - "PNG" (default)
  - "JPEG"
- **Description**: Default format when no extension is specified
- **Behavior**:
  - File dialog shows both PNG and JPEG filters
  - Format is determined by file extension (.png → PNG, .jpg/.jpeg → JPEG)
  - Default format used when no extension provided
  - Settings menu controls the default format preference

#### Image Size

- **Setting Name**: "Size"
- **UI Element**: Dropdown menu with custom input option
- **Values**:
  - "Current Window Size" (default)
  - "1280x720 (HD)"
  - "1920x1080 (Full HD)"
  - "2560x1440 (2K)"
  - "Custom..." (opens width/height input fields)
- **Description**: Set the output image dimensions
- **Custom Size Validation**:
  - **Width**: Integer between 100 and 8000 pixels
  - **Height**: Integer between 100 and 8000 pixels
  - **Aspect Ratio**: Warn if aspect ratio exceeds 10:1 or 1:10
  - **Sanitization**: Strip whitespace, convert to integers, validate bounds

#### Resolution/DPI

- **Setting Name**: "Resolution"
- **UI Element**: Dropdown menu with intelligent defaults
- **Values**:
  - "Auto (Match Display)" (default) - Automatically detects system DPI
  - "1x (Standard)"
  - "2x (High DPI)"
  - "3x (Retina)"
- **Description**: Resolution multiplier for export quality
- **Platform Detection**:
  - **macOS**: Detect Retina displays (2x, 3x) automatically
  - **Windows**: Detect high DPI displays (1.25x, 1.5x, 2x) automatically  
  - **Linux**: Detect fractional scaling (1.25x, 1.5x, 2x) automatically
  - **WASM**: Detect device pixel ratio from browser

#### Include UI Elements

- **Setting Name**: "Include UI"
- **UI Element**: Checkbox
- **Default**: Unchecked (false)
- **Description**: Include menu, toolbar, side panel, status bar in export

#### Include Waveform Elements

- **Setting Name**: "Include Waveform Elements"
- **UI Element**: Checkbox group
- **Options**:
  - "Timeline/Axis" (default: checked)
  - "Time Markers" (default: checked)
  - "Cursor Line" (default: checked)
- **Description**: Control which waveform elements are included

#### Export Theme

- **Setting Name**: "Theme"
- **UI Element**: Dropdown menu
- **Values**:
  - "Use Current Theme" (default)
  - "Dark+"
  - "Light+"
  - "Solarized"
- **Description**: Apply a specific theme for the export

### 3.2. Settings Menu Integration (Simplified)

The export settings will be integrated into the existing "Settings" menu structure as follows:

```
Settings
├── Export Plot
│   ├── Default Format: PNG ▼
│   ├── Size: Current Window Size ▼
│   │   └── Custom: [Width: 1920] [Height: 1080] (when Custom selected)
│   ├── Resolution: Auto (Match Display) ▼
│   ├── Include UI: ☐
│   ├── Include Waveform Elements
│   │   ├── Timeline/Axis: ☑
│   │   ├── Time Markers: ☑
│   │   └── Cursor Line: ☑
│   └── Theme: Use Current Theme ▼
```

### 3.3. Settings Persistence

- **Settings Storage**: Export settings will be persisted in the user's configuration file
- **Default Values**: All settings will have sensible defaults that match the current behavior
- **Reset Option**: "Reset to Defaults" option will be available in the settings menu
- **Per-Session Memory**: Settings will be remembered across application sessions

### 3.4. Essential Validation (Simplified)

#### Image Format Validation

- **Allowed Values**: PNG, JPEG
- **Sanitization**: Convert to lowercase, strip whitespace

#### Image Size Validation

- **Predefined Sizes**: Must match exact predefined values
- **Sanitization**: No custom input needed - dropdown only

#### UI Element Validation

- **Boolean Values**: Must be true/false
- **Sanitization**: Convert to boolean, default to false

#### Theme Validation

- **Allowed Themes**: Must match existing theme names
- **Sanitization**: Convert to lowercase, validate against available themes

#### File Path Validation

- **Path Format**: Must be valid file system path
- **Extension**: Must match selected image format
- **Sanitization**: Remove invalid filename characters, ensure proper extension

### 3.5. User Experience Principles

- **Progressive Disclosure**: Keep advanced options in command files
- **Sensible Defaults**: Most users will use defaults
- **Clear Labels**: Use simple, descriptive names
- **Immediate Feedback**: Show validation errors inline
- **One-Click Export**: Default settings should work for most users

### 3.6. DPI Detection Implementation

#### Platform-Specific DPI Detection

```rust
// DPI detection for different platforms
pub fn detect_system_dpi() -> f32 {
    #[cfg(target_os = "macos")]
    {
        // macOS: Use Core Graphics to detect Retina displays
        use core_graphics::display::CGDisplay;
        let display = CGDisplay::main();
        display.scale_factor() as f32
    }
    
    #[cfg(target_os = "windows")]
    {
        // Windows: Use WinAPI to detect DPI scaling
        use winapi::um::winuser::GetDpiForWindow;
        use winapi::um::winuser::GetDesktopWindow;
        let dpi = unsafe { GetDpiForWindow(GetDesktopWindow()) };
        dpi as f32 / 96.0 // Convert to scaling factor
    }
    
    #[cfg(target_os = "linux")]
    {
        // Linux: Use X11 or Wayland to detect scaling
        // This is a simplified example - real implementation would be more complex
        std::env::var("GDK_SCALE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1.0)
    }
    
    #[cfg(target_arch = "wasm32")]
    {
        // WASM: Use browser's device pixel ratio
        web_sys::window()
            .and_then(|w| w.device_pixel_ratio())
            .unwrap_or(1.0) as f32
    }
    
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux", target_arch = "wasm32")))]
    {
        1.0 // Default fallback
    }
}

// Convert detected DPI to user-friendly resolution setting
pub fn dpi_to_resolution_setting(dpi: f32) -> &'static str {
    match dpi {
        d if d >= 2.5 => "3x (Retina)",
        d if d >= 1.5 => "2x (High DPI)", 
        d if d >= 1.25 => "1.5x (High DPI)",
        _ => "1x (Standard)",
    }
}
```

#### User Experience Benefits

- **Automatic Quality**: Exports automatically match display quality
- **No Manual Configuration**: Users don't need to know their DPI
- **Platform Optimized**: Each platform uses native DPI detection
- **Fallback Support**: Graceful degradation for unsupported platforms

#### Settings Behavior

- **Default**: "Auto (Match Display)" - automatically detects and applies system DPI
- **Manual Override**: Users can still choose specific resolution if needed
- **Status Feedback**: Show detected DPI in settings (e.g., "Auto (2x detected)")
- **Persistence**: Remember user's choice (auto vs manual)

#### Custom Size Implementation

```rust
// Custom size validation and sanitization
pub fn validate_custom_size(width: &str, height: &str) -> Result<(u32, u32), ValidationError> {
    let width = width.trim().parse::<u32>()
        .map_err(|_| ValidationError::InvalidNumber)?;
    let height = height.trim().parse::<u32>()
        .map_err(|_| ValidationError::InvalidNumber)?;
    
    if width < 100 || width > 8000 || height < 100 || height > 8000 {
        return Err(ValidationError::OutOfBounds);
    }
    
    // Warn about extreme aspect ratios
    let aspect_ratio = width as f32 / height as f32;
    if aspect_ratio > 10.0 || aspect_ratio < 0.1 {
        // Show warning but allow the export
        log::warn!("Extreme aspect ratio detected: {:.2}:1", aspect_ratio);
    }
    
    Ok((width, height))
}

// UI behavior for custom size
pub enum SizeSetting {
    Predefined(String), // "Current Window Size", "1280x720 (HD)", etc.
    Custom { width: u32, height: u32 },
}
```

#### User Experience for Custom Size

- **Progressive Disclosure**: Custom fields only appear when "Custom..." is selected
- **Input Validation**: Real-time validation with inline error messages
- **Sensible Defaults**: Pre-fill with current window size when switching to custom
- **Aspect Ratio Warnings**: Alert users about extreme ratios but allow them
- **Persistence**: Remember custom dimensions across sessions
- **Common Presets**: Suggest common dimensions (square, portrait, etc.)

### 3.8. File Extension-Based Format Selection Implementation

#### Format Detection Logic

```rust
// Determine export format from file path
pub fn detect_format_from_path(path: &Path, default_format: ExportFormat) -> ExportFormat {
    if let Some(extension) = path.extension() {
        match extension.to_str().unwrap_or("").to_lowercase().as_str() {
            "png" => ExportFormat::Png,
            "jpg" | "jpeg" => ExportFormat::Jpeg,
            _ => default_format, // Use default if extension not recognized
        }
    } else {
        default_format // No extension, use default
    }
}

// File dialog configuration
pub fn create_export_file_dialog(default_format: ExportFormat) -> AsyncFileDialog {
    let mut dialog = AsyncFileDialog::new()
        .add_filter("PNG Images", &["png"])
        .add_filter("JPEG Images", &["jpg", "jpeg"])
        .add_filter("All Images", &["png", "jpg", "jpeg"]);
    
    // Set default filename with appropriate extension
    let default_filename = match default_format {
        ExportFormat::Png => "waveform.png",
        ExportFormat::Jpeg => "waveform.jpg",
    };
    
    dialog.set_file_name(default_filename)
}
```

#### User Experience Benefits

- **Intuitive**: Users naturally understand file extensions determine format
- **Flexible**: Can change format by changing extension in filename
- **Consistent**: Matches behavior of all other applications
- **Efficient**: One less UI element to manage
- **Fallback**: Default format used when no extension provided

#### Settings Behavior

- **Default Format Setting**: Controls what format to use when no extension is provided
- **File Dialog**: Shows both PNG and JPEG filters
- **Smart Defaults**: Pre-fills filename with appropriate extension based on default format
- **Format Detection**: Automatically detects format from chosen filename

### 3.9. Advanced Features (Command File Only)

The following advanced features will be available only through command files:

- Custom image dimensions
- Anti-aliasing control
- Custom time ranges
- Signal filtering
- Background color overrides
- Multiple viewport exports
- Batch export operations

This approach keeps the UI simple and focused on the most common use cases while providing power users with full control through command files.

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
