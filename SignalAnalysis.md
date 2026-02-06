# Signal Analyzer — Requirements & UX Specification

## 1. Feature Overview

The Signal Analyzer is a new table model for Surfer's tile-based UI that computes statistical summaries (average, min, max, sum) of selected signals sampled at trigger events inferred from the chosen sampling signal. Results are broken down by marker-delimited intervals, giving users insight into signal behavior across different phases of simulation.

### Core Capabilities

- **Automatic sampling mode selection**: Sampling behavior is inferred from the selected sampling signal type/encoding
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
  │     └── signal: VariableRef        // clock / counter / event trigger signal
  ├── signals: Vec<AnalyzedSignal>
  │     ├── variable: VariableRef
  │     ├── field: Vec<String>         // reserved for v2 (field-level numeric metrics)
  │     └── translator: String         // format name for numeric interpretation
  └── run_revision: u64                // incremented on refresh/re-run

AnalysisRunContext (runtime-only, not serialized)
  ├── resolved_sampling_mode: PosEdge | AnyChange | Event
  ├── marker_snapshot: Vec<(u8, BigInt)>  // sorted marker copy at run start
  └── time_range: (BigInt, BigInt)        // waveform start/end at run start
```

### 2.2 Automatic Sampling Mode Selection

Sampling mode is inferred automatically from the selected sampling signal:

| Sampling signal kind | Resolved mode | Trigger Condition |
|----------------------|---------------|-------------------|
| **VCD event signal** (`VariableMeta::is_event()`) | **Event** | Each event occurrence |
| **1-bit signal** | **PosEdge** | Transition to logic `1` from logic `0` |
| **n-bit signal** (n > 1) | **AnyChange** | Every value change |

Precedence rule: event encoding is checked first, then bit-width.

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
- The selected sampling signal remains eligible for analysis (self-analysis is allowed)

**Sampling Mode (read-only)**:
- Display-only label derived from the selected sampling signal:
- Event signal → `Event`
- 1-bit signal → `Pos. Edge`
- n-bit signal → `Any Change`

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
│  Create TableTileState     │── spec: TableModelSpec::AnalysisResults { kind, params }
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
│  2. Detect trigger events (inferred mode) │
│  3. At each trigger:                   │
│     - Query all analyzed signals       │
│     - Accumulate per-interval stats    │
│  4. Build model rows                   │
│  5. Return TableCache                  │
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

Format: `"Signal Analysis: <sampling_signal> (<resolved_mode>)"`

Example: `"Signal Analysis: top.clk (posedge)"`

### 3.5 Re-analysis and Refresh

The analysis is a snapshot computation — it does not auto-update when markers move. To support iterative workflows:

**Inline actions in the table tile (v1)**:
```
┌──────────────────────────┐
│  Refresh analysis        │  ← Re-run with current markers
│  Edit configuration...   │  ← Re-open wizard with current settings
│  ─────────────────────── │
│  Close tile              │
└──────────────────────────┘
```

- **Refresh analysis**: Re-runs analysis using current markers and the same sampling config. It increments `run_revision` and forces model/cache rebuild even if sort/filter/generation are unchanged.
- **Edit configuration...**: Re-opens the configuration wizard pre-populated with the current analysis settings. User can modify sampling signal and analyzed signals, then re-run.

---

## 4. Computation Algorithm

### 4.1 Trigger Point Collection

```
Input: sampling signal accessor + sampling signal metadata
Output: sorted Vec<u64> of trigger timestamps

resolved_mode = infer_mode(meta):
    if meta.is_event(): Event
    else if meta.num_bits == 1: PosEdge
    else: AnyChange

For each (time, value) in sampling_signal.iter_changes():
    match resolved_mode:
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
Drop markers outside [t_start, t_end].

Intervals:
    [t_start,    marker_1)   label: "start → Marker 1"
    [marker_1,   marker_2)   label: "Marker 1 → Marker 2"
    ...
    [marker_N,   t_end]      label: "Marker N → end"   // inclusive end for final interval
    [t_start,    t_end]      label: "GLOBAL"

If no markers exist:
    single interval [t_start, t_end] with label "GLOBAL"
```

### 4.3 Per-Signal Accumulation

```
For each trigger timestamp t:
    Determine which interval t belongs to (binary search on interval boundaries)
    For each analyzed signal:
        query_variable(signal, t) → current value at-or-before t
        translate_numeric(meta, value) → f64   // v1: only root variable metrics
        If valid f64 (not NaN):
            interval_accum.count += 1
            interval_accum.sum += value
            interval_accum.min = min(interval_accum.min, value)
            interval_accum.max = max(interval_accum.max, value)
            global_accum: same updates
        Else:
            keep metric cells as "—" for that signal/interval

After all triggers:
    For each interval and each signal:
        average = sum / count
        If count == 0: all metrics = "—"
```

### 4.4 Performance Considerations

- **Signal loading**: Missing signals are requested before launching analysis work (preflight on UI/update thread). The table model itself remains read-only; it does not mutate/load waveform data from worker threads.
- **Memory**: Accumulator state is O(intervals x signals) — negligible. Trigger point list is O(transitions of sampling signal).
- **Large waveforms**: For files with millions of clock edges, the computation is CPU-bound. The async worker thread keeps the UI responsive. A progress indication appears in the table tile (standard loading spinner from the table cache build flow).
- **Numeric precision**: Use `f64` accumulators. This provides sufficient precision for typical simulation values. Sum may lose precision for very large sample counts of large values — acceptable trade-off.

---

## 5. Serialization & State Persistence

### 5.1 TableModelSpec Extension

Use the existing analysis slot in `TableModelSpec` (architecture-aligned):

```
TableModelSpec::AnalysisResults {
    kind: AnalysisKind::SignalAnalysisV1,
    params: AnalysisParams::SignalAnalysis(SignalAnalysisConfig)
}
```

Where `SignalAnalysisConfig` contains:
- `sampling_variable: VariableRef`
- `analyzed_signals: Vec<AnalyzedSignalSpec>` (variable ref + field path + translator name)
- `run_revision: u64`

Marker positions are **not** serialized in the spec — they are captured at run time from `WaveData.markers`.

### 5.2 Cache Invalidation

The analysis table cache should be rebuilt when:
- User explicitly requests refresh
- User edits configuration and re-runs
- Waveform data is reloaded (cache generation changes)
- Model/context revision changes (time format, translator-set changes, variable display-format changes)

To make refresh deterministic, the model/cache identity must include `run_revision`
(for example via `TableModelKey`/cache-key model revision input), so repeated refreshes
cannot be deduplicated as a no-op.

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

Message::RefreshSignalAnalysis {
    tile_id: TableTileId
}

Message::EditSignalAnalysis {
    tile_id: TableTileId
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
                     │                       │── OpenSignalAnalysisWizard ─▶│
                     │                       │                          │
                     │                       │◀── open wizard dialog ──│
                     │                       │                          │
  Configure          │                       │                          │
  sampling signal &  │◀── dialog fields ────▶│                          │
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
| Sampling signal kind changes | Resolved sampling mode updates automatically |
| Sampling signal has no transitions | Analysis completes with interval/global rows and `"—"` metrics |
| No triggers fall within an interval | Interval row shows `"—"` for all metrics |
| Signal not loaded in waveform backend | Preflight requests load; model returns `DataUnavailable` until data arrives |
| A non-empty `field` is configured | Supported as metadata in config; numeric metrics remain root-level in v1 (`"—"` if field-only value) |
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

- **Mode inference**: Verify automatic mapping `event -> Event`, `1-bit -> PosEdge`, `n-bit -> AnyChange`
- **Trigger detection**: Verify inferred mode produces correct trigger timestamps from known signal data
- **Interval construction**: Test marker sorting, deduplication, boundary computation
- **Accumulator math**: Verify average/min/max/sum with known inputs, including edge cases (single sample, zero values, negative values)
- **Non-numeric signals**: Verify `"—"` output for signals without numeric translation
- **Empty intervals**: Verify correct handling when no triggers fall in an interval
- **TableModel trait compliance**: Schema, row_count, cell, sort_key, search_text, on_activate

### 8.2 Integration Tests

- **Round-trip serialization**: `SignalAnalysisConfig` survives RON serialize/deserialize
- **Cache build flow**: End-to-end `BuildTableCache` → `TableCacheBuilt` with analysis spec
- **Row activation**: Clicking interval row sets cursor to correct timestamp
- **Refresh semantics**: Refresh forces rebuild when sort/filter/generation are unchanged
- **Stale-result safety**: Late worker result from old run revision is dropped

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
