# Fused Waveform and FTR Sources

## Purpose

Surfer should support the open-source equivalent of a commercial mixed waveform
database: a waveform file such as `cpu_sim.vcd` or `cpu_sim.fst` and a
transaction trace such as `cpu_sim.ftr` opened together, visible in the same
sidebar, and mixable on one timeline canvas.

The target is not a special two-file hack. The implementation should lift the
current single-source assumption to an N-source session model where each source
still owns exactly one parsed database, while the canvas, cursors, markers,
annotations, tables, and hierarchy views operate over source-qualified items.

## Current Evidence

The current architecture is intentionally single-source:

- `UserState` stores one `Option<WaveData>` at `libsurfer/src/state.rs:90`, and
  `previous_waves` is also one `Option<WaveData>` at
  `libsurfer/src/state.rs:95`.
- `WaveData` stores one `DataContainer`, one `WaveSource`, and one
  `WaveFormat` at `libsurfer/src/wave_data.rs:58`.
- `DataContainer` is an exclusive enum of `Waves`, `Transactions`, or `Empty` at
  `libsurfer/src/data_container.rs:9`.
- Opening a file routes `.ftr` to transaction loading and every other extension
  to waveform loading at `libsurfer/src/wave_source.rs:220`.
- Loaded waveform and transaction messages both replace or update the same
  `self.user.waves` slot through `on_waves_loaded` at
  `libsurfer/src/state.rs:363` and `on_transaction_streams_loaded` at
  `libsurfer/src/state.rs:487`.
- The load messages carry `LoadOptions`, but not an identity for "which source
  this response belongs to", at `libsurfer/src/message.rs:160` and
  `libsurfer/src/message.rs:201`.
- The hierarchy renders one `WaveData`: the separate view reads
  `self.user.waves` at `libsurfer/src/hierarchy.rs:119`, and
  `draw_all_scopes` iterates the current container's root scopes at
  `libsurfer/src/hierarchy.rs:422`.
- The canvas draw cache chooses either waveform drawing or transaction drawing
  for the whole session from `waves.inner` at
  `libsurfer/src/drawing_canvas.rs:541`.
- The transaction renderer then assumes every displayed stream belongs to that
  one transaction container at `libsurfer/src/drawing_canvas.rs:678`.
- `DisplayedItem::Variable` and `DisplayedItem::Stream` are canvas rows, but
  neither carries a source id today; see `libsurfer/src/displayed_item.rs:61`
  and `libsurfer/src/displayed_item.rs:345`.
- `TransactionStreamRef` and `TransactionRef` identify FTR data only by FTR IDs,
  which may collide between files, at `libsurfer/src/transaction_container.rs:247`
  and `libsurfer/src/transaction_container.rs:309`.
- The shared viewport, cursor, and marker model already lives in `WaveData` at
  `libsurfer/src/wave_data.rs:70`, `libsurfer/src/wave_data.rs:71`, and
  `libsurfer/src/wave_data.rs:72`.

These anchors imply the correct change: split source ownership from shared view
state, then make every source-owned reference explicit.

## UX

### Opening Files

The file menu should expose three workflows:

1. `Open...`
   Replaces the whole session, matching today's behavior. A single VCD/FST/GHW
   opens as waveform-only. A single FTR opens as transaction-only.

2. `Add Source...`
   Adds one file to the current session without replacing loaded sources or
   canvas rows. This is the generic path for adding `cpu_sim.ftr` after
   `cpu_sim.vcd`, or adding the waveform after the FTR was opened first.

3. `Open Waveform + Transactions...`
   Opens a multi-select dialog filtered to `*.vcd`, `*.fst`, `*.ghw`, and
   `*.ftr`. The dialog accepts two or more files, loads them as an additive
   batch, validates that they share the same time domain, and reports all
   rejected files together. This should be the primary UX for the common
   `cpu_sim.vcd + cpu_sim.ftr` case.

The same semantics should be available at startup:

```text
surfer cpu_sim.vcd cpu_sim.ftr
```

The current startup model accepts only one wave source through
`StartupParams.waves` at `libsurfer/src/lib.rs:170`; it should become an ordered
source list. The first file is loaded with `ReplaceSession`, and subsequent
files are loaded with `AddSource` so command-line, file-dialog, drag-and-drop,
and test-harness behavior all converge on one load path.

Drag and drop follows the same rules. Dropping multiple files on an empty
session behaves like `Open Waveform + Transactions...`; dropping onto a non-empty
session behaves like `Add Source...` for each file. If only one of the pair is
dropped, Surfer should not guess a sibling path automatically in phase 1; it can
show a small "Add matching FTR..." affordance later, but the implementation does
not depend on filename similarity.

A later UX phase may offer sibling auto-detection: after opening `cpu_sim.vcd`,
probe for `cpu_sim.ftr`, and after opening `cpu_sim.ftr`, probe for a same-stem
waveform file. This should be an explicit `Ask`/`Always`/`Never` behavior, not
silent loading. The existing sibling state-file flow is the pattern to reuse:
newly loaded data checks for a sibling state file at `libsurfer/src/state.rs:481`,
and `SuggestOpenSiblingStateFile` is gated through the autoload policy at
`libsurfer/src/lib.rs:1640`.

### Sidebar Layout

The hierarchy sidebar should be source-sectioned.

In tree mode, the top level is:

```text
cpu_sim.vcd  VCD
  tb
    cpu
      clk
      rst
      ...
cpu_sim.ftr  FTR
  tr
    pipelined_stream
      read
      write
```

In separate mode, both panes are still source-sectioned:

- The scopes pane shows source headers and scope roots below each source.
- The variables pane shows variables/generators for the currently active
  source-qualified scope.

In variables mode, all variables/generators are grouped by source header. The
current "Streams are not yet supported" branch for transaction-only all-variable
view at `libsurfer/src/hierarchy.rs:407` should disappear; FTR generators are
valid entries in a multi-source variable list.

Source headers should show:

- Display label: basename for files, URL host/path for URLs, `CXXRTL` for live
  sources, `Dropped file` for anonymous data.
- Format badge: `VCD`, `FST`, `GHW`, `FTR`, or `CXXRTL`.
- Time domain badge: for example `1 ns, 0..125000`.
- Progress/error state when a source is loading or rejected.
- Actions in a context menu: `Reload Source`, `Close Source`, `Rename Source`,
  and `Reveal in File History` when applicable.

Labels must disambiguate without noise. If only one source is open, keep today's
labels. If multiple sources are open, add a source prefix or compact pill where
names can collide: `cpu_sim: clk`, `cpu_sim.ftr: write`. The source label should
also be visible in tooltips and table titles.

### Mixing Items On One Canvas

Users can add waveform signals and transaction streams/generators in any order.
The central canvas remains a single ordered `items_tree`:

```text
clk
rst
instruction
Divider
data_stream.write
pc
```

Rows retain their existing visual semantics:

- Signal rows render digital/analog waveforms from their source's
  `WaveContainer`.
- FTR stream/generator rows render transactions from their source's
  `TransactionContainer`.
- Dividers, timelines, markers, annotations, groups, arrows, and viewport
  controls remain session-level.

The row label panel should add a source cue only when useful. A compact source
pill or left color strip is enough; do not duplicate long paths in every row.
Context menus should include `Reveal in Hierarchy` and `Show Source` for
source-owned rows.

### Errors

Additive loading is transactional at the source level. If `cpu_sim.ftr` is
rejected because its time domain differs, the already loaded waveform remains
unchanged. The error should identify both sources and the conflicting values:

```text
Cannot add cpu_sim.ftr.
Time domain differs from cpu_sim.vcd:
  cpu_sim.vcd: 1 ns, 0..125000
  cpu_sim.ftr: 1 ps, 0..125000000
Surfer does not rescale or offset sources in mixed sessions.
```

If a batch contains multiple files, accept all matching files and reject only
the mismatches. If the first file in an empty batch is rejected for parse errors,
the session stays empty.

## Architecture

### Core Types

Keep `DataContainer` as the per-source parsed database abstraction. It already
unifies wave and transaction metadata through methods such as `metadata` at
`libsurfer/src/data_container.rs:189`, `max_timestamp` at
`libsurfer/src/data_container.rs:104`, `root_scopes` at
`libsurfer/src/data_container.rs:113`, and `variables_in_scope` at
`libsurfer/src/data_container.rs:168`.

Introduce source identity and a source store:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceId(pub u64);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadedSource {
    pub id: SourceId,
    pub label: String,
    pub accent_color: Option<String>,
    pub source: WaveSource,
    pub format: WaveFormat,
    #[serde(skip, default = "DataContainer::__new_empty")]
    pub inner: DataContainer,
    pub time_domain: Option<TimeDomain>,
    pub selected_server_file_index: Option<usize>,
    pub cache_generation: u64,
    pub load_state: SourceLoadState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeDomain {
    pub timescale: TimeScale,
    pub max_timestamp: BigInt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceStore {
    pub sources: Vec<LoadedSource>,
    pub next_source_id: u64,
    pub session_time_domain: Option<TimeDomain>,
}
```

The `Vec` preserves display/load order without adding a new dependency. Lookup
helpers hide the linear search; source counts are expected to be small, and an
internal skipped index can be added later if profiling shows it matters.

`WaveData` should become the shared canvas/session view state. The least
disruptive migration is:

- Move `inner`, `source`, `format`, `cache_generation`, and source-specific
  in-flight caches from `WaveData` into `LoadedSource`.
- Add `sources: SourceStore` to `WaveData`.
- Keep `items_tree`, `displayed_items`, `viewports`, `cursor`, `markers`,
  annotations, focus, scroll, and drawing layout fields in `WaveData`.

This preserves current call sites that conceptually operate on the canvas while
forcing data lookups to go through `waves.sources`.

### Source-Qualified References

Every reference that names data inside a source must carry `SourceId`.

Keep source identity out of `surfer-translation-types`. `VariableRef` is a
plugin-facing reference type at `surfer-translation-types/src/variable_ref.rs:11`,
`ScopeRef` is defined at `surfer-translation-types/src/scope_ref.rs:5`, and both
intentionally treat backend IDs as performance hints rather than identity in
their hash/equality implementations at
`surfer-translation-types/src/variable_ref.rs:56` and
`surfer-translation-types/src/scope_ref.rs:30`. Translators should keep seeing
the same per-variable metadata and values; `SourceId` is a `libsurfer` resolution
concern for deciding which loaded container to query.

Add these wrappers:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceScopeRef<T> {
    pub source: SourceId,
    pub inner: T,
}

pub type SourceVariableRef = SourceScopeRef<VariableRef>;
pub type SourceWaveScopeRef = SourceScopeRef<ScopeRef>;
pub type SourceStreamScopeRef = SourceScopeRef<StreamScopeRef>;
pub type SourceTransactionStreamRef = SourceScopeRef<TransactionStreamRef>;
pub type SourceTransactionRef = SourceScopeRef<TransactionRef>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActiveScope {
    Wave(SourceWaveScopeRef),
    Stream(SourceStreamScopeRef),
}
```

The exact names can change during implementation, but the invariants cannot:

- `DisplayedVariable` stores `source: SourceId` next to `variable_ref`.
- `DisplayedStream` stores `source: SourceId` next to
  `transaction_stream_ref`.
- `FocusedTransaction` stores `SourceTransactionRef`, not `TransactionRef`.
- `TableModelSpec::TransactionTrace` and `TableModelSpec::EventTable` store
  `SourceTransactionStreamRef`.
- `TableModelSpec::SignalChangeList` and `MultiSignalEntry` store
  `SourceVariableRef`.
- `ScopeType` or its replacement stores source identity for active scope and
  hierarchy expansion.

Do not infer source identity by name, path, FTR IDs, or current active scope.
Those are display attributes, not stable references.

### Lookup API

Add a narrow lookup layer on `SourceStore` so call sites do not manually index
the map and match `DataContainer` repeatedly:

```rust
impl SourceStore {
    pub fn source(&self, id: SourceId) -> Option<&LoadedSource>;
    pub fn source_mut(&mut self, id: SourceId) -> Option<&mut LoadedSource>;
    pub fn waves(&self, id: SourceId) -> Option<&WaveContainer>;
    pub fn waves_mut(&mut self, id: SourceId) -> Option<&mut WaveContainer>;
    pub fn transactions(&self, id: SourceId) -> Option<&TransactionContainer>;
    pub fn transactions_mut(&mut self, id: SourceId) -> Option<&mut TransactionContainer>;
    pub fn common_time_domain(&self) -> Option<&TimeDomain>;
    pub fn validate_time_domain(&self, candidate: &TimeDomain) -> Result<()>;
}
```

Use this layer in:

- `WaveData::add_variables`, which currently unwraps one wave container at
  `libsurfer/src/wave_data.rs:543`.
- `WaveData::add_generator` and `WaveData::add_stream`, which currently read
  one transaction container at `libsurfer/src/wave_data.rs:713` and
  `libsurfer/src/wave_data.rs:754`.
- `TransactionTraceModel::new`, which currently takes the only transaction
  container from `ctx.waves` at `libsurfer/src/table/sources/transaction_trace.rs:92`.
- `table_model_context`, which currently exposes one `Option<&WaveData>` and one
  cache generation at `libsurfer/src/lib.rs:3096`.

### Additive Loading

Do not overload `LoadOptions` to mean "add another database". It currently means
clear or keep unavailable items while switching/reloading at
`libsurfer/src/wave_source.rs:180`.

Add an explicit intent:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LoadIntent {
    ReplaceSession,
    AddSource,
    ReloadSource {
        source: SourceId,
        keep_unavailable: bool,
    },
}
```

Add source-aware messages:

```rust
LoadFileWithIntent(Utf8PathBuf, LoadIntent)
LoadUrlWithIntent(String, LoadIntent)
LoadDataWithIntent(Vec<u8>, LoadIntent)
WaveHeaderLoaded { request: LoadRequestId, source_id: SourceId, ... }
WaveBodyLoaded { request: LoadRequestId, source_id: SourceId, ... }
SignalsLoaded { request: LoadRequestId, source_id: SourceId, ... }
TransactionStreamsLoaded { request: LoadRequestId, source_id: SourceId, ... }
CloseSource(SourceId)
ReloadSource(SourceId, bool)
```

Keep today's messages as compatibility shims:

- `Message::LoadFile(path, LoadOptions::Clear)` maps to `ReplaceSession`.
- `Message::LoadFile(path, LoadOptions::KeepAvailable | KeepAll)` keeps today's
  "switch/reload while preserving rows" behavior until command files are
  migrated.
- New UI paths use `LoadFileWithIntent(..., AddSource)`.

The loader should reserve a `SourceId` and create a pending `LoadedSource`
before starting async work. Async responses must carry both `SourceId` and a
`LoadRequestId` so stale responses from an earlier reload cannot overwrite a
newer source. This fixes a latent issue in the current code where
`WaveBodyLoaded` assumes the current `self.user.waves` is the one whose header
loaded at `libsurfer/src/lib.rs:1385`.

### Source Commit Protocol

Use a two-step commit for every source:

1. Parse enough metadata to construct `LoadedSource.inner` and `TimeDomain`.
2. Validate the candidate against `SourceStore.session_time_domain`.
3. If valid, commit the source and update UI state.
4. If invalid, remove the pending source and emit `Message::Error`.

For local FTR, parsing currently happens synchronously in
`load_transactions_from_file` at `libsurfer/src/wave_source.rs:460`. It can still
commit in one step once the candidate `TransactionContainer` is available.
Keeping that behavior is acceptable for the first fused-source implementation,
but a later polish phase should move FTR parsing behind `perform_work`, matching
the waveform header/body path that already dispatches blocking work from
`libsurfer/src/wave_source.rs:269`. Large FTR files should not freeze the UI
while another source is already visible; the existing
`OUTSTANDING_TRANSACTIONS` repaint mechanism at `libsurfer/src/lib.rs:156`
provides the model for keeping progress moving.

For local waveform files, the header is loaded first at
`libsurfer/src/lib.rs:1328`, but the max timestamp may not be known until the
body/time table is loaded at `libsurfer/src/lib.rs:1385`. Therefore:

- Pending waveform sources can appear in the sidebar after header load.
- The source is not eligible for mixed rendering until body/time-domain
  validation succeeds.
- If validation fails after body load, remove that source and any rows added
  while pending.

For remote Surver sources, the header path at `libsurfer/src/lib.rs:1363` should
also create a pending source. The time table response determines final
validation.

## Shared Timeline

### Strict Time Domain Rule

Phase 1 should force one exact time domain for every source in a mixed session.
Surfer should not rescale, offset, or merge partially overlapping time ranges.

Two sources are compatible only if:

- `TimeScale.unit` matches exactly.
- `TimeScale.multiplier` matches exactly.
- `max_timestamp` matches exactly after zero/unknown values are normalized.
- Both sources use the same implicit origin, timestamp zero.

This follows the user's requested policy: force the same time space and report
an error if it differs. It is intentionally stricter than commercial viewers
because Surfer has no reliable cross-format origin/offset metadata today.

`TimeUnit::None` is compatible only with `TimeUnit::None`. It must not be
silently treated as seconds, cycles, or FTR `Unit`.

### Session Time Domain

The first committed source establishes:

```rust
WaveData.sources.session_time_domain = Some(TimeDomain {
    timescale,
    max_timestamp,
});
```

All viewport math uses this session domain. Replace the existing
`WaveData::num_timestamps`, which currently reads one `inner.max_timestamp()` at
`libsurfer/src/wave_data.rs:1170`, with:

```rust
pub fn num_timestamps(&self) -> Option<BigInt> {
    self.sources
        .session_time_domain
        .as_ref()
        .map(|domain| domain.max_timestamp.clone())
        .filter(|t| !t.is_zero())
}
```

The existing `Viewport` type can stay unchanged. It is already a shared
relative viewport over a provided timestamp count at
`libsurfer/src/viewport.rs:103`, converts pixels to time at
`libsurfer/src/viewport.rs:156`, converts time to pixels at
`libsurfer/src/viewport.rs:185`, and clips when a file length changes at
`libsurfer/src/viewport.rs:212`.

Cursor and markers remain single session values. They are already shared
`BigInt` timestamps at `libsurfer/src/wave_data.rs:71` and
`libsurfer/src/wave_data.rs:72`. No per-source cursor is needed.

Timeline formatting should use the session time scale instead of an arbitrary
container. Replace call sites like `TimeFormatter::new(&waves.inner.metadata().timescale, ...)`
at `libsurfer/src/drawing_canvas.rs:1255` and
`libsurfer/src/waveform_tile.rs:251` with the session domain.

### Reload With Same Domain

Reloading one source must not mutate the session time domain while other sources
are present. If a reloaded source changes time scale or max timestamp, keep the
old source loaded and report the mismatch. If it is the only source, use today's
viewport clipping behavior through `WaveData::update_viewports` at
`libsurfer/src/wave_data.rs:400`.

## Rendering Architecture

The current draw cache enum assumes the entire canvas is either waveforms or
transactions at `libsurfer/src/drawing_canvas.rs:541`. Mixed rendering needs a
canvas cache that can contain both command families at once.

Replace:

```rust
enum CachedDrawData {
    WaveDrawData(CachedWaveDrawData),
    TransactionDrawData(CachedTransactionDrawData),
}
```

with:

```rust
pub struct CachedCanvasDrawData {
    pub wave: CachedWaveDrawData,
    pub transactions: HashMap<SourceId, CachedTransactionDrawData>,
}
```

or an equivalent row-keyed cache:

```rust
pub enum CachedRowDrawData {
    Variable(VariableDrawCommands),
    Stream(TxDrawingCommands),
}
```

The important behavior is per-row dispatch:

1. Iterate visible rows once from `items_tree`.
2. For `DisplayedItem::Variable`, look up that row's `source` as a wave source
   and generate wave commands using that source's `WaveContainer`.
3. For `DisplayedItem::Stream`, look up that row's `source` as an FTR source and
   generate transaction commands using that source's `TransactionContainer`.
4. Draw ticks, cursor, markers, annotations, and graphics once per viewport
   using the session time domain.

Transaction draw maps must be keyed by `SourceTransactionStreamRef`, not
`TransactionStreamRef`. The current transaction cache maps use
`TransactionStreamRef` and `TransactionRef` at
`libsurfer/src/drawing_canvas.rs:663`; both need source qualification to avoid
cross-file collisions.

Analog cache keys also need source identity. `WaveData::build_analog_cache_async`
currently finds the wave container through one `inner.as_waves()` at
`libsurfer/src/wave_data.rs:1210`. Its cache key should include `SourceId`, and
analog inflight registries should move to `LoadedSource` so reloading one
waveform source does not invalidate unrelated waveform sources.

## Hierarchy Architecture

The existing hierarchy has useful per-container branches. Preserve those, but
change the outer loop.

Target flow:

```rust
for source in waves.sources.visible_sources() {
    draw_source_header(source);
    match &source.inner {
        DataContainer::Waves(_) => draw_wave_source_scopes(source.id, ...),
        DataContainer::Transactions(_) => draw_transaction_source_scopes(source.id, ...),
        DataContainer::Empty => draw_pending_or_error(source),
    }
}
```

Source headers should be stable egui IDs derived from `SourceId`, not path
strings. The current FTR root uses `egui::Id::from("Streams")` at
`libsurfer/src/transactions.rs:665`; this must include `SourceId` so multiple
FTR files do not share collapse state.

`Message::SetActiveScope` currently carries `Option<ScopeType>` at
`libsurfer/src/message.rs:85`. It should carry `Option<ActiveScope>` or
`Option<SourceScopeRef<ScopeType>>`.

Wave scope context actions currently emit source-less messages like
`Message::AddScope(scope.clone(), false)` at `libsurfer/src/hierarchy.rs:536`.
FTR generator clicks emit source-less `Message::AddStreamOrGenerator` at
`libsurfer/src/transactions.rs:780`. All such messages must become
source-qualified.

## Table Architecture

Tables are already specified independently from the canvas through
`TableModelSpec`, but transaction and signal specs are source-less today.
`TransactionTrace` stores only a `TransactionStreamRef` at
`libsurfer/src/table/model.rs:38`, and `TableModelContext` exposes only
`Option<&WaveData>` at `libsurfer/src/table/model.rs:245`.

Change table specs to source-qualified refs:

```rust
SignalChangeList { variable: SourceVariableRef, field: Vec<String> }
MultiSignalChangeList { variables: Vec<SourceMultiSignalEntry> }
TransactionTrace { generator: SourceTransactionStreamRef }
EventTable { generator: SourceTransactionStreamRef }
```

Table creation then looks up the exact source. `ensure_transaction_stream_loaded`
currently loads from the only transaction container at
`libsurfer/src/table_controller.rs:348`; it should take
`SourceTransactionStreamRef` and load only that source.

Table titles should include the source label when more than one source is open:
`Transactions: cpu_sim.ftr / pipelined_stream.write`.

Table cache keys must include source generation. The current cache context has
one `cache_generation` at `libsurfer/src/table/model.rs:251`. Replace it with a
source-aware generation, for example:

```rust
pub struct TableModelContext<'a> {
    pub waves: &'a WaveData,
    pub source_generations: HashMap<SourceId, u64>,
    ...
}
```

Only tables whose source changes should rebuild after a per-source reload.

## State Files

State files serialize `UserState` with RON through `encode_state` at
`libsurfer/src/state_file_io.rs:296` and load it back through
`load_state_from_bytes` at `libsurfer/src/state_file_io.rs:306`. The schema must
round-trip multiple sources and migrate old single-source files.

New serialized shape:

```rust
pub struct WaveData {
    pub sources: SourceStoreState,
    pub active_scope: Option<ActiveScope>,
    pub items_tree: DisplayedItemTree,
    pub displayed_items: HashMap<DisplayedItemRef, DisplayedItem>,
    pub viewports: Vec<Viewport>,
    pub cursor: Option<BigInt>,
    pub markers: HashMap<u8, BigInt>,
    ...
}

pub struct SourceStoreState {
    pub sources: Vec<SourceState>,
    pub next_source_id: u64,
    pub session_time_domain: Option<TimeDomain>,
}

pub struct SourceState {
    pub id: SourceId,
    pub label: String,
    pub source: WaveSource,
    pub format: WaveFormat,
    pub time_domain: Option<TimeDomain>,
}
```

Parsed containers remain `serde(skip)` just like the current `WaveData.inner` at
`libsurfer/src/wave_data.rs:60`; state loading must reload databases from
`SourceState.source`.

Migration rules:

- Old state with `waves: Some(WaveData { inner, source, format, ... })` becomes
  one source with `SourceId(0)`.
- Old displayed variables/streams receive `SourceId(0)`.
- Old table specs receive `SourceId(0)`.
- Old transaction focus receives `SourceId(0)`.
- If no source is currently loaded and a state file names sources, enqueue
  source loads for all serialized source locators, validate the shared time
  domain, then restore canvas rows.
- If sources are already loaded, match by serialized `SourceId` when the state
  was loaded from the same state file; otherwise prefer exact `WaveSource`
  equality and fall back to prompting or placeholders.

Missing source behavior should be explicit. If a state references a missing file,
keep source-owned rows as placeholders with the source label and show the source
header as errored. The current placeholder model for unavailable waveform items
at `libsurfer/src/displayed_item.rs:300` is a useful pattern, but it also needs
source identity.

## Edge Cases

### Closing A Source

`CloseSource(SourceId)` should:

- Remove the source from `SourceStore`.
- Remove all displayed rows owned by that source.
- Remove or close table tiles whose specs reference that source.
- Clear active scope if it points at that source.
- Clear focused transaction if it points at that source.
- Remove annotations attached to removed rows, matching the existing item removal
  behavior at `libsurfer/src/wave_data.rs:638`.
- Keep session-level cursor, markers, annotations not attached to removed rows,
  viewports, groups, dividers, and timelines.

If the source has displayed rows or tables, confirm with counts:
`Close cpu_sim.ftr? This removes 3 transaction rows and 1 table.` The command
file/message path can bypass UI confirmation by dispatching the explicit close
message.

If the last source is closed, reset `session_time_domain` and leave a clean
empty canvas.

### Per-Source Reload

`ReloadSource(SourceId, keep_unavailable)` should reload only that source's
locator. The current global reload reads `waves.source` and reloads the whole
single source at `libsurfer/src/lib.rs:1570`; that should become a wrapper that
reloads the active or first source for backward compatibility.

Reload behavior:

- If reload succeeds and the time domain matches, replace only that
  `LoadedSource.inner`, increment only that source's `cache_generation`, and
  reconcile rows owned by that source.
- If reload succeeds but the time domain differs while other sources exist, keep
  the old source and report an error.
- If reload fails, keep the old source and mark its header with the error.
- If the source is `WaveSource::Data`, anonymous drag-and-drop bytes, or CXXRTL
  without a reconnectable locator, disable reload.

File watchers should be per source. The desktop entry point currently installs
one watcher for the startup file at `surfer/src/main.rs:185`; a multi-source
session should store watcher handles by `SourceId` and emit
`SuggestReloadSource(SourceId)`.

### Remote And Local Mix

Remote waveform plus local FTR is supported when their time domains match.
Source identity must include the remote server and selected file index because
`selected_server_file_index` is currently global in `UserState` at
`libsurfer/src/state.rs:141`.

Rules:

- A Surver waveform source stores `WaveSource::Url(server)` and
  `selected_server_file_index` in `LoadedSource`.
- Remote signal loading messages carry `SourceId`; `SignalsLoaded` currently has
  no source identity at `libsurfer/src/message.rs:199`.
- A URL that downloads raw `.ftr` bytes can be added as an FTR source through the
  existing byte sniffing path at `libsurfer/src/wave_source.rs:247`.
- Surver-hosted FTR should be considered unsupported until Surver exposes FTR
  hierarchy and transaction payload endpoints.

### Duplicate Names And IDs

Duplicate basenames, scopes, variables, stream names, generator IDs, and
transaction IDs are allowed across sources. Source identity is the only
disambiguator.

This specifically fixes FTR transaction collisions: `TransactionRef` currently
wraps only `TransactionId` at `libsurfer/src/transaction_container.rs:309`, so
two FTR files can both contain `TransactionId(4)`. Mixed sessions must use
`SourceTransactionRef { source, inner: TransactionRef { id } }` everywhere
focus, tooltips, tables, and draw caches refer to transactions.

### Undo/Redo

Undo/redo currently stores canvas state through `current_canvas_state` at
`libsurfer/src/state.rs:730`. It should continue to store canvas-level rows,
markers, annotations, and focus, but not clone parsed source containers. Source
add/close/reload should be explicit undoable operations only after a dedicated
source undo design exists. Phase 1 can make source add/close non-undoable and
clear redo on source topology changes.

### Commands And WCP

Command files and WCP commands can remain single-active-source initially, but
new commands must be source-aware:

- `add_signal cpu_sim.vcd tb.cpu.clk`
- `add_transaction cpu_sim.ftr pipelined_stream.write`
- `set_active_scope cpu_sim.ftr tr.pipelined_stream`

Existing source-less commands resolve against the active source. If more than
one compatible source could match, return an ambiguity error rather than picking
one silently.

## Phased Implementation Plan

Each phase should be independently testable. Use dev/test builds only.

### Phase 0: Single-Source Refactor Oracle

First introduce `SourceId` plumbing around the existing single `WaveData.inner`:
source-qualified display items, source-qualified active scope, and
source-qualified add/navigation messages all default to `SourceId(0)`. Do not
move containers into `SourceStore` yet. No additive loading, source headers, or
mixed rendering behavior should be visible in this phase.

This phase's main test is the existing snapshot suite: single-source waveform
and FTR rendering should remain unchanged. The existing FTR hierarchy snapshot
at `libsurfer/src/tests/snapshot.rs:2001` and FTR canvas snapshot at
`libsurfer/src/tests/snapshot.rs:2683` are especially important because they
exercise the transaction-only paths that will later share the canvas with
waveforms.

### Phase 1: Test Fixtures And Harness Hooks

Add or identify a tiny matching waveform/FTR pair under `examples/`. The pair
must share exact `TimeDomain` and contain:

- At least one waveform signal with visible transitions.
- At least one FTR stream with one generator.
- Matching max timestamp.

Extend the snapshot harness. `snapshot_ui_with_file_and_msgs` currently loads
one file at `libsurfer/src/tests/snapshot.rs:229`; add
`snapshot_ui_with_files_and_msgs` that loads the first file with
`ReplaceSession`, adds subsequent files with `AddSource`, then waits through
`wait_for_waves_fully_loaded` at `libsurfer/src/tests/snapshot.rs:2236`.

### Phase 2: Source Store And Time Domain

Build on the Phase 0 `SourceId` plumbing by adding `LoadedSource`,
`SourceStore`, and `TimeDomain` with unit tests for:

- First source establishes the session domain.
- Matching source is accepted.
- Different time unit is rejected.
- Different multiplier is rejected.
- Different max timestamp is rejected.
- `TimeUnit::None` matches only `TimeUnit::None`.

No UX change is required in this phase. Existing one-source tests must continue
to pass.

### Phase 3: Move Containers Into SourceStore

Refactor `WaveData` so parsed containers live under `waves.sources`, while
canvas state remains in `WaveData`.

Mechanical targets:

- Replace `waves.inner` lookups with `waves.sources.*` lookup helpers.
- Preserve one-source behavior by assigning `SourceId(0)` to the first source.
- Update `waves_fully_loaded`, which currently checks one container at
  `libsurfer/src/state.rs:714`, to require all committed/pending sources to be
  fully loaded.
- Update draw invalidation to size caches by viewport, not source count, while
  cache contents become source-aware.

Run the existing snapshot and table tests after this phase. The FTR snapshots at
`libsurfer/src/tests/snapshot.rs:2001` and
`libsurfer/src/tests/snapshot.rs:2683` should still render unchanged for
single-source FTR.

### Phase 4: Additive Load Messages

Implement `LoadIntent`, source-aware async messages, and pending source commit.

Tests:

- Load waveform, then add matching FTR; assert two sources exist and the
  waveform source remains current.
- Load FTR, then add matching waveform; assert the same result in reverse order.
- Add mismatched FTR; assert one source remains and an error is emitted.
- Simulate stale reload response by sending an old `LoadRequestId`; assert it is
  ignored.

### Phase 5: Source-Sectioned Hierarchy

Render all sources in tree and separate hierarchy modes.

Tests:

- Snapshot with VCD+FTR in tree mode shows both source headers.
- Snapshot with VCD+FTR in separate mode shows both source headers and switching
  active scopes changes the variable/generator pane.
- Snapshot with duplicate source basenames shows disambiguated labels.

This phase should update wave scope messages from `Message::AddScope` at
`libsurfer/src/hierarchy.rs:536` and FTR add messages from
`libsurfer/src/transactions.rs:780` to source-aware variants.

### Phase 6: Mixed Canvas Rendering

Change draw command generation from whole-container dispatch to row dispatch.

Tests:

- Snapshot with a signal row and an FTR generator row on the same canvas.
- Snapshot with cursor, marker, and default timeline visible across both row
  types.
- Snapshot with a divider between waveform and transaction rows.
- Message test moving focus and adding rows at focus across mixed item types.

The key regression guard is that `generate_draw_commands` no longer selects one
path from `waves.inner` as it does at `libsurfer/src/drawing_canvas.rs:541`.

### Phase 7: Tables, Tooltips, And Transaction Focus

Make table specs, transaction focus, tooltips, and event overlays source-aware.

Tests:

- Open a transaction table from the FTR source in a mixed session.
- Save and reload a state file with the transaction table open.
- Focus a transaction in the mixed canvas and ensure the correct FTR source is
  used even if another FTR source has the same transaction ID.
- Signal change-list table still opens from waveform rows.

### Phase 8: Close, Reload, Remote Mix, And State Round-Trip

Implement source close, per-source reload, per-source file watchers, and full
state-file round-trip.

Tests:

- Closing the FTR source removes transaction rows and FTR tables but leaves
  waveform rows, cursor, and markers.
- Reloading only the FTR source preserves waveform rows and source IDs.
- Reload mismatch keeps the old source and emits an error.
- Local waveform plus remote waveform/FTR byte URL validates through the same
  time-domain path.
- State file with VCD+FTR reloads both sources and restores mixed rows.
- Old single-source state file migrates to `SourceId(0)`.

The existing save/load snapshots around state files, for example the state
restore flow near `libsurfer/src/tests/snapshot.rs:2349`, should be extended
rather than replaced.

### Phase 9: Optional UX And Concurrency Polish

These are good follow-ups, but not prerequisites for the core fused-source
architecture:

- Add sibling auto-detection using an `Ask`/`Always`/`Never` policy and tests
  covering accept, decline, and disabled modes.
- Use `LoadedSource.accent_color` for sidebar headers and compact row badges or
  left gutters. Snapshot-test duplicate source names and mixed rows so the
  source cue is visible but not noisy.
- Move synchronous FTR parsing to `perform_work` and surface per-source progress
  in the sidebar. Existing FTR snapshot tests should remain unchanged, and a
  message-level test should verify that an already visible source remains usable
  while the second source is loading.

## Non-Goals For The First Implementation

- No automatic time rescaling between units.
- No timestamp offset alignment.
- No partial overlap view where sources have different max timestamps.
- No hidden sidecar FTR embedded inside a waveform source.
- No guessing source identity from filenames.
- No new unsafe Rust.

These constraints are deliberate. They keep the first implementation correct,
testable, and aligned with Surfer's existing waveform and transaction
abstractions while still delivering the desired mixed-database UX.
