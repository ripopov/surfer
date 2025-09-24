# Task Step 02: Integrate Export into libsurfer

## Objective

Make the `export_png` function from `libsurfer/src/export.rs` publicly accessible through the `libsurfer` crate.

## Acceptance Criteria

- The `export_png` function is re-exported in `libsurfer/src/lib.rs`.
- The `libsurfer` crate compiles successfully after this change.

## Requirements Traceability

- Traces to: Design Document: PNG Export Feature (Section 3.3 Component Responsibilities - `libsurfer/src/lib.rs`)

## Test Strategy

- This step will be verified by successful compilation of the `libsurfer` crate.

## Implementation Details

- Add `pub mod export;` and `pub use export::export_png;` to `libsurfer/src/lib.rs`.
