# Task Step 04: Add Integration Test

## Objective

Create an integration test to verify the command-line PNG export functionality.

## Acceptance Criteria

- A new integration test exists that:
    - Loads a sample waveform.
    - Exports it to a temporary PNG file using the CLI argument.
    - Verifies the existence of the PNG file.
    - (Optional, if feasible) Performs a basic check on the image dimensions or content.
- The integration test passes successfully.

## Requirements Traceability

- Traces to: Task: Implement PNG Export Feature (Test Strategy)

## Test Strategy

- Use `std::process::Command` to invoke the `surfer` executable with the `--export` argument.
- Create a temporary directory for the output PNG file.
- Assert that the command exits successfully (exit code 0).
- Assert that the output PNG file exists.
- Clean up the temporary file.

## Implementation Details

- Create a new test file, e.g., `surfer/tests/cli_export.rs`.
- Use `cargo_bin("surfer")` to get the path to the executable.
- Use `tempfile::tempdir()` for temporary file management.
- Consider using `image` crate to open and inspect the generated PNG for basic validation (e.g., dimensions).
