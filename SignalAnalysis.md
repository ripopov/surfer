# Signal Analyzer — Requirements & UX Specification

## 1. Feature Overview

The Signal Analyzer is a new table model for Surfer's tile-based UI that computes statistical summaries (average, min, max, sum) of selected signals sampled at configurable trigger events. Results are broken down by marker-delimited intervals, giving users insight into signal behavior across different phases of simulation.

### Core Capabilities

- **Sampling modes**: Positive edge of a clock, any-change (counter mode), or event-type signal
- **Computed metrics**: Average, minimum, maximum, sum of sampled numeric values per signal
- **Interval segmentation**: Global statistics plus per-interval breakdown using waveform markers as interval boundaries
- **Async computation**: Analysis runs on a background worker thread
- **Persistent configuration**: Analysis spec serialized in `.surf.ron` state files via `TableModelSpec`

---

## 2. Data Model

### 2.1 Analysis Configuration

```
AnalysisConfig
  ├── sampling: SamplingConfig
  │     ├── mode: PosEdge | AnyChange | Event
  │     └── signal: VariableRef        // clock or trigger signal
  ├── signals: Vec<AnalyzedSignal>
  │     ├── variable: VariableRef
  │     ├── field: Vec<String>         // sub-field path (for structs)
  │     └── translator: String         // format name for numeric interpretation
  └── marker_snapshot: Vec<(u8, BigInt)>  // sorted copy of markers at analysis time
```

### 2.2 Sampling Modes

| Mode | Trigger Condition | Use Case |
|------|-------------------|----------|
| **PosEdge** | Sampling signal transitions to `1` (rising edge) | Synchronous clock-domain analysis |
| **AnyChange** | Every value change of the sampling signal | Asynchronous / counter-driven sampling |
| **Event** | Each event occurrence on an event-type signal | Protocol event analysis |

### 2.3 Result Schema

The analysis model produces a table with the following column structure:

```
┌──────────────┬───────────────────┬────────────────────────────────────┐
│              │                   │        Per analyzed signal         │
│  Interval    │  Info             ├─────────┬───────┬───────┬──────────┤
│  End Time    │  (label)          │ Average │  Min  │  Max  │   Sum    │
├──────────────┼───────────────────┼─────────┼───────┼───────┼──────────┤
│  t_marker1   │ start → marker1   │  12.5   │   3   │  22   │   150    │
│  t_marker2   │ marker1 → marker2 │  14.2   │   5   │  25   │   213    │
│  ...         │ ...               │  ...    │  ...  │  ...  │   ...    │
│  t_end       │ markerN → end     │  11.8   │   2   │  20   │   142    │
│  t_end       │ GLOBAL            │  12.9   │   2   │  25   │   505    │
└──────────────┴───────────────────┴─────────┴───────┴───────┴──────────┘
```

**Columns per analyzed signal** (4 columns each):
- `<signal_name>.avg` — arithmetic mean of sampled values
- `<signal_name>.min` — minimum sampled value
- `<signal_name>.max` — maximum sampled value
- `<signal_name>.sum` — sum of all sampled values

**Fixed columns**:
- `Interval End` — timestamp of the interval endpoint (formatted per user time settings)
- `Info` — human-readable interval label, e.g. `"Marker 1 → Marker 2"`

**Special rows**:
- One row per interval (N markers produce N+1 intervals: start→m1, m1→m2, ..., mN→end)
- Final row: **GLOBAL** — statistics across the entire waveform time range

### 2.4 Non-Numeric Signal Handling

Signals whose translator cannot produce `f64` values (via `translate_numeric()`) are included in the table but show `"—"` in all metric cells. This keeps the table layout consistent and makes it clear which signals lack numeric interpretation.

---

## 3. User Experience

### 3.1 Entry Point: Context Menu

Analysis is initiated from the waveform viewport context menu when one or more signals are selected.

```
┌─────────────────────────────────┐
│  Format                      ▸  │
│  Color                       ▸  │
│  ─────────────────────────────  │
│  Signal change list             │
│  Analyze selected signals...    │  ← New menu item
│  ─────────────────────────────  │
│  Copy value                     │
│  ...                            │
└─────────────────────────────────┘
```

**Visibility rules**:
- Shown when at least one `DisplayedVariable` is among the selected items
- Non-variable items (dividers, markers, streams, groups) in the selection are silently ignored
- Menu item text: **"Analyze selected signals..."** (ellipsis signals that a configuration dialog follows)

### 3.2 Configuration Wizard

After clicking "Analyze selected signals...", a configuration dialog opens. The wizard uses a single-page dialog (not multi-step) to keep the interaction fast while providing all necessary configuration.

#### Dialog Layout

```
┌─ Signal Analyzer Configuration ─────────────────────────────────┐
│                                                                  │
│  Sampling Signal                                                 │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │  top.clk                                                ▾ │  │
│  └────────────────────────────────────────────────────────────┘  │
│                                                                  │
│  Sampling Mode                                                   │
│  ┌──────────────┐ ┌──────────────┐ ┌──────────────┐             │
│  │ ● Pos. Edge  │ │ ○ Any Change │ │ ○ Event      │             │
│  └──────────────┘ └──────────────┘ └──────────────┘             │
│                                                                  │
│  Signals to analyze (3 selected)                                 │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │  ☑ top.data_out    [Unsigned ▾]                           │  │
│  │  ☑ top.counter     [Signed ▾]                             │  │
│  │  ☑ top.voltage     [Real ▾]                               │  │
│  └────────────────────────────────────────────────────────────┘  │
│                                                                  │
│  Markers: 3 markers will define 4 intervals                     │
│                                                                  │
│           ┌──────────┐  ┌──────────┐                            │
│           │  Cancel   │  │   Run    │                            │
│           └──────────┘  └──────────┘                            │
└──────────────────────────────────────────────────────────────────┘
```

#### Dialog Elements

**Sampling Signal Selector**:
- Combo box listing all variables currently displayed in the waveform viewport
- Default: first 1-bit signal in the displayed items list (heuristic for clock detection)
- Signals are shown with their display names (matching the viewport)
- The selected sampling signal is automatically excluded from the "signals to analyze" list

**Sampling Mode**:
- Radio button group with three options: `Pos. Edge`, `Any Change`, `Event`
- Default: `Pos. Edge` if sampling signal is 1-bit, `Any Change` otherwise
- `Event` mode enabled only when sampling signal has event encoding (`VariableMeta::is_event()`)

**Signals to Analyze**:
- Scrollable list of all selected variables (pre-checked)
- Each signal has a checkbox (allows deselecting individual signals)
- Each signal has a translator/format dropdown showing the current translator
- The translator determines numeric interpretation for metric computation
- At least one signal must remain checked for the "Run" button to be enabled

**Marker Info Line**:
- Read-only text showing how many markers exist and how many intervals they define
- Example: `"3 markers will define 4 intervals"` or `"No markers — only global statistics"`

**Action Buttons**:
- **Cancel**: Closes the dialog with no side effects
- **Run**: Validates configuration, closes dialog, creates analysis table tile

#### Keyboard Support

- `Enter` triggers **Run** (when focused/valid)
- `Escape` triggers **Cancel**
- `Tab` cycles through dialog fields

### 3.3 Analysis Execution Flow

```
User clicks "Run"
        │
        ▼
┌────────────────────────────┐
│  Create TableTileState     │── spec: TableModelSpec::SignalAnalysis { config }
│  with AnalysisConfig       │   config: default_view_config()
└─────────────┬──────────────┘
              │
              ▼
┌────────────────────────────┐
│  Add tile to tree          │── tile_tree.add_table_tile(id)
│  Emit BuildTableCache      │
└─────────────┬──────────────┘
              │
              ▼  (async worker thread)
┌────────────────────────────────────────┐
│  1. Iterate sampling signal changes    │
│  2. Detect trigger edges               │
│  3. At each trigger:                   │
│     - Query all analyzed signals       │
│     - Accumulate per-interval stats    │
│  4. Build result rows                  │
│  5. Return TableCache + model          │
└─────────────┬──────────────────────────┘
              │
              ▼
┌────────────────────────────┐
│  TableCacheBuilt           │── Display results in table tile
│  message received          │
└────────────────────────────┘
```

### 3.4 Result Table Tile

The analysis result appears as a standard table tile (bottom pane or horizontal split) with full table features:

- **Sorting**: Click column headers to sort by any metric
- **Filtering**: Use the filter bar to search intervals by name
- **Column visibility**: Toggle metric columns on/off
- **Copy**: Select rows and copy as TSV to clipboard
- **Row activation**: Clicking a row moves the waveform cursor to that interval's end timestamp
- **Selection mode**: Single-select with activate-on-select (clicking a row navigates the waveform)

#### Table Title

Format: `"Signal Analysis: <sampling_signal> (<mode>)"`

Example: `"Signal Analysis: top.clk (posedge)"`

### 3.5 Re-analysis and Refresh

The analysis is a snapshot computation — it does not auto-update when markers move. To support iterative workflows:

**Context menu on the analysis table tile header**:
```
┌──────────────────────────┐
│  Refresh analysis        │  ← Re-run with current markers
│  Edit configuration...   │  ← Re-open wizard with current settings
│  ─────────────────────── │
│  Close tile              │
└──────────────────────────┘
```

- **Refresh analysis**: Re-runs the analysis using the same signals and sampling config, but with the current marker positions. Emits a new `BuildTableCache` with updated marker snapshot.
- **Edit configuration...**: Re-opens the configuration wizard pre-populated with the current analysis settings. User can modify signals, sampling, then re-run.

---

## 4. Computation Algorithm

### 4.1 Trigger Point Collection

```
Input: sampling signal accessor, sampling mode
Output: sorted Vec<u64> of trigger timestamps

For each (time, value) in sampling_signal.iter_changes():
    match mode:
        PosEdge:
            if value == "1" and previous_value == "0":
                emit time as trigger point
        AnyChange:
            emit time as trigger point
        Event:
            emit time as trigger point  (every event occurrence)
```

### 4.2 Interval Definition

```
Input: sorted markers Vec<(u8, BigInt)>, waveform time range [t_start, t_end]
Output: Vec<Interval>

Sort markers by timestamp.
Remove duplicate timestamps.

Intervals:
    [t_start,    marker_1]   label: "start → Marker 1"
    [marker_1,   marker_2]   label: "Marker 1 → Marker 2"
    ...
    [marker_N,   t_end]      label: "Marker N → end"
    [t_start,    t_end]      label: "GLOBAL"

If no markers exist:
    single interval [t_start, t_end] with label "GLOBAL"
```

### 4.3 Per-Signal Accumulation

```
For each trigger timestamp t:
    Determine which interval t belongs to (binary search on interval boundaries)
    For each analyzed signal:
        query_variable(signal, t) → value
        translate_numeric(meta, value) → f64
        If valid f64 (not NaN):
            interval_accum.count += 1
            interval_accum.sum += value
            interval_accum.min = min(interval_accum.min, value)
            interval_accum.max = max(interval_accum.max, value)
            global_accum: same updates

After all triggers:
    For each interval and each signal:
        average = sum / count
        If count == 0: all metrics = "—"
```

### 4.4 Performance Considerations

- **Signal loading**: All analyzed signals and the sampling signal must be loaded before analysis begins. The async worker should request loading if needed (via `load_variable`) and wait.
- **Memory**: Accumulator state is O(intervals x signals) — negligible. Trigger point list is O(transitions of sampling signal).
- **Large waveforms**: For files with millions of clock edges, the computation is CPU-bound. The async worker thread keeps the UI responsive. A progress indication appears in the table tile (standard loading spinner from the table cache build flow).
- **Numeric precision**: Use `f64` accumulators. This provides sufficient precision for typical simulation values. Sum may lose precision for very large sample counts of large values — acceptable trade-off.

---

## 5. Serialization & State Persistence

### 5.1 TableModelSpec Extension

A new variant is added to `TableModelSpec`:

```
TableModelSpec::SignalAnalysis {
    config: SignalAnalysisConfig
}
```

Where `SignalAnalysisConfig` contains:
- `sampling_variable: VariableRef`
- `sampling_mode: SamplingMode` (PosEdge | AnyChange | Event)
- `analyzed_signals: Vec<AnalyzedSignalSpec>` (variable ref + field path + translator name)

Marker positions are **not** serialized in the spec — they are captured at analysis time from `WaveData.markers`. This means loading a `.surf.ron` file with an analysis tile will re-analyze using the markers present in the restored session.

### 5.2 Cache Invalidation

The analysis table cache should be rebuilt when:
- User explicitly requests refresh
- User edits configuration and re-runs
- Waveform data is reloaded (cache generation changes)

The analysis does **not** auto-invalidate on marker moves — this is by design, as analysis is a deliberate snapshot operation.

---

## 6. Message Flow

### 6.1 New Message Variants

```
Message::OpenSignalAnalysisWizard {
    selected_variables: Vec<(VariableRef, Vec<String>, String)>
    // (variable, field path, current translator name)
}

Message::RunSignalAnalysis {
    config: SignalAnalysisConfig
}
```

### 6.2 Interaction Sequence

```
                    User                    UI                     SystemState
                     │                       │                          │
  Right-click        │                       │                          │
  selected signals   │──── context menu ────▶│                          │
                     │                       │                          │
  Click "Analyze     │                       │                          │
  selected           │                       │                          │
  signals..."        │──── menu click ──────▶│                          │
                     │                       │── OpenSignalAnalysis ──▶│
                     │                       │   Wizard                 │
                     │                       │                          │
                     │                       │◀── open wizard dialog ──│
                     │                       │                          │
  Configure          │                       │                          │
  sampling &         │◀── dialog fields ────▶│                          │
  signals            │                       │                          │
                     │                       │                          │
  Click "Run"        │──── run button ──────▶│                          │
                     │                       │── RunSignalAnalysis ───▶│
                     │                       │                          │
                     │                       │    ┌─ create tile ──────│
                     │                       │    │  create spec       │
                     │                       │    │  add to tile tree  │
                     │                       │    │  BuildTableCache   │
                     │                       │    └──── async ─────────│
                     │                       │                          │
                     │                       │◀── TableCacheBuilt ────│
                     │                       │                          │
                     │◀── table rendered ───│                          │
```

---

## 7. Edge Cases & Validation

### 7.1 Wizard Validation

| Condition | Behavior |
|-----------|----------|
| No signals selected when opening wizard | Menu item is disabled/hidden |
| Sampling signal has no transitions | Analysis completes with empty table, info message in tile |
| No triggers fall within an interval | Interval row shows `"—"` for all metrics |
| Signal not loaded in waveform backend | Async worker loads signal before sampling |
| Sampling signal same as analyzed signal | Allowed (self-analysis is valid) |
| All analyzed signals are non-numeric | Table shows `"—"` in all metric columns; still useful for interval structure |
| No markers present | Table shows single GLOBAL row |
| Markers at identical timestamps | Duplicates removed, treated as single marker |

### 7.2 Error Handling

- **Signal load failure**: Table tile shows error via `TableCacheError::DataUnavailable`
- **Translator not found**: Falls back to default translator, metric cells show `"—"`
- **Empty waveform**: Table tile shows `"No data available"`

---

## 8. Testing Strategy

All functionality must be verifiable through automated tests.

### 8.1 Unit Tests (table/sources/signal_analysis.rs)

- **Trigger detection**: Verify PosEdge/AnyChange/Event modes produce correct trigger timestamps from known signal data
- **Interval construction**: Test marker sorting, deduplication, boundary computation
- **Accumulator math**: Verify average/min/max/sum with known inputs, including edge cases (single sample, zero values, negative values)
- **Non-numeric signals**: Verify `"—"` output for signals without numeric translation
- **Empty intervals**: Verify correct handling when no triggers fall in an interval
- **TableModel trait compliance**: Schema, row_count, cell, sort_key, search_text, on_activate

### 8.2 Integration Tests

- **Round-trip serialization**: `SignalAnalysisConfig` survives RON serialize/deserialize
- **Cache build flow**: End-to-end `BuildTableCache` → `TableCacheBuilt` with analysis spec
- **Row activation**: Clicking interval row sets cursor to correct timestamp

### 8.3 Snapshot Tests

- **Wizard dialog rendering**: Visual snapshot of the configuration dialog
- **Result table rendering**: Visual snapshot of a populated analysis table

---

## 9. Future Extensions (Out of Scope)

The following are explicitly out of scope for the initial implementation but inform the design:

- **Custom metrics**: User-defined expressions (e.g., standard deviation, RMS)
- **Falling edge / dual-edge sampling**: Additional sampling modes
- **Auto-refresh on marker move**: Reactive re-analysis
- **Export to CSV**: Direct file export of analysis results
- **Histogram visualization**: Graphical distribution of sampled values
- **Cross-signal correlation**: Metrics comparing relationships between signals
