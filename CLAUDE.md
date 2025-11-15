# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Surfer is a waveform viewer for VCD, FST, and GHW files with a focus on a snappy usable interface and extensibility. It's built using Rust and the egui/eframe GUI framework, and supports both native and WASM targets.

## Common Commands

### Building
```bash
# Build debug version
cargo build

# Build release version (much faster, use for performance testing)
cargo build --release

# Build and run debug executable
cargo run --bin surfer

# Build with optional features
cargo build --features accesskit
cargo build --features f128
cargo build --features python
cargo build --features wasm_plugins
```

### Testing
```bash
# Run all tests
ulimit -n 10240 && cargo test

# Run tests with Python features (includes snapshot tests)
cargo test --features python

# Run tests including ignored tests (as CI does)
cargo test --features python -- --include-ignored

# Run specific test
cargo test <test_name>

# Update snapshot test images after changes
cargo test
./accept_snapshots.bash
```

### Linting
```bash
# Run clippy
cargo clippy

# Run clippy and apply fixes automatically
cargo clippy --fix
```

### Formatting
```bash
# Format code
cargo fmt
```

### Server Mode
```bash
# Start surfer in server mode
cargo run --bin surfer -- server --file <waveform.vcd>

# Or use standalone server (surver)
cargo run --bin surver -- <waveform.vcd>
```

## Workspace Structure

This is a Cargo workspace with multiple crates:

- **surfer** - Main binary crate, thin wrapper around libsurfer
- **libsurfer** - Core library containing all application logic, can be compiled to WASM
- **surver** - Standalone server for remote waveform viewing
- **surfer-translation-types** - Types for the translator plugin system
- **wasm_example_translator** - Example WASM translator plugin
- **translator-docs** - Documentation generator for translators
- **f128** - Optional 128-bit floating point support (git submodule)
- **instruction-decoder** - Instruction decoding support (git submodule)

## Architecture

### Core Data Model

The application follows a message-passing architecture using egui's immediate-mode GUI:

1. **WaveData** (`libsurfer/src/wave_data.rs`) - Owns all wave-related state including:
   - `DataContainer` - Wraps either `WaveContainer` (VCD/FST/GHW) or `TransactionContainer` (FTR)
   - `DisplayedItemTree` - Tree structure of items to display in the waveform view
   - `Viewport` - Controls visible time range
   - `cursor` and `markers` - Time navigation

2. **SystemState** (`libsurfer/src/system_state.rs`) - Root application state containing:
   - `UserState` - Serializable state (config overrides, waves, UI state)
   - Message queue for async operations
   - Canvas state for drawing

3. **WaveContainer** (`libsurfer/src/wave_container.rs`) - Wraps the underlying `wellen` library which parses waveform files

4. **TransactionContainer** (`libsurfer/src/transaction_container.rs`) - Handles FTR (transaction) files

### Translation System

The translator system converts raw bit vectors into human-readable values:

- Base trait: `Translator<VarId, ScopeId, Message>` from `surfer-translation-types`
- Built-in translators in `libsurfer/src/translation/`:
  - `basic_translators.rs` - Hex, binary, unsigned, signed, ASCII, etc.
  - `numeric_translators.rs` - Floating point formats (IEEE 754, bfloat16, posit, etc.)
  - `instruction_translators.rs` - Instruction decoding for RISC-V, MIPS, LoongArch
  - `enum_translator.rs` - Spade enum support
  - `clock.rs` - Clock signal translation
  - `python_translators.rs` - Python plugin support
  - `wasm_translator.rs` - WASM plugin support

Translators return a `TranslationPreference` (Prefer/Yes/No) to indicate if they can translate a variable.

### Message System

The application uses a message queue (`Message` enum in `libsurfer/src/message.rs`) to handle:
- Async file loading
- User commands from the command prompt
- UI interactions
- WCP (Waveform Control Protocol) events

Messages are processed in `SystemState::process_messages()`.

### Command System

Surfer has a fuzzy-completion command interface:

- Command parser: `libsurfer/src/command_parser.rs`
- Command prompt UI: `libsurfer/src/command_prompt.rs`
- Fuzzy matching: `libsurfer/src/fzcmd.rs`
- Batch commands: `libsurfer/src/batch_commands.rs` (for `.sucl` script files)

### Drawing Pipeline

1. **View** (`libsurfer/src/view.rs`) - Main rendering logic, orchestrates drawing
2. **DrawingCanvas** (`libsurfer/src/drawing_canvas.rs`) - Caches draw commands for waveforms
3. **AnalogRenderer** (`libsurfer/src/analog_renderer.rs`) - Renders analog waveforms with interpolation

Drawing is cached based on viewport to avoid regenerating on every frame.

### Remote Support

- **WCP (Waveform Control Protocol)** - Allows remote control and serving waveforms
- Server implementation in `surver`
- Client code in `libsurfer/src/remote/`
- Test coverage in `libsurfer/src/tests/wcp.rs` and `libsurfer/src/tests/wcp_tcp.rs`

## Testing

### Snapshot Tests

The primary testing mechanism is snapshot (image) tests in `libsurfer/src/tests/snapshot.rs`:

- Tests render the UI using `egui_skia_renderer` and compare against reference images
- Reference images are in `snapshots/` directory
- When making rendering changes:
  1. Run `cargo test` (tests will fail)
  2. Review `.new.png` files in `snapshots/`
  3. Run `./accept_snapshots.bash` to accept changes
  4. Commit updated `.png` files

The snapshot test system:
- Spawns a tokio runtime for async operations
- Uses message queue to set up test scenarios
- Renders to a Skia surface
- Compares images using `image-compare` crate

### Unit Tests

Unit tests are scattered throughout modules. Remote/WCP functionality has dedicated test modules.

## Configuration Patterns

When adding a new config option:

1. Add an `Option<T>` field to `UserState` in `libsurfer/src/state.rs`
2. Add the default value to `SurferConfig` in `libsurfer/src/config.rs`
3. Add a getter method in `libsurfer/src/state_util.rs`:
   ```rust
   pub fn my_config_value(&self) -> T {
       self.user.my_config_value
           .unwrap_or_else(|| self.user.config.my_config_value)
   }
   ```

This pattern allows:
- User overrides are saved in state files
- Default comes from config file when not overridden
- Config changes apply to existing states that haven't overridden the value

## Performance

- Use `show_performance` command in the UI to see frame timing breakdown
- Use `show_performance redraw` to disable draw cache for performance testing
- Always test performance in `--release` mode
- Draw command caching in `DrawingCanvas` is critical for performance
- Flamegraphs: `CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph -- examples/picorv32.vcd -c performance.sucl`

## Platform-Specific Notes

### Target Architecture
- Most code supports both native and `target_arch = "wasm32"`
- Use conditional compilation `#[cfg(target_arch = "wasm32")]` or `#[cfg(not(target_arch = "wasm32"))]`
- WASM uses `web-time` crate instead of `std::time`
- File I/O operations are native-only

### Dependencies
- OpenSSL required for native builds
- Git submodules must be initialized: `git submodule update --init --recursive`

## Pre-commit Hooks

The project uses pre-commit hooks that run:
- `cargo fmt` - Auto-formats code
- Spelling checks (must fix manually)
- Image compression with oxipng

If pre-commit reformats code, you must commit again.

## CI/CD

GitLab CI is configured in `.gitlab-ci.yml`:
- `clippy` - Linting
- `test` - Runs `cargo test --features python -- --include-ignored`
- `cargo_about` - License compliance
- `build_book` - Documentation
- Platform-specific builds for Linux, Windows, macOS

Coverage reports show which lines were executed (red/green in MR view).

## Key Rust Patterns

- Heavy use of `eyre::Result` for error handling
- `derive_more` for boilerplate reduction
- `lazy_static!` for global state (e.g., `EGUI_CONTEXT`, `OUTSTANDING_TRANSACTIONS`)
- Trait objects with `dyn Translator` for plugin system
- Message passing instead of direct state mutation
- Immediate-mode GUI with egui (no retained widget tree)

## Integration Points

- **VS Code Extension** - `surfer-vscode/` directory
- **WASM Build** - Can be embedded in web apps via iframe, controlled with `postMessage`
- **Integration API** - See `surfer/assets/integration.js`
- **Python Plugins** - Via `pyo3` when `python` feature enabled
- **WASM Plugins** - Via `extism` when `wasm_plugins` feature enabled
