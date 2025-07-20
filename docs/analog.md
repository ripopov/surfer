# Analog Signal Rendering System

This document describes the analog signal visualization system implemented in Surfer, covering both user-facing features and internal implementation details.

## Feature Description

Analog rendering mode displays numeric signals as continuous waveforms rather than discrete digital transitions.
Analog visualization is particularly valuable when the **numeric magnitude** of a multi-bit signal carries meaning beyond its binary representation. Common use cases include:

- **Data converters**: ADC outputs, DAC inputs, and sigma-delta modulators—visualize quantization behavior, missing codes, and reconstructed signal envelopes
- **DSP pipelines**: Filter responses (FIR/IIR/CIC), FFT magnitudes, CORDIC outputs, and fixed-point arithmetic showing overflows and saturation
- **FIFO fill levels, Scoreboard queues**: Watermark signals and buffer occupancy, outstanding transaction counts.
- **Event counters**: Packet counts, error tallies, and performance metrics over time.

### Enabling Analog Mode

Right-click on a variable in the waveform view and select the analog rendering style from the context menu. Available options:

- **Off** - Standard digital rendering (default)
- **Step (Viewport)** - Step-function rendering with Y-axis scaled to visible range
- **Step (Global)** - Step-function rendering with Y-axis scaled to entire signal range
- **Interpolated (Viewport)** - Linear interpolation between values, Y-axis scaled to visible range
- **Interpolated (Global)** - Linear interpolation between values, Y-axis scaled to entire signal range

Note: The Analog submenu is only shown for waveforms loaded via the wellen backend (VCD/FST/GHW). It is hidden for other backends, such as CXXRTL live simulations and transaction/FTR sources.

### Amplitude Labels

Min and max value labels are displayed at the right edge of the waveform area, showing the current Y-axis range. These update based on the selected scaling mode:
- In Viewport mode: shows the visible min/max
- In Global mode: shows the overall signal min/max

### Anti-Aliasing Behavior

When zoomed out such that multiple signal transitions occur within a single pixel, the renderer displays a vertical bar spanning the true min/max range for that pixel.
This ensures no peaks or troughs are visually lost due to aliasing—a critical feature when:
- Viewing long simulations at overview zoom levels
- Identifying glitches or outliers that would otherwise be invisible
- Verifying that a signal stays within expected bounds across the entire simulation

### Translator Compatibility

Analog mode works with any translator that produces numeric output:
- Unsigned/Signed: Direct integer interpretation
- Hex/Binary: Parsed as integers for plotting
- Float (IEEE 754, bfloat16, fp8 etc.)
- Custom translators: Any translator returning numeric strings

Non-numeric values (X, Z, undefined states) are rendered as red-highlighted regions rather than plotted points

---

## Performance Considerations

- To improve performance, analog rendering is built on top of a cache that stores pre-computed signal ranges and min/max values.
- After the cache is built, analog rendering is effectively as fast as digital rendering; frames reuse cached data and pay only the per-frame draw cost.
- Cache construction is the expensive step. Profiling shows the time is dominated by parsing numeric strings produced by translators.
- Real-valued signals currently incur a double conversion (f64 → string → f64) in the value pipeline:
  `wellen::SignalValue::Real(value) => VariableValue::String(format!("{value}"))`
  This adds overhead and could be optimized by threading `f64` values through without formatting.
  - This is a subject for future optimizations: Translators should be refactored to provide a fast binary to numeric conversion.

---

---

## CXXRTL Limitations

Analog rendering is not supported for CXXRTL live simulation connections due to architectural differences from file-based waveforms.
CXXRTL stores data in a `BTreeMap` structure, uses `VariableRef` identifiers, and fetches data remotely—requiring either memory-intensive
local snapshots or protocol changes for server-side range queries. Supporting CXXRTL analog would require a unified cache key type,
a custom signal accessor, version-based invalidation for live data, and updates across message types and renderers.

---

## Design Decisions / Developer Notes

### Signal Representation for Analog Rendering

Analog signals are represented using two complementary data structures:

1. **`AnalogSignalCache`** (`analog_signal_cache.rs`): A pre-computed cache containing:
   - `SignalRMQ`: Range Min/Max Query structure for O(1) range queries
   - `global_min` / `global_max`: Signal bounds across entire time range
   - `num_timestamps`: Total time units (for cache validity checking)

   Caches are keyed by `(SignalRef, translator_name)` in the parent HashMap, enabling cache sharing across aliased variables that point to the same underlying signal.

2. **`AnalogDrawingCommands`** (`drawing_canvas.rs`): Per-frame drawing instructions containing:
   - `viewport_min` / `viewport_max`: Y-axis bounds for visible region
   - `global_min` / `global_max`: Copied from cache for global scaling mode
   - `values`: Vector of `AnalogDrawingCommand` (flat spans or per-pixel ranges)
   - `min_valid_pixel` / `max_valid_pixel`: Clipping bounds

   Note: `AnalogDrawingCommand` uses `f32` for pixel positions (`start_px`, `end_px`) to support samples outside viewport bounds (negative values for samples before viewport, values > view_width for samples after viewport). This enables correct interpolation at viewport edges.

Signal values are converted from raw bit vectors to `f64` via the translator system. The `parse_numeric_value()` function handles format-specific parsing (hex, binary, decimal, float) based on the translator name.

### Analog Cache Structure and Operation

The `AnalogSignalCache` wraps a `SignalRMQ` structure optimized for time-range queries:

```rust
// Keyed by (SignalRef, translator_name) in WaveData::analog_signal_caches
pub struct AnalogSignalCache {
    pub rmq: SignalRMQ,
    pub global_min: f64,
    pub global_max: f64,
    pub num_timestamps: u64,
}
```

**Cache Key**: `(wellen::SignalRef, String)` where:
- `SignalId` is the canonical signal identity (handles variable aliases pointing to the same signal)
- `String` is the translator name (different translators produce different numeric interpretations)

This design enables cache sharing: multiple displayed variables that are aliases to the same underlying signal with the same translator will share a single cache entry.

**Cache Validity**: A cache is valid when `num_timestamps` matches the waveform length. The signal identity and translator are implicit in the cache key lookup.
**Cache Invalidation**: The cache uses a mark-and-sweep invalidation strategy, which simplifies the codebase by eliminating explicit per-variable invalidation calls.

*Mark-and-sweep approach* (`SweepUnusedAnalogCaches` message):
- During draw command generation, each analog variable that successfully uses its cache reports the cache key via `VariableDrawCommands::used_cache_key`
- After processing all variables, `generate_wave_draw_commands()` collects all used cache keys and sends a `SweepUnusedAnalogCaches { used_keys }` message
- The message handler retains only caches present in `used_keys`, automatically removing:
  - Caches for variables no longer displayed
  - Caches for variables with analog mode disabled

*Cache validation in rendering* (`variable_analog_draw_commands`):
- The cache key `(SignalRef, translator_name)` ensures the correct cache is retrieved
- Before using a cache, the renderer validates that `num_timestamps` matches the current waveform length
- If validation fails (e.g., waveform was reloaded), the cache is not marked as "used" and will be swept
- A `BuildAnalogCache` message is returned to trigger async rebuild

### Async Cache Building

Cache construction runs on background threads to prevent UI blocking:

**Flow:**
1. `variable_analog_draw_commands()` checks for valid cache using key `(SignalRef, translator_name)`
2. If missing/invalid, returns `Message::BuildAnalogCache { signal_ref, translator_name, variable_ref }`
3. `WaveData::build_analog_cache_async()` spawns background work via `async_util::perform_work()`
4. Worker creates `SignalAccessor` (cheap Arc clones), iterates signal, builds RMQ
5. On completion, sends `Message::AnalogCacheBuilt { cache_key: (SignalRef, String), cache, error }`
6. Main thread inserts cache into `analog_signal_caches` HashMap keyed by `(SignalRef, translator_name)`
7. Next frame uses the new cache (all aliased variables benefit from the same cache)

**Thread Safety**: The `SignalAccessor` holds `Arc<Signal>` and `Arc<TimeTable>`, enabling zero-copy transfer to worker threads. `TranslatorList` uses `Arc<DynTranslator>` for the same reason.
**Progress Tracking**: `WaveData::cache_build_in_progress` tracks pending builds. Status bar displays "Building analog cache..." when builds are in progress.
**Repaint Triggering**: Workers increment `OUTSTANDING_TRANSACTIONS` and call `EGUI_CONTEXT.request_repaint()` to ensure the UI updates when cache completes.

**Why `OUTSTANDING_TRANSACTIONS` instead of `progress_tracker`?**
Unifying these would require changing `progress_tracker` to `Vec<LoadProgress>`, adding new status variants, and managing concurrent entry lifecycle—added complexity.

### Drawing Command Generation

The `CommandBuilder` struct in `analog_renderer.rs` generates minimal draw instructions using a hybrid approach that combines pixel-by-pixel iteration (for correct Range commands in dense regions) with signal-centric positioning (for correct interpolation at viewport edges).

**Algorithm:**
```
1. add_before_viewport_sample(): Query sample at viewport start
   - If sample is before viewport (negative pixel), record its position for interpolation

2. iterate_pixels(): For each pixel in [0, end_px]:
    t0, t1 = time range for this pixel

    if signal is flat (no change in [t0, t1]):
        extend or emit Flat command
        jump ahead to next_change pixel (optimization)
    else:
        query cache for (min, max) in [t0, t1)
        emit Range command

3. add_after_viewport_sample(): Query sample after viewport end
   - If there's a transition after viewport, include it for right-edge interpolation

4. finalize(): Flush pending commands, ensuring first command starts from before-viewport sample
```

**Command Types:**
- `CommandKind::Flat { value, end_px }`: Constant value spanning pixels (end_px is f32, can extend beyond viewport)
- `CommandKind::Range { min, max }`: Pixel with multiple transitions, draw vertical bar

**Why Hybrid Approach**: The algorithm must handle two distinct scenarios:
- **Dense regions** (many transitions per pixel): Requires pixel-by-pixel iteration to correctly detect and emit Range commands for anti-aliasing
- **Sparse regions** (few transitions): Requires signal-centric positioning to correctly interpolate at viewport boundaries

A purely signal-centric approach would skip over pixels needing Range commands. A purely pixel-centric approach would not include samples outside the viewport needed for edge interpolation.

**Jump-Ahead Optimization**: When a flat region is detected, the algorithm computes the pixel position of `next_change` and skips directly there, avoiding per-pixel iteration over constant regions.

**Viewport Edge Interpolation**: For interpolated rendering mode, the algorithm includes:
- The sample BEFORE the viewport (may have negative pixel position) for left-edge interpolation
- The sample AFTER the viewport (may have pixel > view_width) for right-edge interpolation

This ensures diagonal interpolation lines are drawn correctly even when the actual sample points are outside the visible viewport.

**Viewport Min/Max Tracking**: As commands are generated, viewport bounds are accumulated for Y-axis scaling in Viewport mode.
