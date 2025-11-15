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

### Y-Axis Scaling

- **Viewport**: The Y-axis range adjusts dynamically based on the min/max values visible in the current view. Zooming in reveals more detail. Use this when:
  - Examining small variations in a signal
  - The signal range varies significantly across the simulation
  - You want maximum visual resolution for the current view

- **Global**: The Y-axis range is fixed to the overall min/max of the entire signal. Use this when:
  - Comparing relative magnitudes across different time regions
  - The absolute value matters (e.g., verifying a signal stays within bounds)
  - Multiple signals should share a common scale for comparison

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
- **Unsigned/Signed**: Direct integer interpretation
- **Hex/Binary**: Parsed as integers for plotting
- **Float (IEEE 754, bfloat16, posit, etc.)**: Native floating-point values
- **Custom translators**: Any translator returning numeric strings

Non-numeric values (X, Z, undefined states) are rendered as highlighted regions rather than plotted points

---

## CXXRTL Limitations

Analog rendering is **not supported** for CXXRTL live simulation connections due to architectural differences from file-based waveforms.
CXXRTL stores data in a `BTreeMap` structure, uses `VariableRef` identifiers, and fetches data remotely—requiring either memory-intensive
local snapshots or protocol changes for server-side range queries. Supporting CXXRTL analog would require a unified cache key type,
a custom signal accessor, version-based invalidation for live data, and updates across message types and renderers.

---

## Design Decisions / Developer Notes

### 0. Signal Representation for Analog Rendering

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

### 1. Why the System Was Designed This Way

The analog rendering system addresses several key challenges:

**Scalability**: Waveform files can contain millions of signal transitions. Naively iterating through all transitions for every frame would be prohibitively slow. The RMQ-based caching approach enables O(log N + 1) queries regardless of signal size.

**Anti-Aliasing**: When zoomed out, multiple transitions may map to a single pixel. Without special handling, peaks and troughs would be lost due to sampling aliasing. The per-pixel min/max tracking ensures accurate visual representation at all zoom levels.

**Responsiveness**: Cache building for large signals can take significant time. Async cache construction prevents UI freezes while building caches in background threads.

**Cache Invalidation**: Translator changes affect numeric interpretation. The cache key includes the translator name, so changing translators automatically uses a different cache entry.

### 2. Internal Architecture

The analog rendering pipeline consists of these components:

```
┌─────────────────┐     ┌───────────────────┐     ┌─────────────────────┐
│ WaveContainer   │────▶│ AnalogSignalCache │────▶│ AnalogDrawingCommands│
│ (wellen Signal) │     │ (SignalRMQ)       │     │ (per-frame)         │
└─────────────────┘     └───────────────────┘     └─────────────────────┘
        │                       │                          │
        │ SignalAccessor        │ O(1) range queries       │ Draw commands
        │ (Arc<Signal>)         │                          │
        ▼                       ▼                          ▼
   Translator              build_analog_           draw_analog()
   (value→f64)             drawing_commands()      (egui rendering)
```

**Key files:**
- `analog_renderer.rs`: Drawing command generation and rendering
- `analog_signal_cache.rs`: Cache structure and builder
- `signal_rmq.rs`: Range Min/Max Query data structure
- `drawing_canvas.rs`: Integration with draw command system
- `wave_data.rs`: Cache storage and async build orchestration

### 3. Analog Cache Structure and Operation

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
- `SignalRef` is the canonical signal identity (handles variable aliases pointing to the same signal)
- `String` is the translator name (different translators produce different numeric interpretations)

This design enables cache sharing: multiple displayed variables that are aliases to the same underlying signal with the same translator will share a single cache entry.

**Cache Validity**: A cache is valid when `num_timestamps` matches the waveform length. The signal identity and translator are implicit in the cache key lookup.

**Cache Storage**: Caches are stored in `WaveData::analog_signal_caches` keyed by `(SignalRef, String)`.

**Cache Invalidation**: The cache uses a mark-and-sweep invalidation strategy, which simplifies the codebase by eliminating explicit per-variable invalidation calls.

*Mark-and-sweep approach* (`SweepUnusedAnalogCaches` message):
- During draw command generation, each analog variable that successfully uses its cache reports the cache key via `VariableDrawCommands::used_cache_key`
- After processing all variables, `generate_wave_draw_commands()` collects all used cache keys and sends a `SweepUnusedAnalogCaches { used_keys }` message
- The message handler retains only caches present in `used_keys`, automatically removing:
  - Caches for variables no longer displayed
  - Caches for variables with analog mode disabled
  - Stale caches from previous waveform loads

*Cache validation in rendering* (`variable_analog_draw_commands`):
- The cache key `(SignalRef, translator_name)` ensures the correct cache is retrieved
- Before using a cache, the renderer validates that `num_timestamps` matches the current waveform length
- If validation fails (e.g., waveform was reloaded), the cache is not marked as "used" and will be swept
- A `BuildAnalogCache` message is returned to trigger async rebuild

*Why mark-and-sweep?*
- **Simplicity**: No need for explicit invalidation calls scattered across message handlers
- **Correctness**: The rendering pipeline knows exactly which caches are valid and in use
- **Automatic cleanup**: Stale caches are removed without explicit tracking of every invalidation scenario

*Proactive cache building* (`SetAnalogSettings` handler):
- When analog mode is enabled, a `BuildAnalogCache` message is sent immediately
- This optimization ensures the cache starts building before the first render attempt
- Without this, users would see a blank waveform briefly while waiting for async build

*All caches cleared* (via `HashMap::new()` on `WaveData` creation):
- **New waveform loaded**: When `update_with_waves()` creates a new `WaveData` instance (`wave_data.rs:206`), all caches are implicitly cleared since the entire `analog_signal_caches` HashMap is initialized empty
- **New wave source opened**: When opening a new VCD/FST/GHW file (`state.rs:258`, `state.rs:319`), a fresh `WaveData` is created with empty caches
- **Waveform reload**: Reloading the current waveform also clears all caches

### 4. Async Cache Building

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

### 5. Drawing Command Generation

The `build_analog_drawing_commands()` function generates minimal draw instructions using a hybrid approach that combines pixel-by-pixel iteration (for correct Range commands in dense regions) with signal-centric positioning (for correct interpolation at viewport edges).

**Algorithm:**
```
1. Query sample at viewport start to get the BEFORE sample position
   - If sample is before viewport (negative pixel), record its position for interpolation

2. For each pixel in [start_px, end_px]:
    t0, t1 = time range for this pixel

    if signal is flat (no change in [t0, t1]):
        extend or emit Flat command
        jump ahead to next_change pixel (optimization)
    else:
        query cache for (min, max) in [t0, t1)
        emit Range command

3. Query sample after viewport end for interpolation
   - If there's a transition after viewport, include it for right-edge interpolation

4. Finalize pending commands, ensuring first command starts from before-viewport sample
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

### 6. SignalRMQ Data Structure

`SignalRMQ` implements blocked Range Minimum/Maximum Query with sparse tables:

**Structure:**
```rust
pub struct SignalRMQ {
    timestamps: Vec<u64>,        // Sorted timestamps
    values: Vec<f64>,            // Corresponding values
    block_size: usize,           // Typically 64
    block_summaries: Vec<MinMax>,// Per-block min/max
    sparse_table: Vec<Vec<MinMax>>,// 2^k range queries
}
```

**Query Algorithm:**
1. Binary search to convert time range [t_start, t_end] to index range [L, R]
2. Handle partial blocks at boundaries with linear scan
3. Use sparse table for O(1) query over complete blocks in the middle
4. Combine results

**Complexity:**
- Construction: O(N + N/B · log(N/B)) time, O(N + N/B · log(N/B)) space
- Time-range query: O(log N) for binary search + O(1) for RMQ = O(log N)
- Index-range query: O(1)

**Memory Efficiency**: Block size of 64 reduces sparse table overhead by ~64x compared to full RMQ while maintaining O(1) block queries.

### 7. Rendering Implementation

`draw_analog()` in `analog_renderer.rs` renders commands to the egui painter:

**Step Mode (`render_step_mode`):**
- Flat commands: horizontal line from start to end at normalized Y
- Range commands: vertical bar from min to max, connected to previous with step transitions
- Pixel positions are clamped to `[min_valid_pixel, max_valid_pixel]` before rendering

**Interpolated Mode (`render_interpolated_mode`):**
The interpolated renderer iterates through commands directly (not using a generic callback) to handle several edge cases:

- **Left edge interpolation**: When the first command's start position is before viewport (negative), the renderer draws a diagonal line from the clamped left edge to the first visible sample point
- **Normal interpolation**: Diagonal lines connect consecutive sample points
- **Transition to NaN**: When the next sample is NaN (Z/X state), a horizontal line is drawn to the NaN region start (can't interpolate toward undefined values)
- **Right edge handling**: The last command may extend beyond viewport; horizontal line drawn to clamped right edge
- **Range commands**: Zigzag through both min and max to capture the signal envelope

**Special Value Handling:**
- NaN values (from X, Z states) render as colored rectangles using `ValueKind::Undef` color
- When transitioning from a finite value to NaN, the renderer draws a horizontal line to the NaN region start rather than attempting interpolation
- Infinite values are skipped

**Coordinate Transform:**
```rust
let normalized_value = (value - min_val) / value_range;
let y = (1.0 - normalized_value) * line_height * height_scaling_factor + offset;
```

**Valid Pixel Bounds**: `min_valid_pixel` and `max_valid_pixel` clip rendering to the actual waveform time range. Commands with positions outside these bounds are clamped, enabling correct interpolation even when sample points are outside the viewport.

---