---
inclusion: manual
---

# Task: Headless Export Plot as PNG

## Clear Objective

Implement the functionality to export waveform plots as PNG images via command files, supporting multiple regions or views and viewport-specific zoom control, and enabling a `--headless` CLI argument for non-GUI execution. This feature focuses on automated, scriptable exports.

## Acceptance Criteria

- **Headless Mode:**
  - The `surfer` executable can be invoked with a `--headless` argument, preventing the GUI from launching.
- **Command File Export:**
  - The `surfer` executable can process command files containing `export_png` commands with parameters to specify regions or views, saving one or more PNG images of the waveform views at the specified path(s).
  - Each exported PNG accurately reflects the loaded waveform, signals, cursors, and markers within its specified region or view.
  - The `export_png` command supports exporting a single specified viewport, or by default, all viewports either as a single combined PNG or as separate PNGs.
  - The command file execution exits without errors on successful export and with an informative error message on failure (e.g., invalid path, rendering error, invalid region specification).
- **Viewport Zoom Control:**
  - The `surfer` executable can process command files containing `set_time_range <START_TIME> <END_TIME> [VIEWPORT_ID]` commands.
  - When `VIEWPORT_ID` is specified, the zoom is applied only to that viewport.
  - When `VIEWPORT_ID` is omitted, the zoom is applied to the currently active viewport.

## Requirements Traceability

- Traces to: Epic: Implement PNG Export Feature (`.work-items/png-export/epic.md`)
- Traces to: User Story: Export Waveform as PNG (`.work-items/png-export/user-story.md`)
- Traces to: Design Document: PNG Export Feature (`.work-items/png-export/design.md`)

## Test Strategy

- Create new integration tests that load a sample waveform, apply some commands (e.g., adding signals, cursors, `set_time_range` for specific viewports), and then execute a command file with `export_png` commands, including parameters for multiple regions or views and viewport selection.
- Verify that multiple PNG files are created at the specified paths, with appropriate naming conventions (e.g., suffixed), and that each accurately represents its specified area.
- (Future step) Compare the generated PNGs with reference images to ensure visual correctness for command file exports.

## Architectural Considerations (from `standards-architecture.md`)

### Conceptual Diagram: Headless Export Flow

```
+---------------------+     +---------------------+
| CLI (surfer --headless) |     | Command File Parser |
| (Invokes Surfer)    |---->| (Parses export_png, |
|                     |     |  set_time_range)    |
+----------|----------+     +----------|----------+
           |                           |
           | (Command Line Args)       | (Parsed Commands)
           V                           V
+---------------------+     +---------------------+
| libsurfer           |     | libsurfer::export   |
| (SystemState Mgmt)  |<----| (Rendering & PNG    |
|                     |     |  Encoding Logic)    |
+----------|----------+     +----------|----------+
           |                           |
           | (Updated SystemState)     | (PNG Data)
           V                           V
+---------------------+     +---------------------+
| File System         |     | Output PNG Files    |
| (Writes PNG Files)  |<----| (Multiple, if       |
+---------------------+     |  specified)         |
```

### Logical View

- The headless export feature will extend the `export` module (e.g., `libsurfer/src/export.rs`) to handle more complex rendering instructions, including specific time ranges, viewport IDs, and potentially multiple export regions.
- The `surfer/src/main.rs` (CLI) will be responsible for parsing command-line arguments (`--headless`) and command file instructions (`export_png`, `set_time_range`).
- The `SystemState` will be manipulated programmatically based on command file inputs before triggering the export.

### Process View

- User invokes `surfer --headless -c <command_file.sucl>`.
- `surfer/src/main.rs` parses `--headless` and the command file.
- For each `set_time_range` command, `libsurfer` updates the `SystemState` for the specified viewport.
- For each `export_png` command, `libsurfer`'s `export` module is called with the current `SystemState` and specific export parameters (time range, viewport, output path).
- The `export` module renders, encodes, and writes the PNG.
- The CLI exits after processing all commands.

### Data View

- Input data includes waveform files, command files (`.sucl`), and CLI arguments.
- The `SystemState` is dynamically modified based on command file instructions.
- Output data consists of one or more PNG files.

## Implementation Plan & Pseudocode

### CLI Argument Parsing Pseudocode (e.g., in `surfer/src/main.rs`)

```rust
// In surfer/src/main.rs
struct Args {
    #[arg(long)]
    headless: bool,
    #[arg(short, long)]
    command_file: Option<PathBuf>,
    // ... other args
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    if args.headless {
        // Initialize SystemState for headless operation
        let mut app_state = SystemState::new_headless();

        if let Some(cmd_file_path) = args.command_file {
            let commands = parse_command_file(&cmd_file_path)?;
            for command in commands {
                match command {
                    Command::SetTimeRange { start, end, viewport_id } => {
                        app_state.set_time_range(start, end, viewport_id)?;
                    },
                    Command::ExportPng { path_prefix, viewport_id, region_specs } => {
                        libsurfer::export::export_waveform_to_png(
                            &app_state,
                            &path_prefix,
                            viewport_id,
                            region_specs,
                            DEFAULT_HEADLESS_WIDTH,
                            DEFAULT_HEADLESS_HEIGHT,
                        )?;
                    },
                    // ... other commands
                }
            }
        }
        // Exit after processing commands
        Ok(())
    } else {
        // Launch GUI (existing logic)
        // ...
        Ok(())
    }
}

// Placeholder for command file parsing
fn parse_command_file(path: &Path) -> Result<Vec<Command>, Box<dyn Error>> {
    // ... read and parse .sucl file ...
    Ok(vec![])
}

enum Command {
    SetTimeRange { start: Time, end: Time, viewport_id: Option<ViewportId> },
    ExportPng { path_prefix: PathBuf, viewport_id: Option<ViewportId>, region_specs: Vec<RegionSpec> },
    // ...
}
```

### Core Export Logic Pseudocode (`libsurfer/src/export.rs` - extended)

```rust
// In libsurfer/src/export.rs

pub fn export_waveform_to_png(
    app_state: &SystemState,
    path_prefix: &Path,
    target_viewport_id: Option<ViewportId>,
    region_specs: Vec<RegionSpec>,
    width: u32,
    height: u32,
) -> Result<(), ExportError> {
    let viewports_to_export = if let Some(id) = target_viewport_id {
        vec![app_state.get_viewport(id)?]
    } else if region_specs.is_empty() {
        app_state.get_all_viewports()
    } else {
        // Logic to determine viewports based on region_specs if not explicitly given
        app_state.get_viewports_for_regions(&region_specs)
    };

    for (idx, viewport) in viewports_to_export.iter().enumerate() {
        let current_state_for_export = app_state.clone_for_viewport_and_regions(viewport, &region_specs)?;

        let mut surface = egui_skia_renderer::create_surface(width, height)?;
        egui_skia_renderer::draw_onto_surface(&mut surface, &current_state_for_export)?;

        let png_data = surface.image_snapshot().encode_png_data()?;

        let output_path = if viewports_to_export.len() > 1 || !region_specs.is_empty() {
            // Append suffix for multiple exports
            path_prefix.with_extension(format!("_{}.png", idx))
        } else {
            path_prefix.with_extension("png")
        };

        std::fs::write(&output_path, &png_data)?;
    }

    Ok(())
}

// Placeholder for RegionSpec and ViewportId
pub struct RegionSpec { /* ... */ }
pub struct ViewportId { /* ... */ }
```

## Task Breakdown Methodology: Sequential, ACID-Compliant Steps

This feature will be broken down into the following sequential, ACID-Compliant Steps:

- **01_add_headless_cli_argument**: Implement the `--headless` CLI argument in `surfer/src/main.rs` to prevent GUI launch.
  - *Acceptance Criteria*: `surfer` executable's `Args` includes `headless` option; GUI does not launch when `--headless` is used.
- **02_add_command_file_parsing**: Implement parsing for the new `export_png` and `set_time_range` commands and their associated arguments (including viewport ID and region/view specifications) within `surfer/src/main.rs`.
  - *Acceptance Criteria*: `surfer` can parse `export_png` and `set_time_range` commands with arguments; command file execution handles errors gracefully.
- **03_implement_set_zoom_logic**: Implement the logic for the `set_time_range` command within `libsurfer/src/export.rs` (or a related module) to adjust the zoom for a specified or active viewport.
  - *Acceptance Criteria*: `set_time_range` command successfully adjusts zoom for target viewport in `SystemState`.
- **04_implement_command_file_export_logic**: Implement the logic within `libsurfer/src/export.rs` to handle multiple region/view specifications and viewport selection from command files, configure the `SystemState` for each, and export individual PNGs.
  - *Acceptance Criteria*: `export_png` command successfully exports multiple PNGs based on region/view specifications and viewport selection; each PNG accurately reflects the specified area.
- **05_add_integration_tests**: Create comprehensive integration tests to verify the command file export functionality (including `set_time_range` and `export_png` with viewport options).
  - *Acceptance Criteria*: New integration tests exist; load sample waveform, apply commands, export to temporary PNGs; verify PNG existence and basic properties; tests pass successfully.
