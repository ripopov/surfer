# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Surfer is a waveform viewer for digital design verification and debugging, written in Rust. It provides a modern, extensible interface for viewing VCD, FST, GHW, and FTR waveform files. The application runs on desktop platforms (Linux, macOS, Windows) and in web browsers via WebAssembly.

## Key Development Commands

### Building

```bash
# Standard debug build
cargo build

# Release build
cargo build --release

# Build with Python translator support
cargo build --release --features python

# Build for WASM (requires trunk and wasm32-unknown-unknown target)
RUSTFLAGS="--cfg=web_sys_unstable_apis" trunk build surfer/index.html --release
```

### Testing

```bash
# Run all tests
cargo test

# Run tests with Python feature
cargo test --features python

# Run specific test
cargo test test_name

# Run snapshot tests and accept new baselines
cargo test snapshot
./accept_snapshots.bash
```

### Linting and Formatting

```bash
# Format code
cargo fmt

# Run clippy
cargo clippy --all-targets

# Run pre-commit checks (if installed)
pre-commit run --all-files
```

### Running the Application

```bash
# Run debug version
cargo run --bin surfer

# Run with a waveform file
cargo run --bin surfer -- examples/counter.vcd

# Run server component
cargo run --bin surver
```

## Architecture Overview

### Project Structure

The codebase is organized as a Cargo workspace:
- `surfer/` - Main application binary
- `libsurfer/` - Core library with all waveform viewing logic
- `surver/` - Standalone server for remote waveform serving
- `surfer-translation-types/` - Shared types for translator plugins
- `instruction-decoder/` - CPU instruction decoding for signal interpretation

### Core Architecture Patterns

1. **Message-Driven Architecture**: All state changes flow through a `Message` enum, processed by `SystemState::update()`. This enables undo/redo and consistent state management.

2. **Data Container System**: Waveforms are abstracted behind `DataContainer` enum, supporting both traditional waveforms (`WaveContainer`) and transaction-based data (`TransactionContainer`).

3. **Translator System**: Extensible signal value interpretation through translators:
   - Built-in: Binary, Hex, Decimal, Clock detection, Fixed-point, etc.
   - Custom: Python API and WASM plugin support

4. **Command System**: Fuzzy-matching command parser (`fzcmd`) for terminal-style interactions with autocomplete.

### Key Components

- **State Management**: `SystemState` holds all application state, implements egui's `App` trait
- **Drawing**: Canvas rendering with cached draw commands, invalidation-based updates
- **Data Loading**: Async loading via `wellen` library, lazy signal data fetching
- **UI Structure**: Left panel (hierarchy), center canvas (waveforms), right panel (variables)
- **Remote Support**: HTTP-based protocol for serving waveforms from remote servers

### Important Conventions

1. **Error Handling**: Use `Result` types, propagate errors to UI via messages
2. **Platform Differences**: Use conditional compilation for desktop vs WASM
3. **Performance**: Cache drawing commands, use viewport culling, load data on-demand
4. **Testing**: Visual regression tests use snapshots, comparison threshold for minor rendering differences
5. **Dependencies**: Always check if a library is already used before adding new ones

### Message Flow Example

```
User clicks "Add Variable" →
UI generates Message::AddVariables →
SystemState::update() processes message →
Updates internal state →
Invalidates draw commands →
Next frame redraws canvas
```

### Working with Translators

Translators convert raw signal values to human-readable formats. When adding new translators:
1. Implement in `libsurfer/src/translation/`
2. Add to `AnyTranslator` enum
3. Register in translator list initialization
4. Follow existing patterns for configuration and caching
