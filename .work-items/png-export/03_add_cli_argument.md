# Task Step 03: Add CLI Argument and Integration

## Objective

Modify `surfer/src/main.rs` to add an `--export` command-line argument and integrate the call to `libsurfer::export_png` when this argument is present.

## Acceptance Criteria

- The `surfer` executable's `Args` structure includes an `export_path` option.
- When `export_path` is provided, the application initializes `SystemState`, calls `libsurfer::export_png`, and exits.
- The GUI does not launch when `--export` is used.
- The command-line tool handles errors during export gracefully, returning a non-zero exit code.

## Requirements Traceability

- Traces to: Design Document: PNG Export Feature (Section 3.1 API Contracts, Section 3.3 Component Responsibilities - `surfer/src/main.rs`)

## Test Strategy

- Manually run `cargo run -- --export output.png` with a sample waveform to confirm a PNG is generated and the application exits without launching the GUI.
- Manually test with an invalid path to ensure error handling.

## Implementation Details

- Add `export_path: Option<Utf8PathBuf>` to the `Args` struct.
- In `main_impl::main`, check for `args.export_path`.
- If present, create a `SystemState`, call `libsurfer::export_png`, and return `Ok(())` or an error.
- Ensure `eframe::run_native` is skipped if `export_path` is present.
- Determine a reasonable default size for the exported image (e.g., 1280x720 or from config).
