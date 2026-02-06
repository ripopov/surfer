# CLAUDE.md (AGENTS.md)

This file provides guidance to Claude Code (claude.ai/code) , Codex and other AI agents when working with code in this repository.

## Project Overview

Surfer is a tile-based waveform and transaction viewer written in Rust with egui, focused on snappy UI and extensibility. It supports VCD, FST, and GHW waveform formats, FTR memory transaction traces, remote loading via Surver, and CXXRTL data sources.

- **Repository:** https://gitlab.com/surfer-project/surfer/
- **License:** EUPL-1.2
- **Rust Edition:** 2024, MSRV 1.90

## Build Commands

Always use dev/test builds, not release builds.

```bash
cargo build                              # Dev build
cargo build --features python,f128       # With optional features

cargo test                               # Run all tests
cargo test --features python             # With Python feature
cargo test --include-ignored             # Run snapshot tests

cargo fmt                                # Format code
cargo clippy --no-deps                   # Lint

./accept_snapshots.bash                  # Accept snapshot test changes
```

## Workspace Structure

- **libsurfer/** - Core library: app state machine, rendering, translators, protocol handlers (compiles to `cdylib+rlib`)
- **surfer/** - Main GUI binary (desktop + wasm entry point)
- **surver/** - Standalone HTTP server for remote waveform viewing
- **surfer-wcp/** - Waveform Control Protocol message schema used by client/server integration
- **surfer-translation-types/** - Shared translator traits/types (`Translator`, `BasicTranslator`) and plugin-facing data model
- **translator-docs/** - Translator API docs/examples crate
- **wasm_example_translator/** - Example WASM translator plugin
- **surfer-vscode/** - VS Code integration package
- **f128/** - Optional 128-bit float support (excluded, requires gcc)
- **instruction-decoder/** - CPU instruction decoding (excluded, git submodule)

## Key Architecture

### State and Message Loop

- **SystemState** (`libsurfer/src/system_state.rs`) - Runtime state: channels, caches, async progress, undo/redo, protocol runtime
- **UserState** (`libsurfer/src/state.rs`) - Serializable user/session state (`.surf.ron`)
- **Message** (`libsurfer/src/message.rs`) - Central action/event enum for UI, async work, protocol commands, and table updates
- Async tasks push `Message` values through channels; `OUTSTANDING_TRANSACTIONS` tracks in-flight async work that should trigger redraws

### Data Model and Loading

- **WaveSource** (`libsurfer/src/wave_source.rs`) supports `File`, `Url`, `Data`, `DragAndDrop`, and `Cxxrtl`
- **DataContainer** (`libsurfer/src/data_container.rs`) unifies `Waves(WaveContainer)` and `Transactions(TransactionContainer)`
- **WaveContainer** (`libsurfer/src/wave_container.rs`) abstracts backend storage (`Wellen`, `Cxxrtl`, `Empty`)
- **WaveData** (`libsurfer/src/wave_data.rs`) owns displayed items, viewports, cursors/markers, graphics, and analog cache generation

### Rendering and Layout

- **Tile layout** (`libsurfer/src/tiles.rs`) manages the central panel via `SurferTileTree` with waveform and table panes
- **Wave rendering** (`libsurfer/src/drawing_canvas.rs`, `libsurfer/src/waveform_tile.rs`, `libsurfer/src/analog_renderer.rs`)
- **Hierarchy/UI model** (`libsurfer/src/displayed_item_tree.rs`, `libsurfer/src/displayed_item.rs`)
- **Table subsystem** (`libsurfer/src/table/`) provides model specs, caching/filtering/sorting, and egui table rendering for signal/transaction views

### Translation System

Translators live in `libsurfer/src/translation/`:

- **basic_translators.rs** - Raw bitvector transforms (hex/signed/unsigned/octal/ASCII/etc.)
- **numeric_translators.rs** - Floating-point, fixed-point, Posit, bfloat16, and related numeric decoders
- **instruction_translators.rs** - RV32/64, MIPS, and LA64 decoding
- **enum_translator.rs / color_translators.rs / event_translator.rs / clock.rs** - Domain-specific display translators
- **wasm_translator.rs** - Runtime WASM translator plugins
- **python_translators.rs** - Python-based custom translators (feature gated)

Translators implement `Translator` or `BasicTranslator` from `surfer-translation-types`.

### Remote and Protocol Layers

- **Surver HTTP server** (`surver/src/server.rs`) serves status, hierarchy, time tables, and signal payloads
- **Remote client** (`libsurfer/src/remote/client.rs`) fetches/validates remote data and converts it into local load messages
- **WCP** (`libsurfer/src/wcp/`) handles bidirectional Waveform Control Protocol commands/events
- **CXXRTL support** (`libsurfer/src/cxxrtl*/`) integrates live simulation data paths

### Data Flows

```text
WaveSource (File/Url/Cxxrtl/Data)
  -> wave_source loaders + async messages
  -> WaveContainer (Wellen/Cxxrtl) OR TransactionContainer
  -> DataContainer
  -> WaveData
  -> DisplayedItemTree + SurferTileTree
  -> drawing_canvas (wave view) / table::view (table tiles)
```

```text
Remote URL -> remote::client -> Surver endpoints
          -> hierarchy/time/signal payloads
          -> same local WaveData pipeline as file loads
```

### Key Large Files

- `lib.rs` (~122KB) - Main app wiring, update handling, and module composition
- `drawing_canvas.rs` (~68KB) - Waveform/transaction drawing command generation
- `displayed_item_tree.rs` (~47KB) - Hierarchy display
- `wave_data.rs` (~41KB) - Core loaded-data state and mutation helpers
- `table/view.rs` (~35KB) - Interactive table tile rendering

## Features

| Feature | Purpose |
|---------|---------|
| `wasm_plugins` | WASM translator plugins (default) |
| `performance_plot` | Performance visualization (default) |
| `accesskit` | Accessibility framework |
| `f128` | 128-bit float translator (requires gcc) |
| `python` | Python custom translators |

## Pre-commit Hooks

The project enforces via `.pre-commit-config.yaml`:
- File hygiene checks (`check-yaml`, `check-json`, `check-toml`, merge conflict markers, EOL/whitespace)
- `cargo fmt` - Code formatting
- `cargo check` - Build/type validation
- `cargo clippy` with specific pedantic rules (`clone_on_copy`, `needless_borrow`, `correctness`, `suspicious`)
- `cargo-sort` - Cargo.toml sorting
- `codespell` - Typo checking
- `oxipng` - PNG optimization

## Coding Style

- Prefer functional style: use iterators, `map`, `filter`, `zip`, `fold`, `flat_map` over imperative loops
- Write generic code where appropriate for reusability and long-term maintenance
- Think about software architecture quality and extensibility
- Use Rust idioms: pattern matching, `Option`/`Result` combinators, destructuring

## Patterns

- **Error handling:** Use `eyre::Result<T>` with `.context()` for error chains
- **Configuration:** RON serialization for user-facing config (`.surf.ron` files)
- **Async work:** Use channels to notify UI thread; track with `OUTSTANDING_TRANSACTIONS` counter
- **State files:** Test with `.surf.ron` state files to preserve UI state

## Testing

**Design for fully automated testing.** When planning features or changes, all functionality must be verifiable through automated tests. Manual testing is not allowed—if something cannot be tested automatically, redesign it so it can be.

Snapshot tests compare rendered images. After visual changes:
1. Run `cargo test --include-ignored`
2. Review `.new.png` files in `snapshots/`
3. Run `./accept_snapshots.bash` to accept changes
