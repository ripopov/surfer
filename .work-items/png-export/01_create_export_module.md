# Task Step 01: Create Export Module

## Objective

Create a new module `libsurfer/src/export.rs` containing a function `export_png` that takes `SystemState`, a file path, and image size, and uses `egui_skia_renderer` and `image` crates to render the state and save it as a PNG.

## Acceptance Criteria

- A new file `libsurfer/src/export.rs` exists.
- The file contains a public function `export_png` with the specified signature.
- The `export_png` function successfully creates an offscreen surface, draws the `SystemState` onto it, and encodes it as a PNG.
- The function returns `Result<()>`.

## Requirements Traceability

- Traces to: Design Document: PNG Export Feature (Section 3.3 Component Responsibilities - `libsurfer/src/export.rs`)

## Test Strategy

- This step will be primarily verified by compilation. A dedicated unit test for `export_png` will be added in a later step, once `SystemState` can be easily mocked or initialized for testing purposes.

## Implementation Details

- Use `egui_skia_renderer::create_surface` to create the rendering surface.
- Use `egui_skia_renderer::draw_onto_surface` to render the `SystemState`.
- Use `surface.image_snapshot().encode(...)` to get the PNG data.
- Use `image::load_from_memory` and `save_with_format` to write the PNG file.
